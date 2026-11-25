//! Shared fixtures for the react crate's integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use contributron_domain::pull_request::{
    AuthorLogin, Label, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
    ReviewThread, ThreadId, ThreadResolution,
};
use contributron_poll::PullRequestLocator;
use contributron_react::{
    ClientFuture, DraftHistory, ExistingComment, MergeableHistory, ProviderClient, ProviderError,
};
use std::collections::BTreeSet;
use std::sync::Mutex;

/// Build an observation with `unresolved` + `resolved` threads, numbered.
pub fn pr(unresolved: usize, resolved: usize) -> ObservedPullRequest {
    let mut threads = Vec::new();
    for i in 0..unresolved {
        threads.push(ReviewThread {
            id: ThreadId::new(format!("u-{i}")).expect("id"),
            resolution: ThreadResolution::Unresolved,
        });
    }
    for i in 0..resolved {
        threads.push(ReviewThread {
            id: ThreadId::new(format!("r-{i}")).expect("id"),
            resolution: ThreadResolution::Resolved,
        });
    }
    ObservedPullRequest::new(
        PullRequestNumber::new(1).expect("number"),
        "example",
        false,
        PullRequestAuthor::DeletedAccount,
        Mergeable::Unknown,
        threads,
    )
}

/// Build an observation whose mergeability is the given value and whose thread list is
/// empty: slice 0015 rule #2 reacts to the merge-state transition and does not look at
/// threads.
pub fn pr_with_mergeable(mergeable: Mergeable) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(1).expect("number"),
        "example",
        false,
        PullRequestAuthor::DeletedAccount,
        mergeable,
        Vec::new(),
    )
}

/// Build an observation whose `draft` flag is the given value and whose threads/
/// mergeability are the slice 0017 don't-care defaults.
pub fn pr_with_draft(draft: bool) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(1).expect("number"),
        "example",
        draft,
        PullRequestAuthor::DeletedAccount,
        Mergeable::Unknown,
        Vec::new(),
    )
}

/// A validated author login value, for configuring reviewer lists in the slice 0017 tests.
pub fn login(value: &str) -> AuthorLogin {
    AuthorLogin::new(value).expect("login")
}

pub fn locator() -> PullRequestLocator {
    PullRequestLocator {
        owner: "alice".to_owned(),
        repo: "project".to_owned(),
        number: 1,
    }
}

/// What the live-merge read should return on the next call. `NoConflict` is the default
/// so slice-0014 tests that never touch the merge path are unaffected.
#[derive(Debug, Clone, Copy)]
pub enum LiveMerge {
    /// The live provider has settled on "no conflict present" (clean/unstable/blocked/…).
    NoConflict,
    /// The live provider still reports a conflict (`dirty`).
    StillConflicting,
    /// The live provider has not yet settled; the rule must surface this as a Failed
    /// reaction so the park-and-retry path drives another attempt.
    Computing,
}

/// Fake provider client: in-memory comments, labels and review-request list, records every
/// post, label mutation and review-request call, a swappable live-merge response for
/// slice 0015/0016 tests, and an in-memory draft-history value for slice 0017. Doubles as a
/// [`MergeableHistory`] and [`DraftHistory`] for history-walk tests.
#[derive(Debug)]
pub struct FakeClient {
    existing: Mutex<Vec<ExistingComment>>,
    posted: Mutex<Vec<String>>,
    live: Mutex<LiveMerge>,
    history: Mutex<Option<Mergeable>>,
    history_err: Mutex<Option<String>>,
    labels: Mutex<BTreeSet<String>>,
    labels_err: Mutex<Option<String>>,
    added_labels: Mutex<Vec<String>>,
    removed_labels: Mutex<Vec<String>>,
    requested_reviewers: Mutex<Vec<String>>,
    requested_reviewers_err: Mutex<Option<String>>,
    request_reviewers_err: Mutex<Option<String>>,
    request_reviewers_calls: Mutex<Vec<Vec<String>>>,
    draft_history: Mutex<Option<bool>>,
    draft_history_err: Mutex<Option<String>>,
}

