//! Scenario 4 of slice 0014: the marker scan is what makes the rule idempotent across
//! process restarts and store wipes, without adding a schema column.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty
)]

mod common;

use common::{FakeClient, locator, pr};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_react::{RULE_MARKER, ReactionOutcome, react};

#[tokio::test]
async fn an_existing_marker_comment_prevents_a_second_post() {
    // Seed the fake provider with a comment that already carries this rule's marker, as if
    // a previous reactor run had posted it and the process restarted. The pair we pass
    // *would* otherwise qualify.
    let existing_body = format!("{RULE_MARKER}\nFrom the previous reactor run.");
    let client = FakeClient::with_existing([existing_body]);
    let owner = UserId::new("alice").expect("static test id");
    let worker = WorkerId::new("poll").expect("static test id");
    let loc = locator();

    let outcome = react(&owner, &worker, &loc, Some(&pr(1, 0)), &pr(0, 1), &client)
        .await
        .expect("react");

    assert_eq!(outcome, ReactionOutcome::AlreadySettled);
    assert!(
        client.posts().is_empty(),
        "no new post, got: {:?}",
        client.posts()
    );
}

#[tokio::test]
async fn a_second_react_call_in_the_same_process_does_not_double_post() {
    // The fake records a post into its own `existing` list, so a repeated call sees it.
    let client = FakeClient::default();
    let owner = UserId::new("alice").expect("static test id");
    let worker = WorkerId::new("poll").expect("static test id");
    let loc = locator();

    react(&owner, &worker, &loc, Some(&pr(2, 0)), &pr(0, 2), &client)
        .await
        .expect("first react");

    let outcome = react(&owner, &worker, &loc, Some(&pr(2, 0)), &pr(0, 2), &client)
        .await
        .expect("second react");

    assert_eq!(outcome, ReactionOutcome::AlreadySettled);
    assert_eq!(
        client.posts().len(),
        1,
        "still one post, got: {:?}",
        client.posts()
    );
}
