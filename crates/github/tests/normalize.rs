//! Scenarios one and three of slice 0007.
//!
//! Every expected value below is written from the slice's Definition of Done and from the
//! fixture as authored, not from running the normalizer and recording what it returned.

// Tests may panic: that is how a failure is reported. The quality bar permits `unwrap` and
// `expect` here and nowhere else.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, PullRequestAuthor, ThreadResolution,
};
use contributron_github::{NormalizeError, normalize_capture};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading fixture {}: {error}", path.display()))
}

/// Scenario 1: two threads, one resolved, and the scalar fields carried through unchanged.
#[test]
fn a_pull_request_with_two_threads_normalizes_with_one_unresolved() {
    let observed = normalize_capture(&fixture("pull_request_with_threads.json"))
        .expect("the recorded fixture is a valid capture");

    assert_eq!(observed.number.get(), 1);
    assert_eq!(
        observed.title,
        "fix(index): escape backslashes before pipes in table cells"
    );
    assert!(!observed.draft);
    assert_eq!(
        observed.author,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("Isonimus").expect("a non-empty login")
        }
    );
    assert_eq!(observed.mergeable, Mergeable::Mergeable);

    assert_eq!(observed.threads().len(), 2);
    assert_eq!(observed.unresolved_thread_count(), 1);

    // Ordered by identifier, so provider ordering cannot change the value. The fixture lists
    // ...X1a (resolved) before ...X2b (unresolved), and "1" sorts before "2".
    assert_eq!(observed.threads()[0].id.as_str(), "PRRT_kwDOKt3bHM5TqX1a");
    assert_eq!(observed.threads()[0].resolution, ThreadResolution::Resolved);
    assert_eq!(observed.threads()[1].id.as_str(), "PRRT_kwDOKt3bHM5TqX2b");
    assert_eq!(
        observed.threads()[1].resolution,
        ThreadResolution::Unresolved
    );
}

/// Scenario 3, first half: an absent field errors and names itself.
#[test]
fn a_missing_number_errors_naming_the_field() {
    let error = normalize_capture(&fixture("missing_number.json"))
        .expect_err("a payload with no pull request number cannot be normalized");

    match error {
        NormalizeError::MissingField { field } => {
            assert_eq!(field, "data.repository.pullRequest.number");
        }
        other => panic!("expected a missing field error, got {other:?}"),
    }
}

/// Scenario 3, first half again: a null where GitHub's own schema forbids one is an absent
/// value, not a state to guess at. Defaulting it either way would invent or hide an open
/// review thread.
#[test]
fn an_unknown_thread_state_errors_naming_the_field() {
    let error = normalize_capture(&fixture("unknown_thread_state.json"))
        .expect_err("a thread with no resolution state cannot be normalized");

    match error {
        NormalizeError::MissingField { field } => {
            assert_eq!(
                field,
                "data.repository.pullRequest.reviewThreads.nodes[0].isResolved"
            );
        }
        other => panic!("expected a missing field error, got {other:?}"),
    }
}

/// Scenario 3, second half: a value present but outside the domain is a different failure
/// from an absent one, and the error says so rather than collapsing both into "bad input".
#[test]
fn a_pull_request_numbered_zero_is_out_of_domain() {
    let capture =
        fixture("pull_request_with_threads.json").replace("\"number\": 1", "\"number\": 0");

    let error = normalize_capture(&capture)
        .expect_err("providers number pull requests from one, so zero is not a pull request");

    match error {
        NormalizeError::OutOfDomain { field, detail } => {
            assert_eq!(field, "data.repository.pullRequest.number");
            assert_eq!(detail, "pull requests are numbered from one");
        }
        other => panic!("expected an out-of-domain error, got {other:?}"),
    }
}

