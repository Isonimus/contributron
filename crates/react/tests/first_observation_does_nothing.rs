//! Scenario 3 of slice 0014: the first observation of a pull request, even with zero
//! unresolved threads, is not a transition: there is no prior state to compare against.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty
)]

mod common;

use common::{FakeClient, locator, pr};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_react::{ReactionOutcome, react};

#[tokio::test]
async fn first_observation_with_zero_unresolved_does_not_post() {
    let client = FakeClient::default();
    let owner = UserId::new("alice");
    let worker = WorkerId::new("poll");
    let loc = locator();

    let outcome = react(&owner, &worker, &loc, None, &pr(0, 0), &client)
        .await
        .expect("react");

    assert_eq!(outcome, ReactionOutcome::NotApplicable);
    assert!(client.posts().is_empty());
}
