//! Scenario 1 of slice 0014: a qualifying transition posts exactly one marker-bearing comment.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{FakeClient, locator, pr};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_react::{RULE_MARKER, ReactionOutcome, react};

#[tokio::test]
async fn transition_from_two_unresolved_to_zero_posts_once_with_marker() {
    let client = FakeClient::default();
    let owner = UserId::new("alice").expect("static test id");
    let worker = WorkerId::new("poll").expect("static test id");
    let loc = locator();
    let prior = pr(2, 0);
    let new = pr(0, 2);

    let outcome = react(&owner, &worker, &loc, Some(&prior), &new, &client)
        .await
        .expect("react");

    assert!(matches!(outcome, ReactionOutcome::Posted { .. }));
    let posts = client.posts();
    assert_eq!(posts.len(), 1, "exactly one post, got: {posts:?}");
    assert!(
        posts[0].contains(RULE_MARKER),
        "post lacked marker: {}",
        posts[0]
    );
}
