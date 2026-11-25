//! Slice 0016: the needs-rebase-label rule's Definition-of-Done scenarios, plus the
//! history-walk and fail-loud regressions.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assertions_on_constants,
    clippy::doc_markdown
)]

mod common;

use common::{FakeClient, LiveMerge, locator, pr_with_mergeable};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_domain::pull_request::{Label, Mergeable};
use contributron_react::{
    LABEL_RULE_AGENT_ID, NEEDS_REBASE_LABEL, ReactError, ReactionKind, ReactionOutcome,
    react_needs_rebase_label,
};
use std::collections::BTreeSet;

fn owner() -> UserId {
    UserId::new("alice").expect("static test id")
}
fn worker() -> WorkerId {
    WorkerId::new("poll").expect("static test id")
}

fn assert_label_applied_receipt(outcome: &ReactionOutcome) {
    match outcome {
        ReactionOutcome::Posted { receipt } => {
            match &receipt.kind {
                ReactionKind::LabelApplied { label } => {
                    assert_eq!(label.as_str(), NEEDS_REBASE_LABEL);
                }
                other => panic!("expected LabelApplied, got {other:?}"),
            }
            match &receipt.attribution.agent {
                contributron_domain::actor::ExecutingAgent::Agent { id } => {
                    assert_eq!(id.as_str(), LABEL_RULE_AGENT_ID);
                }
                contributron_domain::actor::ExecutingAgent::NoAgent => {
                    panic!("needs-rebase-label must record an agent id");
                }
            }
        }
        other => panic!("expected Posted, got {other:?}"),
    }
}

fn assert_label_removed_receipt(outcome: &ReactionOutcome) {
    match outcome {
        ReactionOutcome::Posted { receipt } => match &receipt.kind {
            ReactionKind::LabelRemoved { label } => {
                assert_eq!(label.as_str(), NEEDS_REBASE_LABEL);
            }
            other => panic!("expected LabelRemoved, got {other:?}"),
        },
        other => panic!("expected Posted, got {other:?}"),
    }
}

fn needs_rebase_set() -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    s.insert(NEEDS_REBASE_LABEL.to_owned());
    s
}

/// DoD 1: Mergeable → Conflicting with absent label applies the label exactly once.
#[tokio::test]
async fn conflict_transition_with_live_read_and_absent_label_applies_once() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);
    // Labels start empty (default).

    let prior = pr_with_mergeable(Mergeable::Mergeable);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let outcome = react_needs_rebase_label(
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

    assert_label_applied_receipt(&outcome);
    assert_eq!(client.added_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
    assert_eq!(client.removed_labels(), Vec::<String>::new());
    assert_eq!(client.labels_snapshot(), needs_rebase_set());
}

/// DoD 2: Conflicting → Mergeable with the label present removes it exactly once.
#[tokio::test]
async fn mergeable_transition_with_present_label_removes_once() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    client.set_labels([NEEDS_REBASE_LABEL]);

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_needs_rebase_label(
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

    assert_label_removed_receipt(&outcome);
    assert_eq!(client.removed_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
    assert_eq!(client.added_labels(), Vec::<String>::new());
    assert!(
        client.labels_snapshot().is_empty(),
        "label should be gone, got {:?}",
        client.labels_snapshot(),
    );
}

/// Label-level idempotency: apply-direction with the label already present short-circuits.
#[tokio::test]
async fn conflict_transition_with_label_already_present_is_no_op() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);
    client.set_labels([NEEDS_REBASE_LABEL]);

    let prior = pr_with_mergeable(Mergeable::Mergeable);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let outcome = react_needs_rebase_label(
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
        "expected AlreadySettled, got {outcome:?}",
    );
    assert_eq!(client.added_labels(), Vec::<String>::new());
    assert_eq!(client.removed_labels(), Vec::<String>::new());
}

/// Label-level idempotency symmetric: remove-direction with the label already absent
/// short-circuits.
#[tokio::test]
async fn mergeable_transition_with_label_already_absent_is_no_op() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    // Labels empty (default).

    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_needs_rebase_label(
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
        "expected AlreadySettled, got {outcome:?}",
    );
    assert_eq!(client.added_labels(), Vec::<String>::new());
    assert_eq!(client.removed_labels(), Vec::<String>::new());
}

/// DoD 3: Unknown prior whose history settled on Mergeable, new is Conflicting → apply.
#[tokio::test]
async fn unknown_prior_with_mergeable_history_and_conflicting_new_applies_label() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);
    client.set_history(Some(Mergeable::Mergeable));

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let outcome = react_needs_rebase_label(
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

    assert_label_applied_receipt(&outcome);
    assert_eq!(client.added_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
}

/// Symmetric: Unknown prior whose history settled on Conflicting, new is Mergeable →
/// remove. The Unknown-bridge is bidirectional.
#[tokio::test]
async fn unknown_prior_with_conflicting_history_and_mergeable_new_removes_label() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::NoConflict);
    client.set_history(Some(Mergeable::Conflicting));
    client.set_labels([NEEDS_REBASE_LABEL]);

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let outcome = react_needs_rebase_label(
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

    assert_label_removed_receipt(&outcome);
    assert_eq!(client.removed_labels(), vec![NEEDS_REBASE_LABEL.to_owned()]);
}

/// DoD 4: the live read reports "still computing" during an apply transition → ReactError
/// so the park-and-retry path drives another attempt.
#[tokio::test]
async fn live_read_computing_errors_so_retry_path_fires() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::Computing);

    let prior = pr_with_mergeable(Mergeable::Mergeable);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let error = react_needs_rebase_label(
        &owner(),
        &worker(),
        &locator(),
        Some(&prior),
        &new,
        &client,
        &client,
    )
    .await
    .expect_err("computing must surface");

    assert!(
        matches!(error, ReactError::Provider(_)),
        "expected Provider error, got {error:?}",
    );
    assert_eq!(client.added_labels(), Vec::<String>::new());
    assert_eq!(client.removed_labels(), Vec::<String>::new());
}

/// Fail-loud history read: when the rule needs to walk the history and the store query
/// errors, the rule surfaces `History` rather than silently returning `NotApplicable`.
#[tokio::test]
async fn history_error_on_unknown_prior_surfaces_as_react_error() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);
    client.set_history_err("sqlite is on fire");

    let prior = pr_with_mergeable(Mergeable::Unknown);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let error = react_needs_rebase_label(
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
}

/// First observation (no prior, no history row) with new=Conflicting does not fire: no
/// evidence of a transition.
#[tokio::test]
async fn first_observation_with_conflicting_and_no_history_does_not_fire() {
    let client = FakeClient::default();
    client.set_live(LiveMerge::StillConflicting);
    client.set_history(None);

    let new = pr_with_mergeable(Mergeable::Conflicting);

    let outcome = react_needs_rebase_label(
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
    assert_eq!(client.added_labels(), Vec::<String>::new());
    assert_eq!(client.removed_labels(), Vec::<String>::new());
}

/// The constant must round-trip through `Label::new` (constructor validation).
#[test]
fn needs_rebase_label_constant_is_a_valid_label() {
    let label = Label::new(NEEDS_REBASE_LABEL).expect("constant must be a valid Label");
    assert_eq!(label.as_str(), "needs-rebase");
}

/// The `agent_id` constant matches the slice frontmatter.
#[test]
fn agent_id_literal_matches_slice() {
    assert_eq!(LABEL_RULE_AGENT_ID, "rule:needs-rebase-label");
}