/// Scenario 3, final clause: no partially populated value escapes. A payload that fails one
/// field produces an error and nothing else, which is asserted by the type rather than by
/// inspection: `normalize_capture` returns a `Result`, so there is no half-built value to
/// return. This test pins the other half of the claim, that an earlier valid field does not
/// cause a later failure to be skipped.
#[test]
fn a_later_invalid_field_is_not_skipped_because_earlier_ones_were_valid() {
    let capture = fixture("pull_request_with_threads.json")
        .replace("\"login\": \"Isonimus\"", "\"login\": \"\"");

    let error = normalize_capture(&capture).expect_err("an empty author login is not a login");

    match error {
        NormalizeError::OutOfDomain { field, detail } => {
            assert_eq!(field, "data.repository.pullRequest.author.login");
            assert_eq!(detail, "an author login cannot be empty");
        }
        other => panic!("expected an out-of-domain error, got {other:?}"),
    }
}

/// Scenario 1, the part the main fixture cannot prove.
///
/// That fixture holds one resolved thread and one unresolved one, so a count that returned
/// the *resolved* threads instead would also return 1 and the test above would still pass.
/// Confirmed by mutation: inverting the filter in `unresolved_thread_count` left the whole
/// suite green. A symmetric fixture cannot detect a symmetric error, so this case breaks the
/// symmetry.
#[test]
fn the_unresolved_count_counts_only_the_unresolved_threads() {
    let mut capture: serde_json::Value =
        serde_json::from_str(&fixture("pull_request_with_threads.json"))
            .expect("the recorded fixture is JSON");
    capture["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"]
        .as_array_mut()
        .expect("the capture holds a list of thread nodes")
        .push(serde_json::json!({
            "id": "PRRT_kwDOKt3bHM5TqX3c",
            "isResolved": false,
        }));

    let observed =
        normalize_capture(&capture.to_string()).expect("three well-formed threads normalize");

    assert_eq!(observed.threads().len(), 3);
    assert_eq!(observed.unresolved_thread_count(), 2);
}

/// A capture that read only the first page of review threads is not an observation.
///
/// Found by adversarial review. The query asks for the first 100 threads and said nothing
/// about the rest, so a pull request with 150 threads normalized into one holding 100 and
/// rendered without complaint. An observation that omits open threads is worse than no
/// observation, because a later rule would act on a pull request it has misread. The ledger
/// still carries the pagination work; this only stops the silence.
#[test]
fn a_truncated_capture_is_refused_rather_than_observed() {
    let mut capture: serde_json::Value =
        serde_json::from_str(&fixture("pull_request_with_threads.json"))
            .expect("the recorded fixture is JSON");
    capture["data"]["repository"]["pullRequest"]["reviewThreads"]["pageInfo"]["hasNextPage"] =
        serde_json::Value::Bool(true);

    let error = normalize_capture(&capture.to_string())
        .expect_err("a capture holding only the first page of threads is incomplete");

    match error {
        NormalizeError::Truncated { field } => {
            assert_eq!(field, "data.repository.pullRequest.reviewThreads");
        }
        other => panic!("expected a truncation error, got {other:?}"),
    }
}

/// A pull request whose author account was deleted is still a pull request.
///
/// Found by adversarial review. GitHub's schema declares `author` nullable and reports null
/// for a deleted account, which is an ordinary state for an old pull request, not a payload
/// this adapter failed to understand. Rejecting it made such a pull request unobservable, and
/// on the live path reported it as the fixtures no longer describing GitHub, which pointed
/// the reader at the wrong problem entirely.
#[test]
fn a_deleted_author_is_observed_as_absent_rather_than_rejected() {
    let mut capture: serde_json::Value =
        serde_json::from_str(&fixture("pull_request_with_threads.json"))
            .expect("the recorded fixture is JSON");
    capture["data"]["repository"]["pullRequest"]["author"] = serde_json::Value::Null;

    let observed = normalize_capture(&capture.to_string())
        .expect("a pull request with no surviving author is still observable");

    assert_eq!(observed.author, PullRequestAuthor::DeletedAccount);
    assert_eq!(observed.number.get(), 1);
    assert_eq!(observed.unresolved_thread_count(), 1);
}
