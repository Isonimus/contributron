//! Finding 1 of the 2026-11-24 adversarial review of slice 0014. A reactor that returns
//! `Failed` on a fresh tick parks the new observation, and the next Unchanged tick retries
//! the reactor against the same unchanged prior. A transient provider error no longer drops
//! the transition.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_poll::{
    PollConfig, ReactionReport, Reactor, Tick, TickOutcome, run_loop_with_reactor,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

type Pair = (Option<String>, String);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_reaction_is_retried_on_the_next_unchanged_tick() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    // The fetch returns the same observation on every call. First tick writes it Fresh;
    // every later tick returns Duplicate, which the loop reports as Unchanged.
    let obs = observation(7, "cleared");
    let fetch = scripted_fetch(vec![Ok(obs.clone())]);

    // The reactor fails its first invocation and succeeds on every subsequent one. Records
    // every (prior, new) pair it saw so the test can prove the retry used the parked `obs`
    // rather than a fresh fetch's result, and that `prior` did not advance past the failure.
    let calls_before_success = Arc::new(Mutex::new(0u32));
    let seen: Arc<Mutex<Vec<Pair>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_for_reactor = Arc::clone(&calls_before_success);
    let seen_for_reactor = Arc::clone(&seen);
    let reactor: Reactor = Arc::new(move |_locator, prior, new| {
        let calls = Arc::clone(&calls_for_reactor);
        let seen = Arc::clone(&seen_for_reactor);
        Box::pin(async move {
            seen.lock()
                .expect("seen")
                .push((prior.as_ref().map(|p| p.title.clone()), new.title.clone()));
            let mut c = calls.lock().expect("calls");
            *c = c.saturating_add(1);
            if *c == 1 {
                ReactionReport::Failed {
                    error: "transient".into(),
                }
            } else {
                ReactionReport::Posted {
                    agent_ids: vec!["rule:threads-cleared".into()],
                    errors: Vec::new(),
                }
            }
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
    for _ in 0..2 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        ticks.push(events_rx.recv().await.expect("a tick"));
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    assert!(
        matches!(ticks[0].outcome, TickOutcome::Fresh),
        "first tick should be Fresh, got {:?}",
        ticks[0].outcome,
    );
    assert!(
        matches!(ticks[0].reaction, Some(ReactionReport::Failed { .. }),),
        "first reaction should be Failed, got {:?}",
        ticks[0].reaction,
    );

    assert!(
        matches!(ticks[1].outcome, TickOutcome::Unchanged),
        "second tick should be Unchanged, got {:?}",
        ticks[1].outcome,
    );
    assert!(
        matches!(ticks[1].reaction, Some(ReactionReport::Posted { .. }),),
        "second reaction should retry and Post, got {:?}",
        ticks[1].reaction,
    );

    let pairs = seen.lock().expect("seen").clone();
    assert_eq!(pairs.len(), 2, "reactor should have been invoked twice");
    assert!(
        pairs[0].0.is_none(),
        "first prior should be None (fresh task)",
    );
    assert_eq!(pairs[0].1, "cleared");
    assert!(
        pairs[1].0.is_none(),
        "retry prior should still be None because the failed call did not advance it",
    );
    assert_eq!(
        pairs[1].1, "cleared",
        "retry should carry the parked observation, not a stale one",
    );
}
