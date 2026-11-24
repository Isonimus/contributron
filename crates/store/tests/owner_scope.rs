//! Scenario 5 of slice 0009: a row carrying a different owner is rejected at write.
//!
//! This is standing invariant 7 moving from review-only to a wired check. The check is on
//! the submitted row, not on a wrapper struct a caller could choose to bypass: the test
//! builds an observation whose attribution names a different user and asserts the write
//! fails and no row was persisted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_store::{EventStore, StoreError, WriteOutcome};
use tempfile::TempDir;

#[test]
fn a_row_whose_owner_mismatches_the_store_is_rejected_and_nothing_is_persisted() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("the-store-owner"))
        .expect("store opens");

    let misowned = observation("a-different-user", "someone", 1, "a title");

    let outcome = store.write(&misowned);
    match outcome {
        Err(StoreError::OwnerMismatch { expected, actual }) => {
            assert_eq!(expected, "the-store-owner");
            assert_eq!(actual, "a-different-user");
        }
        other => panic!("expected OwnerMismatch, got: {other:?}"),
    }

    assert!(
        store.list().expect("list").is_empty(),
        "no row may be persisted when the owner mismatches",
    );
}

/// Two owners sharing a store file must dedup per owner, not globally.
///
/// Reproduces finding 1 of the 2026-11-24 adversarial review: event identity excludes
/// attribution (ADR-0008), so two owners observing the same pull request state compute the
/// same `event_id`. With a single-column primary key, the second owner's write would collide
/// with the first owner's row, be mapped to `WriteOutcome::Duplicate`, and the second owner's
/// `read`/`list` (scoped by `owner_user`) would return nothing — a silent write loss across
/// the owner boundary. The dedup must be `(owner_user, event_id)`, not `event_id` alone.
#[test]
fn two_owners_sharing_a_file_dedup_per_owner() {
    let tmp = TempDir::new().expect("a temporary directory");
    let path = tmp.path().join("events.sqlite3");

    let alice_view = observation("alice", "someone", 1, "shared title");
    let bob_view = observation("bob", "someone", 1, "shared title");

    assert_eq!(
        alice_view.event_id().expect("alice id"),
        bob_view.event_id().expect("bob id"),
        "premise: same content, same identity (ADR-0008)",
    );

    let alice_store = EventStore::open(&path, user("alice")).expect("open for alice");
    assert_eq!(
        alice_store.write(&alice_view).expect("alice writes"),
        WriteOutcome::Fresh,
    );
    drop(alice_store);

    let bob_store = EventStore::open(&path, user("bob")).expect("open for bob");
    assert_eq!(
        bob_store.write(&bob_view).expect("bob writes"),
        WriteOutcome::Fresh,
        "bob's write must land, not collide with alice's content-equal row",
    );

    let bob_sees = bob_store.list().expect("bob lists");
    assert_eq!(
        bob_sees.len(),
        1,
        "bob must see his own row, saw: {bob_sees:?}"
    );
    assert_eq!(bob_sees[0], bob_view);

    let alice_store = EventStore::open(&path, user("alice")).expect("reopen for alice");
    let alice_sees = alice_store.list().expect("alice lists");
    assert_eq!(
        alice_sees.len(),
        1,
        "alice must still see her own row, saw: {alice_sees:?}",
    );
    assert_eq!(alice_sees[0], alice_view);
}
