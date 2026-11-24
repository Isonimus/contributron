//! Finding 1 of the 2026-11-24 adversarial review: a store failure on one target's task
//! used to return `Err` and the error was held until every sibling task finished (which
//! only happened on SIGINT), while the operator saw nothing on stderr — the silent partial
//! outage class. The fix routes store errors through the event channel as a
//! [`TickOutcome::StoreFailed`], the same way fetch errors already were.
//!
//! This test pre-creates a store file with a mismatching `PRAGMA user_version`, which the
//! store refuses to open (`StoreError::SchemaVersionMismatch`). The polling task must
//! report that failure on the event channel before exiting, and [`run_loop`] must still
//! return `Ok` because other targets are not punished for one target's broken store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{constant_fetch, locator, observation, user};
use contributron_poll::{PollConfig, Tick, TickOutcome, run_loop};
use rusqlite::Connection;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_store_open_failure_is_reported_on_the_event_channel_not_silently_returned() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");

    // Pre-create the file with a user_version this build does not recognize. The schema
    // check in `EventStore::open` turns this into `StoreError::SchemaVersionMismatch`.
    let conn = Connection::open(&store_path).expect("open for pre-population");
    conn.execute_batch("PRAGMA user_version = 99;")
        .expect("pragma");
    drop(conn);

    let owner = user("local-operator");
    let target = locator("alice", "project", 7);
    let (fetch, _calls) = constant_fetch(observation(7, "any"));
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
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    // The failing open happens at task startup, before any sleep. Yield so the task runs.
    let tick = events_rx.recv().await.expect("one StoreFailed tick");
    match tick.outcome {
        TickOutcome::StoreFailed { error, backoff } => {
            assert!(
                error.contains("schema version mismatch"),
                "error should name the schema mismatch, got: {error:?}",
            );
            assert!(
                backoff.is_none(),
                "open failure is fatal for the target: no retry backoff expected",
            );
        }
        other => panic!("expected StoreFailed on open, got: {other:?}"),
    }

    // The task exits on its own after reporting. No cancel was sent; `run_loop` returns
    // once the last task finishes, which is now. The loop returns Ok because this target's
    // failure is an operational event, not a loop-killing one.
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok despite the per-target failure");

    assert!(
        events_rx.try_recv().is_err(),
        "no further tick after the StoreFailed report"
    );
}
