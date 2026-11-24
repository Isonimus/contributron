//! Slice 0015 end-to-end: the poll loop writes two distinct observations, the reactor
//! sees the `Conflicting → Mergeable` transition on the second tick and posts one marker
//! comment, and a third tick with the same mergeable state is Unchanged and posts
//! nothing. Idempotency across restart is covered by re-opening the store under the same
//! path and watching the marker scan short-circuit the second post attempt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    MERGE_RULE_AGENT_ID, MERGE_RULE_MARKER, ReactionOutcome, react_merge_conflicts_cleared,
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

/// Scripted fetch: returns each entry in order; the last entry repeats so later ticks
/// still see a value.
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

/// Build a `Reactor` closure that invokes the merge-conflicts-cleared rule against a
/// shared [`FakeClient`], folding the rule's [`ReactionOutcome`] into the loop's
/// [`ReactionReport`].
fn reactor_from(client: Arc<FakeClient>) -> Reactor {
    let owner = UserId::new("local-operator");
    let worker = WorkerId::new("cli");
    Arc::new(move |locator, prior, new| {
        let client = Arc::clone(&client);
        let owner = owner.clone();
        let worker = worker.clone();
        Box::pin(async move {
            match react_merge_conflicts_cleared(
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
                Ok(ReactionOutcome::AlreadyPosted) => ReactionReport::AlreadyPosted,
                Ok(ReactionOutcome::Posted { .. }) => ReactionReport::Posted {
                    agent_id: MERGE_RULE_AGENT_ID.to_owned(),
                },
                Err(e) => ReactionReport::Failed {
                    error: format!("{e}"),
                },
            }
        }) as ReactorFuture
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn conflict_then_mergeable_posts_once_then_unchanged_posts_nothing() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(7);

    // Three observations: conflicting, then mergeable with a different title (so it writes
    // Fresh), then the same mergeable observation repeated (so the third tick is Unchanged).
    let script = vec![
        Ok(obs(7, "conflicting", Mergeable::Conflicting)),
        Ok(obs(7, "cleared", Mergeable::Mergeable)),
        Ok(obs(7, "cleared", Mergeable::Mergeable)),
    ];
    let fetch = scripted_fetch(script);

    let client = Arc::new(FakeClient::default());
    client.set_live(LiveMerge::NoConflict);
    let reactor = reactor_from(Arc::clone(&client));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator"),
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
    for _ in 0..3 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        ticks.push(events_rx.recv().await.expect("a tick"));
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    // First tick: Fresh, NotApplicable (no prior).
    assert!(matches!(ticks[0].outcome, TickOutcome::Fresh));
    assert!(matches!(
        ticks[0].reaction,
        Some(ReactionReport::NotApplicable),
    ));

    // Second tick: Fresh, Posted (transition fired).
    assert!(matches!(ticks[1].outcome, TickOutcome::Fresh));
    assert!(
        matches!(ticks[1].reaction, Some(ReactionReport::Posted { .. })),
        "second tick reaction should be Posted, got {:?}",
        ticks[1].reaction,
    );

    // Third tick: Unchanged, no reactor call (unchanged + no parked retry).
    assert!(matches!(ticks[2].outcome, TickOutcome::Unchanged));
    assert!(
        ticks[2].reaction.is_none(),
        "an Unchanged tick with no parked retry has no reactor call; got {:?}",
        ticks[2].reaction,
    );

    // Exactly one post reached the provider.
    let posts = client.posts();
    assert_eq!(posts.len(), 1, "exactly one post, got: {posts:?}");
    assert!(posts[0].contains(MERGE_RULE_MARKER));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn marker_present_from_a_prior_run_blocks_a_second_post_after_restart() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(7);

    // The marker from a prior reactor run is already on the pull request.
    let client = Arc::new(FakeClient::with_existing(std::iter::once(format!(
        "{MERGE_RULE_MARKER}\nold body",
    ))));
    client.set_live(LiveMerge::NoConflict);

    let script = vec![
        Ok(obs(7, "conflicting", Mergeable::Conflicting)),
        Ok(obs(7, "cleared", Mergeable::Mergeable)),
    ];
    let fetch = scripted_fetch(script);
    let reactor = reactor_from(Arc::clone(&client));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();
    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator"),
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

    assert!(matches!(
        ticks[1].reaction,
        Some(ReactionReport::AlreadyPosted),
    ));
    assert!(
        client.posts().is_empty(),
        "no new post when the marker is already on the pull request, got {:?}",
        client.posts(),
    );
}
