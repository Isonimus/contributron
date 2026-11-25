//! Slice 0011, scenario "fairness": two pull requests on the same interval are each polled
//! the same number of times under paused virtual time, so a slow or failing fetch on one
//! target does not starve the other.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{constant_fetch, locator, observation, user};
use contributron_poll::{PollConfig, Tick, TickOutcome, run_loop};
use std::collections::HashMap;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn two_targets_on_the_same_interval_are_each_polled_the_same_number_of_times() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target_a = locator("alice", "project", 1);
    let target_b = locator("bob", "elsewhere", 2);

    let (fetch, _calls) = constant_fetch(observation(99, "any"));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target_a.clone(), target_b.clone()],
        interval,
        owner: owner.clone(),
        worker: contributron_domain::actor::WorkerId::new("cli").expect("static test id"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    // Run for ten intervals of virtual time. Each target's schedule after its first poll is
    // `interval` apart, so over N intervals each should fire N times once past the initial
    // jitter window. We collect until at least N*2 ticks have been received, then shut
    // down. A generous upper bound on advances keeps the test from stalling if the
    // scheduler's actual rate is slightly below ideal.
    let mut per_target: HashMap<String, u32> = HashMap::new();
    for _ in 0..20 {
        tokio::time::advance(interval).await;
        // Drain any ticks produced by this advance before advancing again.
        while let Ok(tick) = events_rx.try_recv() {
            assert!(
                matches!(tick.outcome, TickOutcome::Fresh | TickOutcome::Unchanged),
                "fetch should not fail, got: {:?}",
                tick.outcome
            );
            *per_target.entry(format!("{}", tick.locator)).or_insert(0) = per_target
                .get(&format!("{}", tick.locator))
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
        }
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok");

    let a_count = per_target.get(&format!("{target_a}")).copied().unwrap_or(0);
    let b_count = per_target.get(&format!("{target_b}")).copied().unwrap_or(0);
    assert!(
        a_count >= 8,
        "target_a should have been polled many times, got {a_count}"
    );
    assert!(
        b_count >= 8,
        "target_b should have been polled many times, got {b_count}"
    );
    // The two counts must not differ by more than one, which is the fairness bound: the
    // jitter puts them at different offsets but the interval between polls is identical.
    let diff = a_count.abs_diff(b_count);
    assert!(
        diff <= 1,
        "target_a={a_count}, target_b={b_count}: one target starved the other",
    );
}
