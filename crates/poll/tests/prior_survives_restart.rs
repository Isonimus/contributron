//! Finding 2 of the 2026-11-24 adversarial review of slice 0014. The in-memory prior is
//! seeded from the store at task startup, so a transition whose two halves land in different
//! process lifetimes still fires the reactor.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_domain::actor::{Attribution, ExecutingAgent, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_poll::{
    POLL_WORKER, PollConfig, ReactionReport, Reactor, Tick, TickOutcome, run_loop_with_reactor,
};
use contributron_store::EventStore;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

type Pair = (Option<String>, String);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_first_fresh_tick_sees_the_store_prior() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    // Seed the store with a "before" observation. The content digest of this row is
    // different from the next observation's, so the loop will write the next one Fresh.
    let before = observation(7, "before");
    {
        let store = EventStore::open(&store_path, owner.clone()).expect("open store");
        let attribution = Attribution {
            user: owner.clone(),
            worker: WorkerId::new(POLL_WORKER),
            agent: ExecutingAgent::NoAgent,
        };
        let observation = PullRequestObservation::new(attribution, before.clone());
        store.write(&observation).expect("seed write");
    }

    // The loop fetches "after". First Fresh tick should hand the reactor (Some(before),
    // after) rather than (None, after).
    let after = observation(7, "after");
    let fetch = scripted_fetch(vec![Ok(after.clone())]);

    let seen: Arc<Mutex<Vec<Pair>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_reactor = Arc::clone(&seen);
    let reactor: Reactor = Arc::new(move |_locator, prior, new| {
        let seen = Arc::clone(&seen_for_reactor);
        Box::pin(async move {
            seen.lock()
                .expect("seen")
                .push((prior.as_ref().map(|p| p.title.clone()), new.title.clone()));
            ReactionReport::NotApplicable
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

    tokio::time::advance(interval.saturating_mul(2)).await;
    let tick = events_rx.recv().await.expect("a tick");

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    assert!(
        matches!(tick.outcome, TickOutcome::Fresh),
        "the after observation should be Fresh, got {:?}",
        tick.outcome,
    );

    let pairs = seen.lock().expect("seen").clone();
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0].0.as_deref(),
        Some("before"),
        "prior should have been seeded from the store, got {:?}",
        pairs[0].0,
    );
    assert_eq!(pairs[0].1, "after");
}
