//! Slice 0011, scenario "changed state": an observation whose content changes between
//! ticks produces a second store row rather than being collapsed into the first.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_poll::{PollConfig, Tick, TickOutcome, run_loop};
use contributron_store::EventStore;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_changed_observation_between_ticks_produces_a_second_row() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    // Five calls return title A; the sixth onward return title B. Both are the same PR
    // number so the store row is keyed by the content digest, not by the locator.
    let obs_a = observation(7, "initial title");
    let obs_b = observation(7, "renamed");
    let script = vec![
        Ok(obs_a.clone()),
        Ok(obs_a.clone()),
        Ok(obs_a.clone()),
        Ok(obs_a.clone()),
        Ok(obs_a.clone()),
        Ok(obs_b.clone()),
    ];
    let fetch = scripted_fetch(script);

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: WorkerId::new("cli"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    let mut outcomes: Vec<TickOutcome> = Vec::new();
    for _ in 0..6 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        outcomes.push(events_rx.recv().await.expect("a tick").outcome);
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok");

    assert!(
        matches!(outcomes[0], TickOutcome::Fresh),
        "tick 0: {:?}",
        outcomes[0]
    );
    for (i, outcome) in outcomes[1..5].iter().enumerate() {
        assert!(
            matches!(outcome, TickOutcome::Unchanged),
            "tick {} (title A): {outcome:?}",
            i.saturating_add(1),
        );
    }
    assert!(
        matches!(outcomes[5], TickOutcome::Fresh),
        "tick 5 (title B) must be a new row, got: {:?}",
        outcomes[5]
    );

    // Verify the store holds two rows, not one.
    let store =
        EventStore::open(&store_path, UserId::new("local-operator")).expect("store reopens");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 2, "two distinct observations persisted");
    let titles: Vec<&str> = rows.iter().map(|o| o.pull_request.title.as_str()).collect();
    assert!(
        titles.contains(&"initial title"),
        "missing A, got {titles:?}"
    );
    assert!(titles.contains(&"renamed"), "missing B, got {titles:?}");
}
