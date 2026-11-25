//! Slice 0011, scenario "transient error": the first call fails, a subsequent call
//! succeeds, the loop reports the failure to the outcome channel and keeps running, and the
//! store eventually holds a row from the successful call.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_domain::actor::UserId;
use contributron_poll::{PollConfig, Tick, TickOutcome, run_loop};
use contributron_store::EventStore;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_transient_error_backs_off_and_the_loop_recovers_on_the_next_call() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 11);

    let obs = observation(11, "recovery");
    // Fail once, then succeed forever (scripted_fetch repeats the final entry).
    let fetch = scripted_fetch(vec![Err("transient"), Ok(obs.clone())]);

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: contributron_domain::actor::WorkerId::new("cli"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    // First tick: advance past the jitter + interval; expect a failure with a backoff of
    // one interval (consecutive_failures = 1 → shift = 0 → multiplier = 1).
    tokio::time::advance(interval.saturating_mul(2)).await;
    let first = events_rx.recv().await.expect("the first tick");
    match &first.outcome {
        TickOutcome::FetchFailed { backoff, error } => {
            assert_eq!(*backoff, interval, "backoff after one failure is 1x");
            assert_eq!(
                error.0, "transient",
                "the stub's error message is preserved"
            );
        }
        other => panic!("expected FetchFailed, got: {other:?}"),
    }

    // Second tick: advance past the backoff; expect the success to land as a fresh row.
    tokio::time::advance(interval.saturating_mul(2)).await;
    let second = events_rx.recv().await.expect("the second tick");
    assert!(
        matches!(second.outcome, TickOutcome::Fresh),
        "second tick after recovery must be Fresh, got: {:?}",
        second.outcome
    );

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok");

    let store =
        EventStore::open(&store_path, UserId::new("local-operator")).expect("store reopens");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1, "exactly one row (the successful call)");
    assert_eq!(rows[0].pull_request.title, "recovery");
}
