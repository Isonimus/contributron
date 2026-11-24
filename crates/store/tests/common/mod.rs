//! Fixture builders shared by the store's integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::actor::{Attribution, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
    ReviewThread, ThreadId, ThreadResolution,
};

pub(crate) fn user(name: &str) -> UserId {
    UserId::new(name)
}

pub(crate) fn observation(
    owner: &str,
    author: &str,
    number: u64,
    title: &str,
) -> PullRequestObservation {
    PullRequestObservation::new(
        Attribution::local_operator(UserId::new(owner), WorkerId::new("cli")),
        ObservedPullRequest::new(
            PullRequestNumber::new(number).expect("non-zero pull request number"),
            title,
            false,
            PullRequestAuthor::Account {
                login: AuthorLogin::new(author).expect("a non-empty login"),
            },
            Mergeable::Unknown,
            vec![ReviewThread {
                id: ThreadId::new("T1").expect("a non-empty identifier"),
                resolution: ThreadResolution::Unresolved,
            }],
        ),
    )
}
