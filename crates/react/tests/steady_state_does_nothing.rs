//! Scenario 2 of slice 0014: a 0 to 0 observation posts nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{FakeClient, locator, pr};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_react::{ReactionOutcome, react};

#[tokio::test]
async fn steady_state_zero_does_not_post() {
    let client = FakeClient::default();
    let owner = UserId::new("alice").expect("static test id");
    let worker = WorkerId::new("poll").expect("static test id");
    let loc = locator();

    let outcome = react(&owner, &worker, &loc, Some(&pr(0, 1)), &pr(0, 1), &client)
        .await
        .expect("react");

    assert_eq!(outcome, ReactionOutcome::NotApplicable);
    assert!(
        client.posts().is_empty(),
        "no post, got: {:?}",
        client.posts()
    );
}
