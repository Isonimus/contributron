//! Scenarios 2 and 3 of slice 0009: re-observing the same state is a no-op; a changed
//! observation writes a new row, keyed by its new identity.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_store::{EventStore, WriteOutcome};
use tempfile::TempDir;

#[test]
fn a_second_write_of_the_same_observation_is_a_duplicate_and_retains_the_first_attribution() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store =
        EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("store opens");

    // Build the original (no agent) and a second observation that shares the observed
    // content but names a different worker and an agent. The attribution differs, the
    // observed content does not, so ADR-0008 requires the identity to be equal.
    let original = observation("owner", "someone", 1, "same title");
    let second_poll = PullRequestObservation::new(
        Attribution {
            user: UserId::new("owner"),
            worker: WorkerId::new("a-different-worker"),
            agent: ExecutingAgent::Agent {
                id: AgentId::new("claude-code"),
            },
        },
        original.pull_request.clone(),
    );
    assert_eq!(
        original.event_id().expect("original id"),
        second_poll.event_id().expect("second id"),
        "premise: attribution does not change identity (ADR-0008)",
    );
    assert_ne!(
        original.attribution, second_poll.attribution,
        "premise: the two writers differ, so we can see which one won",
    );

    assert_eq!(
        store.write(&original).expect("first write"),
        WriteOutcome::Fresh,
    );
    assert_eq!(
        store.write(&second_poll).expect("second write"),
        WriteOutcome::Duplicate,
    );

    let retained = store
        .read(&original.event_id().expect("id"))
        .expect("read")
        .expect("row present");
    assert_eq!(
        retained.attribution, original.attribution,
        "first-writer-wins: scenario 2",
    );
}

#[test]
fn an_observation_whose_content_differs_writes_a_second_row() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store =
        EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("store opens");

    let first = observation("owner", "someone", 1, "the original title");
    let different = observation("owner", "someone", 1, "a changed title");

    assert_ne!(
        first.event_id().expect("first id"),
        different.event_id().expect("different id"),
        "scenario 3 premise: different content, different identity",
    );

    assert_eq!(store.write(&first).expect("write one"), WriteOutcome::Fresh);
    assert_eq!(
        store.write(&different).expect("write two"),
        WriteOutcome::Fresh,
    );

    let both = store.list().expect("list");
    assert_eq!(both.len(), 2, "two rows expected, saw: {both:?}");
}
