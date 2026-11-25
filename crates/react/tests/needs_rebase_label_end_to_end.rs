//! Slice 0016 end-to-end: the poll loop writes distinct observations across ticks, the
//! reactor sees the Mergeable→Conflicting→Mergeable cycle, and a single apply then a
//! single remove reach the fake provider. A second scenario restarts with the label
//! already present on the pull request (as if a prior process did the apply) and checks
//! the restart does not reapply.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown
)]

mod common;

use common::{FakeClient, LiveMerge};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
};
use contributron_poll::{
    Fetch, FetchError, FetchFuture, PollConfig, PullRequestLocator, ReactionReport, Reactor,
    ReactorFuture, Tick, TickOutcome, run_loop_with_reactor,
};
use contributron_react::{
    LABEL_RULE_AGENT_ID, NEEDS_REBASE_LABEL, ReactionOutcome, react_needs_rebase_label,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

fn obs(number: u64, title: &str, mergeable: Mergeable) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(number).expect("nonzero pr number"),
        title,
        false,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("alice").expect("nonempty login"),
        },
        mergeable,
        Vec::new(),
    )
}

fn pull_locator(number: u64) -> PullRequestLocator {
    PullRequestLocator {
        owner: "alice".into(),
        repo: "project".into(),
        number,
    }
}

fn scripted_fetch(script: Vec<Result<ObservedPullRequest, &'static str>>) -> Fetch {
    let script = Arc::new(Mutex::new(script));
    Arc::new(move |_locator| {
        let script = Arc::clone(&script);
        Box::pin(async move {
            let mut guard = script.lock().expect("scripted_fetch mutex");
            let entry = if guard.len() > 1 {
                guard.remove(0)
            } else {
                guard
                    .first()
                    .cloned()
                    .unwrap_or(Err("scripted_fetch: empty"))
            };
            entry.map_err(|m| FetchError(m.to_string()))
        }) as FetchFuture
    })
}

fn reactor_from(client: Arc<FakeClient>) -> Reactor {
    let owner = UserId::new("local-operator").expect("static test id");
    let worker = WorkerId::new("cli").expect("static test id");
    Arc::new(move |locator, prior, new| {
        let client = Arc::clone(&client);
        let owner = owner.clone();
        let worker = worker.clone();
        Box::pin(async move {
            match react_needs_rebase_label(
                &owner,
                &worker,
                &locator,
                prior.as_ref(),
                &new,
                client.as_ref(),
                client.as_ref(),
            )
            .await
            {
                Ok(ReactionOutcome::NotApplicable) => ReactionReport::NotApplicable,
                Ok(ReactionOutcome::AlreadySettled) => ReactionReport::AlreadySettled {
                    agent_ids: vec![LABEL_RULE_AGENT_ID.to_owned()],
                    errors: Vec::new(),
                },
                Ok(ReactionOutcome::Posted { .. }) => ReactionReport::Posted {
                    agent_ids: vec![LABEL_RULE_AGENT_ID.to_owned()],
                    errors: Vec::new(),
                },
                Err(e) => ReactionReport::Failed {
                    error: format!("{e}"),
                },
            }
        }) as ReactorFuture
    })
}

/// DoD scenarios 1-2 driven end-to-end: Mergeable → Conflicting → Mergeable applies the
/// label on tick 2 and removes it on tick 3.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn conflict_cycle_applies_then_removes_across_two_ticks() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(7);

    let script = vec![
        Ok(obs(7, "mergeable-1", Mergeable::Mergeable)),
        Ok(obs(7, "conflicting", Mergeable::Conflicting)),
        Ok(obs(7, "mergeable-2", Mergeable::Mergeable)),
    ];
    let fetch = scripted_fetch(script);

    let client = Arc::new(FakeClient::default());
    // The live read mirrors the scripted mergeable: tick 1 and tick 3 see no conflict;
    // tick 2 sees a conflict. The fake's single-slot `live` is flipped before each tick
    // completes by the test driver.
    client.set_live(LiveMerge::NoConflict);
    let reactor = reactor_from(Arc::clone(&client));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator").expect("static test id"),
        worker: WorkerId::new("cli").expect("static test id"),
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

    // Tick 1: fresh Mergeable write, no prior, no action.
    tokio::time::advance(interval.saturating_mul(2)).await;
    ticks.push(events_rx.recv().await.expect("tick 1"));

    // Flip live to StillConflicting before the next tick runs.
    client.set_live(LiveMerge::StillConflicting);
    tokio::time::advance(interval.saturating_mul(2)).await;
    ticks.push(events_rx.recv().await.expect("tick 2"));

    // Flip live back to NoConflict for the remove direction.
    client.set_live(LiveMerge::NoConflict);
    tokio::time::advance(interval.saturating_mul(2)).await;
    ticks.push(events_rx.recv().await.expect("tick 3"));

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    // Tick 1: first observation, no transition evidence.
    assert!(matches!(ticks[0].outcome, TickOutcome::Fresh));
    assert!(matches!(
        ticks[0].reaction,
        Some(ReactionReport::NotApplicable),
    ));

    // Tick 2: Mergeable→Conflicting → apply.
    assert!(matches!(ticks[1].outcome, TickOutcome::Fresh));
    assert!(
        matches!(ticks[1].reaction, Some(ReactionReport::Posted { .. })),
        "tick 2 reaction should be Posted, got {:?}",
        ticks[1].reaction,
    );

    // Tick 3: Conflicting→Mergeable → remove.
    assert!(matches!(ticks[2].outcome, TickOutcome::Fresh));
    assert!(
        matches!(ticks[2].reaction, Some(ReactionReport::Posted { .. })),
        "tick 3 reaction should be Posted, got {:?}",
        ticks[2].reaction,
    );

    assert_eq!(client.added_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
    assert_eq!(client.removed_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
}

/// Marker-equivalent: on a restart with the label already attached, a re-observation of
/// Conflicting does not reapply the label. The label set is the source of truth for
/// idempotency — labels survive a process kill, so a prior apply remains visible.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn live_label_set_matches_rule_state_after_restart() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(7);

    let client = Arc::new(FakeClient::default());
    client.set_live(LiveMerge::StillConflicting);
    client.set_labels([NEEDS_REBASE_LABEL]);

    let script = vec![
        Ok(obs(7, "mergeable", Mergeable::Mergeable)),
        Ok(obs(7, "conflicting", Mergeable::Conflicting)),
    ];
    let fetch = scripted_fetch(script);
    let reactor = reactor_from(Arc::clone(&client));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator").expect("static test id"),
        worker: WorkerId::new("cli").expect("static test id"),
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

    // Tick 2 crossed Mergeable→Conflicting, but the label was already present, so the
    // rule short-circuits.
    assert!(
        matches!(
            ticks[1].reaction,
            Some(ReactionReport::AlreadySettled { .. })
        ),
        "tick 2 reaction should be AlreadySettled, got {:?}",
        ticks[1].reaction,
    );
    assert!(
        client.added_labels().is_empty(),
        "no new label apply, got {:?}",
        client.added_labels(),
    );
}
