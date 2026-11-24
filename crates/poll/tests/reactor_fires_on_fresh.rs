//! Slice 0014: the polling task threads the prior observation forward in memory and hands
//! each (prior, new) pair to the attached reactor after a fresh store write. An unchanged
//! tick skips the reactor (there is no new state for it to compare against), and a tick
//! with no reactor attached reports `None`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_poll::{
    PollConfig, ReactionReport, Reactor, Tick, TickOutcome, run_loop_with_reactor,
};
use contributron_domain::pull_request::ObservedPullRequest;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

type Pair = (Option<ObservedPullRequest>, ObservedPullRequest);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_reactor_receives_the_prior_from_the_previous_fresh_tick() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    // Three distinct observations so every store write is Fresh (content digest changes).
    let obs_a = observation(7, "title a");
    let obs_b = observation(7, "title b");
    let obs_c = observation(7, "title c");
    let script = vec![
        Ok(obs_a.clone()),
        Ok(obs_b.clone()),
        Ok(obs_c.clone()),
    ];
    let fetch = scripted_fetch(script);

    // The reactor records every (prior, new) pair it saw. Its decision is deterministic:
    // `NotApplicable` on the first call (prior is None), `Posted` thereafter.
    let seen: Arc<Mutex<Vec<Pair>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_reactor = Arc::clone(&seen);
    let reactor: Reactor = Arc::new(move |_locator, prior, new| {
        let seen = Arc::clone(&seen_for_reactor);
        Box::pin(async move {
            let decision = if prior.is_some() {
                ReactionReport::Posted {
                    agent_id: "rule:threads-cleared".into(),
                }
            } else {
                ReactionReport::NotApplicable
            };
            seen.lock().expect("mutex").push((prior, new));
            decision
        })
    });

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop_with_reactor(
            config,
            fetch,
            Some(reactor),
            &store_path_for_loop,
            events_tx,
            cancel_rx,
        )
        .await
    });

    let mut ticks: Vec<Tick> = Vec::new();
    for _ in 0..3 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        ticks.push(events_rx.recv().await.expect("a tick"));
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    // Every tick was a fresh write.
    for (i, t) in ticks.iter().enumerate() {
        assert!(
            matches!(t.outcome, TickOutcome::Fresh),
            "tick {i} should be Fresh, got {:?}",
            t.outcome,
        );
    }

    // First reactor call: prior = None. Later calls: prior = the previous observation.
    assert!(matches!(
        ticks[0].reaction,
        Some(ReactionReport::NotApplicable),
    ));
    assert!(matches!(
        ticks[1].reaction,
        Some(ReactionReport::Posted { .. }),
    ));
    assert!(matches!(
        ticks[2].reaction,
        Some(ReactionReport::Posted { .. }),
    ));

    let pairs = seen.lock().expect("mutex").clone();
    assert_eq!(pairs.len(), 3);
    assert!(pairs[0].0.is_none(), "first prior is None");
    assert_eq!(pairs[0].1.title, "title a");
    assert_eq!(
        pairs[1].0.as_ref().map(|o| o.title.as_str()),
        Some("title a"),
    );
    assert_eq!(pairs[1].1.title, "title b");
    assert_eq!(
        pairs[2].0.as_ref().map(|o| o.title.as_str()),
        Some("title b"),
    );
    assert_eq!(pairs[2].1.title, "title c");
}