impl Default for FakeClient {
    fn default() -> Self {
        Self {
            existing: Mutex::new(Vec::new()),
            posted: Mutex::new(Vec::new()),
            live: Mutex::new(LiveMerge::NoConflict),
            history: Mutex::new(None),
            history_err: Mutex::new(None),
            labels: Mutex::new(BTreeSet::new()),
            labels_err: Mutex::new(None),
            added_labels: Mutex::new(Vec::new()),
            removed_labels: Mutex::new(Vec::new()),
            requested_reviewers: Mutex::new(Vec::new()),
            requested_reviewers_err: Mutex::new(None),
            request_reviewers_err: Mutex::new(None),
            request_reviewers_calls: Mutex::new(Vec::new()),
            draft_history: Mutex::new(None),
            draft_history_err: Mutex::new(None),
        }
    }
}

impl FakeClient {
    pub fn with_existing(bodies: impl IntoIterator<Item = String>) -> Self {
        let existing = bodies
            .into_iter()
            .map(|body| ExistingComment { body })
            .collect();
        Self {
            existing: Mutex::new(existing),
            ..Self::default()
        }
    }

    pub fn posts(&self) -> Vec<String> {
        self.posted.lock().expect("posted").clone()
    }

    /// Flip the live-merge response the fake returns on the next calls.
    pub fn set_live(&self, state: LiveMerge) {
        *self.live.lock().expect("live") = state;
    }

    /// Configure what the fake's `MergeableHistory::latest_settled` returns.
    pub fn set_history(&self, found: Option<Mergeable>) {
        *self.history.lock().expect("history") = found;
    }

    /// Force the history query to error on the next call.
    pub fn set_history_err(&self, message: impl Into<String>) {
        *self.history_err.lock().expect("history_err") = Some(message.into());
    }

    /// Seed the labels currently attached to the pull request.
    pub fn set_labels(&self, labels: impl IntoIterator<Item = &'static str>) {
        let mut guard = self.labels.lock().expect("labels");
        guard.clear();
        for l in labels {
            guard.insert(l.to_owned());
        }
    }

    /// Force the label list to error on the next call.
    pub fn set_labels_err(&self, message: impl Into<String>) {
        *self.labels_err.lock().expect("labels_err") = Some(message.into());
    }

    /// Snapshot of the labels currently attached.
    pub fn labels_snapshot(&self) -> BTreeSet<String> {
        self.labels.lock().expect("labels").clone()
    }

    /// Everything `add_label` was called with, in order.
    pub fn added_labels(&self) -> Vec<String> {
        self.added_labels.lock().expect("added").clone()
    }

    /// Everything `remove_label` was called with, in order.
    pub fn removed_labels(&self) -> Vec<String> {
        self.removed_labels.lock().expect("removed").clone()
    }

    /// Seed the reviewers currently on the review-request list.
    pub fn set_requested_reviewers(&self, reviewers: impl IntoIterator<Item = &'static str>) {
        let mut guard = self
            .requested_reviewers
            .lock()
            .expect("requested_reviewers");
        guard.clear();
        for r in reviewers {
            guard.push(r.to_owned());
        }
    }

    /// Force the `list_requested_reviewers` call to error on the next call.
    pub fn set_requested_reviewers_err(&self, message: impl Into<String>) {
        *self
            .requested_reviewers_err
            .lock()
            .expect("requested_reviewers_err") = Some(message.into());
    }

    /// Force the `request_reviewers` call to error on the next call.
    pub fn set_request_reviewers_err(&self, message: impl Into<String>) {
        *self
            .request_reviewers_err
            .lock()
            .expect("request_reviewers_err") = Some(message.into());
    }

    /// Everything `request_reviewers` was called with, in order (each call's full argument
    /// list, so a test can assert the exact subset the rule computed).
    pub fn request_reviewers_calls(&self) -> Vec<Vec<String>> {
        self.request_reviewers_calls
            .lock()
            .expect("request_reviewers_calls")
            .clone()
    }

    /// Snapshot of who is currently on the review-request list (after applied writes).
    pub fn requested_reviewers_snapshot(&self) -> Vec<String> {
        self.requested_reviewers
            .lock()
            .expect("requested_reviewers")
            .clone()
    }

    /// Configure what the fake's [`DraftHistory::last_draft`] returns.
    pub fn set_draft_history(&self, found: Option<bool>) {
        *self.draft_history.lock().expect("draft_history") = found;
    }

    /// Force the draft-history query to error on the next call.
    pub fn set_draft_history_err(&self, message: impl Into<String>) {
        *self.draft_history_err.lock().expect("draft_history_err") = Some(message.into());
    }
}

