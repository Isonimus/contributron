//! Finding 1 of the 2026-11-25 adversarial review of slice 0016. A reactor that returns
//! `Posted { errors: [..] }` or `AlreadySettled { errors: [..] }` on a fresh tick parks the
//! new observation and does not advance `prior`: a sibling rule's error means at least one
//! rule's comparison never completed, so the transition is retried on the next Unchanged
//! tick. Without this, a rule that errors beside a successful peer loses its transition for
//! good (the prior moves past the gate and the rule never fires again).
//!
//! Rules are idempotent — the posting rule's second run reports `AlreadySettled`, and the
//! previously-failing rule gets to post. Re-running is safe.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

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
async fn a_partial_reaction_is_retried_on_the_next_unchanged_tick() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    let obs = observation(7, "cleared");
    let fetch = scripted_fetch(vec![Ok(obs.clone())]);

    let calls = Arc::new(Mutex::new(0u32));
    let seen: Arc<Mutex<Vec<Pair>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_for_reactor = Arc::clone(&calls);
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
                // One rule posted, another errored: a Posted-with-errors tick. Invariant 6
                // records the write; the error means the sibling's transition did not run.
                ReactionReport::Posted {
                    agent_ids: vec!["rule:threads-cleared".into()],
                    errors: vec![(
                        "rule:needs-rebase-label".into(),
                        "live_merge_has_conflict: 503".into(),
                    )],
                }
            } else {
                // Retry: the first rule finds its marker already in place, the second now
                // succeeds. Both rules are idempotent, so re-running is safe.
                ReactionReport::Posted {
                    agent_ids: vec!["rule:needs-rebase-label".into()],
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
        worker: contributron_domain::actor::WorkerId::new("cli"),
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
        matches!(
            ticks[0].reaction,
            Some(ReactionReport::Posted { ref errors, .. }) if !errors.is_empty()
        ),
        "first reaction should be Posted with a non-empty errors list, got {:?}",
        ticks[0].reaction,
    );

    assert!(
        matches!(ticks[1].outcome, TickOutcome::Unchanged),
        "second tick should be Unchanged, got {:?}",
        ticks[1].outcome,
    );
    assert!(
        matches!(
            ticks[1].reaction,
            Some(ReactionReport::Posted { ref errors, .. }) if errors.is_empty()
        ),
        "second reaction should retry and Post cleanly, got {:?}",
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
        "retry prior should still be None because the partial-failure call did not advance it",
    );
    assert_eq!(
        pairs[1].1, "cleared",
        "retry should carry the parked observation, not a stale one",
    );
}
