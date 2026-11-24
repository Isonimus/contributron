//! Shared helpers for the polling loop's integration tests.

#![allow(dead_code)]

use contributron_domain::actor::UserId;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
};
use contributron_poll::{Fetch, FetchError, FetchFuture, PullRequestLocator};
use std::sync::Arc;
use std::sync::Mutex;

pub(crate) fn user(name: &str) -> UserId {
    UserId::new(name)
}

pub(crate) fn locator(owner: &str, repo: &str, number: u64) -> PullRequestLocator {
    PullRequestLocator {
        owner: owner.into(),
        repo: repo.into(),
        number,
    }
}

pub(crate) fn observation(number: u64, title: &str) -> ObservedPullRequest {
    observation_with_mergeable(number, title, Mergeable::Unknown)
}

pub(crate) fn observation_with_mergeable(
    number: u64,
    title: &str,
    mergeable: Mergeable,
) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(number).expect("a nonzero pr number"),
        title,
        false,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("alice").expect("a nonempty login"),
        },
        mergeable,
        Vec::new(),
    )
}

/// A stub [`Fetch`] that returns a fixed observation and records how many calls it saw.
pub(crate) fn constant_fetch(observation: ObservedPullRequest) -> (Fetch, Arc<Mutex<u32>>) {
    let calls = Arc::new(Mutex::new(0u32));
    let calls_for_closure = Arc::clone(&calls);
    let fetch: Fetch = Arc::new(move |_locator| {
        let o = observation.clone();
        let calls = Arc::clone(&calls_for_closure);
        Box::pin(async move {
            let mut guard = calls.lock().expect("stub-fetch call counter not poisoned");
            *guard = guard.saturating_add(1);
            Ok(o)
        }) as FetchFuture
    });
    (fetch, calls)
}

/// A [`Fetch`] driven by a sequence of outcomes: the Nth call returns the Nth entry. Each
/// entry is either an observation or a fetch error. When the sequence is exhausted, the
/// last entry is repeated so a loop that outruns the script does not see a surprise.
pub(crate) fn scripted_fetch(script: Vec<Result<ObservedPullRequest, &'static str>>) -> Fetch {
    let script = Arc::new(Mutex::new(script));
    Arc::new(move |_locator| {
        let script = Arc::clone(&script);
        Box::pin(async move {
            let mut guard = script.lock().expect("scripted-fetch mutex not poisoned");
            // Pull from the front; keep the last entry around so later calls see it too.
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
