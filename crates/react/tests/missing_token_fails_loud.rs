//! Scenario 5 (write-path half) of slice 0014, extended to the slice 0016 label write
//! path: a qualifying transition on a reactor whose provider client has no credential
//! returns an error that names the missing credential, and no partial write escapes.
//!
//! The react crate itself has no concept of a token: credentials are a CLI concern
//! (ADR-0013). The scenario's "missing credential" is modelled here as a `ProviderClient`
//! whose every method returns [`ProviderError::Client`] with a message naming the missing
//! credential, which is exactly the shape the CLI wires in when `watch` runs without a
//! `--token-path` and without `CONTRIBUTRON_TOKEN`. The two sides stay matched because the
//! error messages share the ADR-0013 wording.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown
)]

mod common;

use common::{locator, pr, pr_with_mergeable};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_domain::pull_request::{Label, Mergeable};
use contributron_poll::PullRequestLocator;
use contributron_react::{
    ClientFuture, ExistingComment, MergeableHistory, ProviderClient, ProviderError, ReactError,
    react, react_needs_rebase_label,
};
use std::sync::Mutex;

/// A client that fails every method with the ADR-0013 missing-credential message, and
/// records every call so the test can assert no post was attempted after the list call
/// failed.
#[derive(Debug, Default)]
struct NoCredentialClient {
    calls: Mutex<Vec<&'static str>>,
}

const MISSING_CREDENTIAL: &str = "no GitHub token available for the write path. Pass --token-path, set CONTRIBUTRON_TOKEN, \
     or store a github.com credential (ADR-0013).";

impl ProviderClient for NoCredentialClient {
    fn list_comments<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<ExistingComment>> {
        self.calls.lock().expect("calls").push("list_comments");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn post_comment<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _body: &'a str,
    ) -> ClientFuture<'a, ()> {
        self.calls.lock().expect("calls").push("post_comment");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn live_merge_has_conflict<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool> {
        self.calls
            .lock()
            .expect("calls")
            .push("live_merge_has_conflict");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn list_labels<'a>(&'a self, _locator: &'a PullRequestLocator) -> ClientFuture<'a, Vec<Label>> {
        self.calls.lock().expect("calls").push("list_labels");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn add_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        self.calls.lock().expect("calls").push("add_label");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn remove_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        self.calls.lock().expect("calls").push("remove_label");
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }
}

/// A trivial history source that returns `Mergeable::Mergeable` so slice 0016's apply
/// transition detector sees Mergeable→Conflicting without walking any store.
#[derive(Debug, Default)]
struct NoopHistory;

impl MergeableHistory for NoopHistory {
    fn latest_settled(&self, _locator: &PullRequestLocator) -> Result<Option<Mergeable>, String> {
        Ok(Some(Mergeable::Mergeable))
    }
}

fn assert_credential_error(error: &ReactError) {
    match error {
        ReactError::Provider(ProviderError::Client(msg)) => {
            assert!(
                msg.contains("token") || msg.contains("CONTRIBUTRON_TOKEN"),
                "error did not name the missing credential: {msg}",
            );
            assert!(
                msg.contains("ADR-0013"),
                "error did not cite ADR-0013: {msg}",
            );
        }
        ReactError::History(_) => panic!("expected provider error, got {error:?}"),
    }
}

#[tokio::test]
async fn a_qualifying_transition_without_a_credential_fails_loud_and_posts_nothing() {
    let client = NoCredentialClient::default();
    let owner = UserId::new("alice");
    let worker = WorkerId::new("poll");
    let loc = locator();
    let prior = pr(2, 0);
    let new = pr(0, 2);

    let error = react(&owner, &worker, &loc, Some(&prior), &new, &client)
        .await
        .expect_err("a qualifying transition without a credential must fail loud");

    assert_credential_error(&error);

    let calls = client.calls.lock().expect("calls").clone();
    assert_eq!(
        calls,
        vec!["list_comments"],
        "the marker scan is the first and only call; post_comment must not run after a \
         failed scan — got {calls:?}",
    );
}

#[tokio::test]
async fn a_non_qualifying_tick_without_a_credential_still_reports_not_applicable() {
    // The read-path half of scenario 5: when the reactor sees a tick that is not a
    // qualifying transition, the transition check inside react() short-circuits before the
    // client is touched, so no credential is needed and no error is raised.
    let client = NoCredentialClient::default();
    let owner = UserId::new("alice");
    let worker = WorkerId::new("poll");
    let loc = locator();

    let outcome = react(&owner, &worker, &loc, None, &pr(0, 2), &client)
        .await
        .expect("first observation never fires");

    assert!(matches!(
        outcome,
        contributron_react::ReactionOutcome::NotApplicable,
    ));
    assert!(
        client.calls.lock().expect("calls").is_empty(),
        "the client must not be touched when no transition fires",
    );
}

/// Slice 0016 DoD 6: the label rule's apply direction without a credential must surface
/// the same ADR-0013 error. The first client method the rule needs is the live
/// mergeability read; its credential error short-circuits before any label mutation.
#[tokio::test]
async fn apply_transition_without_a_credential_errors_before_any_label_mutation() {
    let client = NoCredentialClient::default();
    let owner = UserId::new("alice");
    let worker = WorkerId::new("poll");
    let loc = locator();
    let history = NoopHistory;
    let prior = pr_with_mergeable(Mergeable::Mergeable);
    let new = pr_with_mergeable(Mergeable::Conflicting);

    let error =
        react_needs_rebase_label(&owner, &worker, &loc, Some(&prior), &new, &history, &client)
            .await
            .expect_err("missing credential must fail loud");

    assert_credential_error(&error);

    let calls = client.calls.lock().expect("calls").clone();
    assert!(
        !calls
            .iter()
            .any(|c| *c == "add_label" || *c == "remove_label"),
        "no label mutation should run after a failed live read, got {calls:?}",
    );
}

/// Slice 0016 DoD 6, remove direction: Conflicting→Mergeable with no credential also errors
/// without a label mutation.
#[tokio::test]
async fn remove_transition_without_a_credential_errors_before_any_label_mutation() {
    #[derive(Debug, Default)]
    struct HistoryConflicting;
    impl MergeableHistory for HistoryConflicting {
        fn latest_settled(
            &self,
            _locator: &PullRequestLocator,
        ) -> Result<Option<Mergeable>, String> {
            Ok(Some(Mergeable::Conflicting))
        }
    }

    let client = NoCredentialClient::default();
    let owner = UserId::new("alice");
    let worker = WorkerId::new("poll");
    let loc = locator();
    let history = HistoryConflicting;
    let prior = pr_with_mergeable(Mergeable::Conflicting);
    let new = pr_with_mergeable(Mergeable::Mergeable);

    let error =
        react_needs_rebase_label(&owner, &worker, &loc, Some(&prior), &new, &history, &client)
            .await
            .expect_err("missing credential must fail loud");

    assert_credential_error(&error);

    let calls = client.calls.lock().expect("calls").clone();
    assert!(
        !calls
            .iter()
            .any(|c| *c == "add_label" || *c == "remove_label"),
        "no label mutation should run after a failed live read, got {calls:?}",
    );
}
