//! Slice 0017: the review-requested rule fires on a draft→ready transition, is idempotent
//! against the current review-request list, walks the store when the in-memory prior is
//! absent, and short-circuits cleanly when no reviewers are configured or the reverse
//! transition is observed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::assert_is_empty
)]

mod common;

use common::{FakeClient, locator, login, pr_with_draft};
use contributron_domain::actor::{ExecutingAgent, UserId, WorkerId};
use contributron_domain::pull_request::AuthorLogin;
use contributron_react::{
    REVIEW_REQUESTED_RULE_AGENT_ID, ReactionKind, ReactionOutcome, react_review_requested,
};

fn owner_worker() -> (UserId, WorkerId) {
    (UserId::new("alice"), WorkerId::new("poll"))
}

#[tokio::test]
async fn fires_on_draft_to_ready_transition() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers([]);
    let prior = pr_with_draft(true);
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one"), login("reviewer-two")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    match outcome {
        ReactionOutcome::Posted { receipt } => {
            match &receipt.kind {
                ReactionKind::ReviewRequested { reviewers: req } => {
                    let names: Vec<String> = req.iter().map(|r| r.as_str().to_owned()).collect();
                    assert_eq!(names, vec!["reviewer-one", "reviewer-two"]);
                }
                other => panic!("expected ReviewRequested receipt, got {other:?}"),
            }
            match &receipt.attribution.agent {
                ExecutingAgent::Agent { id } => {
                    assert_eq!(id.as_str(), REVIEW_REQUESTED_RULE_AGENT_ID);
                }
                ExecutingAgent::NoAgent => panic!("expected an Agent attribution, got NoAgent"),
            }
        }
        other => panic!("expected Posted, got {other:?}"),
    }
    let calls = client.request_reviewers_calls();
    assert_eq!(
        calls,
        vec![vec!["reviewer-one".to_owned(), "reviewer-two".to_owned()]],
        "write should carry both missing reviewers in the configured order",
    );
}

#[tokio::test]
async fn no_transition_on_stable_ready() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    // The prior was already ready; the review-request list is empty; still no fire.
    client.set_requested_reviewers([]);
    let prior = pr_with_draft(false);
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when no transition",
    );
}

#[tokio::test]
async fn already_settled_when_every_reviewer_present() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers(["reviewer-one", "reviewer-two"]);
    let prior = pr_with_draft(true);
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one"), login("reviewer-two")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::AlreadySettled));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when every reviewer is already on the list",
    );
}

#[tokio::test]
async fn walks_history_when_prior_is_none() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    // The in-memory prior is None (fresh task after restart). The store history says the
    // last observation was draft, so the transition fires.
    client.set_draft_history(Some(true));
    client.set_requested_reviewers([]);
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one")];

    let outcome = react_review_requested(
        &owner, &worker, &loc, None, &new, &client, &client, &reviewers,
    )
    .await
    .expect("rule");

    match outcome {
        ReactionOutcome::Posted { receipt } => match &receipt.kind {
            ReactionKind::ReviewRequested { reviewers: req } => {
                assert_eq!(req.len(), 1);
                assert_eq!(req[0].as_str(), "reviewer-one");
            }
            other => panic!("expected ReviewRequested receipt, got {other:?}"),
        },
        other => panic!("expected Posted, got {other:?}"),
    }
}

#[tokio::test]
async fn not_applicable_when_history_is_empty() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    // No in-memory prior AND no stored history: no transition can be evidenced.
    client.set_draft_history(None);
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one")];

    let outcome = react_review_requested(
        &owner, &worker, &loc, None, &new, &client, &client, &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when the history walk is empty",
    );
}

#[tokio::test]
async fn not_applicable_when_no_reviewers_configured() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    let prior = pr_with_draft(true);
    let new = pr_with_draft(false);
    let reviewers: Vec<AuthorLogin> = Vec::new();

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    // With no reviewers configured the client is not touched at all: not for the list, not
    // for the write. A token-less watch is safe.
    assert!(client.request_reviewers_calls().is_empty());
    assert_eq!(client.requested_reviewers_snapshot(), Vec::<String>::new());
}

#[tokio::test]
async fn subset_requests_only_missing_reviewers() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers(["reviewer-one"]);
    let prior = pr_with_draft(true);
    let new = pr_with_draft(false);
    // Configured: three reviewers. One is already on the list, two are not. Write must
    // carry exactly the two that are missing, in the configured order.
    let reviewers = vec![
        login("reviewer-one"),
        login("reviewer-two"),
        login("reviewer-three"),
    ];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    match outcome {
        ReactionOutcome::Posted { receipt } => match &receipt.kind {
            ReactionKind::ReviewRequested { reviewers: req } => {
                let names: Vec<String> = req.iter().map(|r| r.as_str().to_owned()).collect();
                assert_eq!(names, vec!["reviewer-two", "reviewer-three"]);
            }
            other => panic!("expected ReviewRequested receipt, got {other:?}"),
        },
        other => panic!("expected Posted, got {other:?}"),
    }
    let calls = client.request_reviewers_calls();
    assert_eq!(
        calls,
        vec![vec!["reviewer-two".to_owned(), "reviewer-three".to_owned()]],
        "write must carry only the missing subset, not the full configured list",
    );
}

#[tokio::test]
async fn ready_to_draft_is_not_applicable() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    let prior = pr_with_draft(false);
    let new = pr_with_draft(true);
    let reviewers = vec![login("reviewer-one")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "the reverse transition must not fire",
    );
}
