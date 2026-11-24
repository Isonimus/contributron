//! Slice 0011, scenario "unchanged state": ten ticks under a stub that returns the same
//! observation produce one fresh row and nine `unchanged` reports, in that order.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{constant_fetch, locator, observation, user};
use contributron_poll::{PollConfig, Tick, TickOutcome, run_loop};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ten_ticks_of_unchanged_state_yield_one_fresh_and_nine_unchanged() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 42);
    let obs = observation(42, "a steady title");

    let (fetch, calls) = constant_fetch(obs);
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

    let mut outcomes: Vec<TickOutcome> = Vec::new();
    // Pump ten ticks by advancing virtual time; `recv` between advances so the task gets a
    // chance to run.
    for _ in 0..10 {
        // Advance by one interval plus the deterministic jitter, which is less than one
        // interval, so two intervals is always enough to fire exactly one tick.
        tokio::time::advance(interval.saturating_mul(2)).await;
        let tick = events_rx
            .recv()
            .await
            .expect("a tick before the channel closes");
        assert_eq!(tick.locator, target);
        outcomes.push(tick.outcome);
    }

    // Shut the loop down so the test ends.
    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop task joined without panic")
        .expect("loop returned Ok");

    assert_eq!(*calls.lock().unwrap(), 10, "fetch was called ten times");
    assert!(
        matches!(outcomes[0], TickOutcome::Fresh),
        "first tick was fresh, got: {:?}",
        outcomes[0]
    );
    for (i, outcome) in outcomes.iter().enumerate().skip(1) {
        assert!(
            matches!(outcome, TickOutcome::Unchanged),
            "tick {i} must be Unchanged, got: {outcome:?}",
        );
    }
}