impl MergeableHistory for FakeClient {
    fn latest_settled(&self, _locator: &PullRequestLocator) -> Result<Option<Mergeable>, String> {
        if let Some(msg) = self.history_err.lock().expect("history_err").clone() {
            return Err(msg);
        }
        Ok(*self.history.lock().expect("history"))
    }
}

impl DraftHistory for FakeClient {
    fn last_draft(&self, _locator: &PullRequestLocator) -> Result<Option<bool>, String> {
        if let Some(msg) = self
            .draft_history_err
            .lock()
            .expect("draft_history_err")
            .clone()
        {
            return Err(msg);
        }
        Ok(*self.draft_history.lock().expect("draft_history"))
    }
}

impl ProviderClient for FakeClient {
    fn list_comments<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<ExistingComment>> {
        let existing = self.existing.lock().expect("existing").clone();
        Box::pin(async move { Ok(existing) })
    }

    fn post_comment<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        body: &'a str,
    ) -> ClientFuture<'a, ()> {
        let body = body.to_owned();
        Box::pin(async move {
            self.existing
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned existing lock: {e}")))?
                .push(ExistingComment { body: body.clone() });
            self.posted
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned posted lock: {e}")))?
                .push(body);
            Ok(())
        })
    }

    fn live_merge_has_conflict<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool> {
        let live = *self.live.lock().expect("live");
        Box::pin(async move {
            match live {
                LiveMerge::NoConflict => Ok(false),
                LiveMerge::StillConflicting => Ok(true),
                LiveMerge::Computing => Err(ProviderError::Client(
                    "live merge state still computing".to_owned(),
                )),
            }
        })
    }

    fn list_labels<'a>(&'a self, _locator: &'a PullRequestLocator) -> ClientFuture<'a, Vec<Label>> {
        if let Some(msg) = self.labels_err.lock().expect("labels_err").clone() {
            return Box::pin(async move { Err(ProviderError::Client(msg)) });
        }
        let snapshot = self.labels.lock().expect("labels").clone();
        Box::pin(async move {
            Ok(snapshot
                .into_iter()
                .filter_map(|s| Label::new(s).ok())
                .collect())
        })
    }

    fn add_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        let name = label.as_str().to_owned();
        Box::pin(async move {
            self.added_labels
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned added lock: {e}")))?
                .push(name.clone());
            self.labels
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned labels lock: {e}")))?
                .insert(name);
            Ok(())
        })
    }

    fn remove_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        let name = label.as_str().to_owned();
        Box::pin(async move {
            self.removed_labels
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned removed lock: {e}")))?
                .push(name.clone());
            self.labels
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned labels lock: {e}")))?
                .remove(&name);
            Ok(())
        })
    }

    fn list_requested_reviewers<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<AuthorLogin>> {
        if let Some(msg) = self
            .requested_reviewers_err
            .lock()
            .expect("requested_reviewers_err")
            .clone()
        {
            return Box::pin(async move { Err(ProviderError::Client(msg)) });
        }
        let snapshot = self
            .requested_reviewers
            .lock()
            .expect("requested_reviewers")
            .clone();
        Box::pin(async move {
            let mut out = Vec::with_capacity(snapshot.len());
            for name in snapshot {
                out.push(AuthorLogin::new(name).map_err(|_| {
                    ProviderError::Client(
                        "fake returned a login that does not pass AuthorLogin::new".to_owned(),
                    )
                })?);
            }
            Ok(out)
        })
    }

    fn request_reviewers<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        reviewers: &'a [AuthorLogin],
    ) -> ClientFuture<'a, ()> {
        let names: Vec<String> = reviewers.iter().map(|r| r.as_str().to_owned()).collect();
        Box::pin(async move {
            if let Some(msg) = self
                .request_reviewers_err
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned request_reviewers_err: {e}")))?
                .take()
            {
                return Err(ProviderError::Client(msg));
            }
            self.request_reviewers_calls
                .lock()
                .map_err(|e| {
                    ProviderError::Client(format!("poisoned request_reviewers_calls: {e}"))
                })?
                .push(names.clone());
            let mut guard = self
                .requested_reviewers
                .lock()
                .map_err(|e| ProviderError::Client(format!("poisoned requested_reviewers: {e}")))?;
            for name in names {
                if !guard.iter().any(|r| r == &name) {
                    guard.push(name);
                }
            }
            Ok(())
        })
    }
}
