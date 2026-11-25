//! Slice 0011, scenario "shutdown": cancellation flipped *while a fetch is in flight*
//! drains that fetch and then exits within one virtual-interval, which is the property the
//! Definition of Done names ("a poll in flight runs to completion").
//!
//! Finding 3 of the 2026-11-24 adversarial review flagged the earlier version: it used a
//! `constant_fetch` that resolved instantly, so cancel was never sent while a fetch was
//! pending, and the "drains in-flight" claim was not actually exercised. This test uses a
//! fetch that blocks on a oneshot channel the test controls, flips cancel while the fetch
//! is blocked, releases the fetch, and asserts a `Fresh` tick for the drained poll plus a
//! clean loop return.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, user};
use contributron_poll::{Fetch, FetchFuture, PollConfig, Tick, TickOutcome, run_loop};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cancellation_mid_fetch_drains_the_in_flight_poll_and_exits() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 3);

    // A oneshot that the test holds the sending end of. The first fetch call takes the
    // receiver and awaits it; later calls (if any) return without blocking.
    let (hang_tx, hang_rx) = oneshot::channel::<()>();
    let slot: Arc<Mutex<Option<oneshot::Receiver<()>>>> = Arc::new(Mutex::new(Some(hang_rx)));
    let obs = observation(3, "drained in flight");
    let fetch: Fetch = {
        let obs = obs.clone();
        Arc::new(move |_locator| {
            let obs = obs.clone();
            let slot = Arc::clone(&slot);
            Box::pin(async move {
                let maybe_rx = slot.lock().expect("fetch slot not poisoned").take();
                if let Some(rx) = maybe_rx {
                    // Awaiting the oneshot parks this task until the test flips it, which
                    // is exactly the "fetch in flight" window the scenario needs.
                    match rx.await {
                        Ok(()) | Err(_) => {}
                    }
                }
                Ok(obs)
            }) as FetchFuture
        })
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: contributron_domain::actor::WorkerId::new("cli").expect("static test id"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    // Advance past the first-poll delay (zero after the review fix, but the sleep of 0 is
    // still an `.await` point). Yield to let the task reach the fetch and park on the
    // oneshot.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    // At this point the fetch is pending on the oneshot. Flip cancel: the loop cannot see
    // it yet because the loop is inside the fetch await, not in the select.
    cancel_tx.send(true).expect("cancel receiver still alive");

    // Nothing has ticked yet: the fetch has not resolved.
    assert!(
        events_rx.try_recv().is_err(),
        "no tick reported while the fetch is still in flight",
    );

    // Release the fetch. The loop should now write the row, send one `Fresh` tick, then
    // see the cancel on its next select iteration and exit.
    hang_tx.send(()).expect("fetch is awaiting the oneshot");

    let tick = events_rx.recv().await.expect("the drained tick");
    assert!(
        matches!(tick.outcome, TickOutcome::Fresh),
        "the in-flight poll must drain to a Fresh row, got: {:?}",
        tick.outcome,
    );

    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok");

    // Nothing more fires after cancel is honoured.
    assert!(
        events_rx.try_recv().is_err(),
        "no tick after the drained poll and the cancel"
    );
}
