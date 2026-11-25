//! Slice 0017: the review-requested rule fires on a draft→ready transition, is idempotent
//! against the current review-request list (case-insensitively), skips the PR author from
//! the configured list, and short-circuits cleanly when no reviewers are configured, when
//! the in-memory prior is absent (fresh task), or when the reverse transition is observed.
//!
//! The 2026-11-25 adversarial review removed the per-tick store walk that stood in for a
//! "walk history when prior is None" scenario: the walk re-read the observation poll had
//! just written and could never change the gate's outcome. A fresh task after restart is
//! still handled correctly because `open_store_and_seed_prior` seeds `prior` from the
//! store before the first tick, so the in-memory `prior` is never `None` for a PR the
//! store already knows about. The `not_applicable_when_prior_is_none` test documents the
//! remaining case (truly first observation of this PR).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::assert_is_empty
)]

mod common;

use common::{FakeClient, locator, login, pr_with_draft, pr_with_draft_and_author};
use contributron_domain::actor::{ExecutingAgent, UserId, WorkerId};
use contributron_domain::pull_request::AuthorLogin;
use contributron_react::{
    REVIEW_REQUESTED_RULE_AGENT_ID, ReactionKind, ReactionOutcome, react_review_requested,
};

fn owner_worker() -> (UserId, WorkerId) {
    (
        UserId::new("alice").expect("static test id"),
        WorkerId::new("poll").expect("static test id"),
    )
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
async fn not_applicable_when_prior_is_none() {
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    let new = pr_with_draft(false);
    let reviewers = vec![login("reviewer-one")];

    // `open_store_and_seed_prior` seeds `prior` from the store before the first tick, so the
    // in-memory `prior` is only `None` for a PR the task has never observed before: a
    // single observation cannot evidence a transition, so the rule reports NotApplicable.
    let outcome = react_review_requested(&owner, &worker, &loc, None, &new, &client, &reviewers)
        .await
        .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::NotApplicable));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when prior is None",
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

#[tokio::test]
async fn skips_pr_author_from_configured_reviewers() {
    // 2026-11-25 adversarial review finding 1: GitHub rejects a review request that names
    // the PR author with 422, failing the whole batched call. The rule drops the author
    // from the request before writing. Case-insensitive because GitHub logins are
    // ASCII-insensitive (ties finding 5 into the same fix).
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers([]);
    let prior = pr_with_draft_and_author(true, "Alice");
    let new = pr_with_draft_and_author(false, "Alice");
    let reviewers = vec![login("alice"), login("bob")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    match outcome {
        ReactionOutcome::Posted { receipt } => match &receipt.kind {
            ReactionKind::ReviewRequested { reviewers: req } => {
                let names: Vec<String> = req.iter().map(|r| r.as_str().to_owned()).collect();
                assert_eq!(
                    names,
                    vec!["bob"],
                    "author must be skipped even when its case differs",
                );
            }
            other => panic!("expected ReviewRequested receipt, got {other:?}"),
        },
        other => panic!("expected Posted, got {other:?}"),
    }
    let calls = client.request_reviewers_calls();
    assert_eq!(
        calls,
        vec![vec!["bob".to_owned()]],
        "only non-author reviewers reach the provider",
    );
}

#[tokio::test]
async fn already_settled_when_only_candidate_is_the_pr_author() {
    // Companion to `skips_pr_author_from_configured_reviewers`: if the author is the only
    // configured reviewer, the rule treats the write as already-settled rather than
    // posting a receipt for an empty request.
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers([]);
    let prior = pr_with_draft_and_author(true, "alice");
    let new = pr_with_draft_and_author(false, "alice");
    let reviewers = vec![login("alice")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::AlreadySettled));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when the only configured reviewer is the author",
    );
}

#[tokio::test]
async fn case_insensitive_against_present_reviewers() {
    // 2026-11-25 adversarial review finding 5: GitHub logins are ASCII-insensitive. Before
    // the fix, configured `Alice` was treated as missing when `alice` was already on the
    // list, producing a false Posted audit entry and a duplicate request. The compare is
    // now `eq_ignore_ascii_case`.
    let (owner, worker) = owner_worker();
    let loc = locator();
    let client = FakeClient::default();
    client.set_requested_reviewers(["alice"]);
    let prior = pr_with_draft(true);
    let new = pr_with_draft(false);
    let reviewers = vec![login("Alice")];

    let outcome = react_review_requested(
        &owner,
        &worker,
        &loc,
        Some(&prior),
        &new,
        &client,
        &reviewers,
    )
    .await
    .expect("rule");

    assert!(matches!(outcome, ReactionOutcome::AlreadySettled));
    assert!(
        client.request_reviewers_calls().is_empty(),
        "no write when the configured login differs only in case from a present one",
    );
}
