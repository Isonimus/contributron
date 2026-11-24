//! Shared fixtures for the react crate's integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use contributron_domain::pull_request::{
    Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber, ReviewThread, ThreadId,
    ThreadResolution,
};
use contributron_poll::PullRequestLocator;
use contributron_react::{
    ClientFuture, ExistingComment, MergeableHistory, ProviderClient, ProviderError,
};
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

/// Fake provider client: in-memory comments, records every post, and a swappable
/// live-merge response for slice 0015 tests. Also carries the fake's stored "history
/// settled mergeable" value so [`FakeClient`] can double as a [`MergeableHistory`] in the
/// slice-0015 tests.
#[derive(Debug)]
pub struct FakeClient {
    existing: Mutex<Vec<ExistingComment>>,
    posted: Mutex<Vec<String>>,
    live: Mutex<LiveMerge>,
    history: Mutex<Option<Mergeable>>,
    history_err: Mutex<Option<String>>,
}

impl Default for FakeClient {
    fn default() -> Self {
        Self {
            existing: Mutex::new(Vec::new()),
            posted: Mutex::new(Vec::new()),
            live: Mutex::new(LiveMerge::NoConflict),
            history: Mutex::new(None),
            history_err: Mutex::new(None),
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
            posted: Mutex::new(Vec::new()),
            live: Mutex::new(LiveMerge::NoConflict),
            history: Mutex::new(None),
            history_err: Mutex::new(None),
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
}

impl MergeableHistory for FakeClient {
    fn latest_settled(&self, _locator: &PullRequestLocator) -> Result<Option<Mergeable>, String> {
        if let Some(msg) = self.history_err.lock().expect("history_err").clone() {
            return Err(msg);
        }
        Ok(*self.history.lock().expect("history"))
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
            // A real provider would also list it afterwards; append so a second reactor call
            // in the same test sees the post as existing.
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
}
