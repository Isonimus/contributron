//! Shared fixtures for the react crate's integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use contributron_domain::pull_request::{
    ObservedPullRequest, PullRequestAuthor, PullRequestNumber, ReviewThread, ThreadId,
    ThreadResolution,
};
use contributron_poll::PullRequestLocator;
use contributron_react::{ClientFuture, ExistingComment, ProviderClient, ProviderError};
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
        threads,
    )
}

pub fn locator() -> PullRequestLocator {
    PullRequestLocator {
        owner: "alice".to_owned(),
        repo: "project".to_owned(),
        number: 1,
    }
}

/// Fake provider client: in-memory comments, records every post.
#[derive(Debug, Default)]
pub struct FakeClient {
    existing: Mutex<Vec<ExistingComment>>,
    posted: Mutex<Vec<String>>,
}

impl FakeClient {
    pub fn with_existing(bodies: impl IntoIterator<Item = String>) -> Self {
        let existing = bodies.into_iter().map(|body| ExistingComment { body }).collect();
        Self {
            existing: Mutex::new(existing),
            posted: Mutex::new(Vec::new()),
        }
    }

    pub fn posts(&self) -> Vec<String> {
        self.posted.lock().expect("posted").clone()
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
}
