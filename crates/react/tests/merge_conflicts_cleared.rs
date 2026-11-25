//! Slice 0015: the merge-conflicts-cleared rule's Definition-of-Done scenarios, plus the
//! correctness fixes from the 2026-11-25 adversarial review (Unknown-bridge, transient
//! live read).
//!
//! Every scenario is assertable against the fake provider client in [`common`], which now
//! doubles as the [`MergeableHistory`] source so the history-walk path is exercised too.
//! The real GitHub REST read lives in `crates/github/src/client.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assertions_on_constants
)]

mod common;

use common::{FakeClient, LiveMerge, locator, pr_with_mergeable};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_domain::pull_request::Mergeable;
use contributron_react::{
    MERGE_RULE_AGENT_ID, MERGE_RULE_MARKER, ReactError, ReactionKind, ReactionOutcome,
    react_merge_conflicts_cleared,
};

fn owner() -> UserId {
    UserId::new("alice").expect("static test id")
}
fn worker() -> WorkerId {
    WorkerId::new("poll").expect("static test id")
}

/// Scenario 1: a stored `Conflicting → Mergeable` transition with a clean live read posts
/// one marker comment.
#[tokio::test]
async fn conflict_to_mergeable_with_clean_live_read_posts_once_with_marker() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    match outcome {
        ReactionOutcome::Posted { receipt } => {
            let body = match &receipt.kind {
                ReactionKind::Commented { body } => body.clone(),
                other => panic!("expected Commented receipt, got {other:?}"),
            };
            assert!(
                body.contains(MERGE_RULE_MARKER),
                "post lacked marker: {body}",
            );
            match &receipt.attribution.agent {
                contributron_domain::actor::ExecutingAgent::Agent { id } => {
                    assert_eq!(id.as_str(), MERGE_RULE_AGENT_ID);
                }
                contributron_domain::actor::ExecutingAgent::NoAgent => {
                    panic!("merge-conflicts-cleared must record an agent id");
                }
            }
        }
        other => panic!("expected Posted, got {other:?}"),
    }

    let posts = client.posts();
    assert_eq!(posts.len(), 1, "exactly one post, got: {posts:?}");
    assert!(posts[0].contains(MERGE_RULE_MARKER));
}

/// Scenario 2: steady-state `Mergeable → Mergeable` does not fire.
#[tokio::test]
async fn steady_state_mergeable_does_not_fire() {
    let client = FakeClient::default();
    let prior = pr_with_mergeable(Mergeable::Mergeable);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Scenario 3: the first observation of a pull request with no stored history never
/// fires.
#[tokio::test]
async fn first_observation_with_mergeable_and_no_history_does_not_fire() {
    let client = FakeClient::default();
    client.set_history(None);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        None,
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Scenario 4: payload said the transition fired, but the live read still reports a
/// conflict — the write path yields to the provider (standing invariant 4). No post.
#[tokio::test]
async fn live_read_disagreement_blocks_post() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(
        matches!(outcome, ReactionOutcome::NotApplicable),
        "a live read that still reports a conflict must block the post, got {outcome:?}",
    );
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Scenario 5: a prior marker comment on the pull request blocks a second post.
#[tokio::test]
async fn prior_marker_comment_blocks_second_post() {
    let client =
        FakeClient::with_existing(std::iter::once(format!("{MERGE_RULE_MARKER}\nold body")));
    client.set_live(LiveMerge::NoConflict);

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(
        matches!(outcome, ReactionOutcome::AlreadySettled),
        "marker already present must short-circuit, got {outcome:?}",
    );
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Finding 1 of the 2026-11-25 review: `Conflicting → Unknown → Mergeable` is the common
/// GitHub path (`mergeable: null` while the server recomputes after a push). The in-memory
/// prior has already advanced to `Unknown`, so the rule must walk back through the store
/// history to recover the Conflicting evidence. With a history that reports Conflicting,
/// the rule fires and posts.
#[tokio::test]
async fn unknown_prior_with_conflicting_history_fires_and_posts() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    client.set_history(Some(Mergeable::Conflicting));

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(
        matches!(outcome, ReactionOutcome::Posted { .. }),
        "Unknown prior + Conflicting history must still fire, got {outcome:?}",
    );
    assert_eq!(client.posts().len(), 1);
}

/// Finding 1, symmetrical half: `Unknown → Mergeable` with no Conflicting anywhere in the
/// history is NOT a cleared-conflict transition. The provider computing from scratch into
/// clean is not evidence conflicts ever existed.
#[tokio::test]
async fn unknown_prior_with_no_conflicting_history_does_not_fire() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    client.set_history(Some(Mergeable::Mergeable));

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect("react");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Finding 2 of the 2026-11-25 review: when the live read reports "still computing", the
/// rule returns an error so the poll loop's park-and-retry path kicks in. The reactor
/// must not silently return `NotApplicable` on a transient non-answer (that would advance
/// prior and lose the transition).
#[tokio::test]
async fn live_read_computing_errors_so_the_retry_path_fires() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::Computing);

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let error = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect_err("Computing must surface as ReactError so the poll loop can park");

    assert!(
        matches!(error, ReactError::Provider(_)),
        "Computing should map to Provider error, got {error:?}",
    );
    assert_eq!(client.posts(), Vec::<String>::new());
}

/// Finding 1 of the 2026-11-25 review (history failure branch): when the rule needs to
/// walk the history and the store query errors, the rule surfaces a `History` error —
/// the fail-loud route — rather than silently returning `NotApplicable`.
#[tokio::test]
async fn history_error_on_unknown_prior_surfaces_as_react_error() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    client.set_history_err("sqlite is on fire");

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let error = react_merge_conflicts_cleared(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect_err("history errors must not be swallowed");

    assert!(
        matches!(error, ReactError::History(ref m) if m.contains("sqlite is on fire")),
        "unexpected error: {error:?}",
    );
    assert_eq!(client.posts(), Vec::<String>::new());
}
