//! Scenario 5 of slice 0009: a row carrying a different owner is rejected at write.
//!
//! This is standing invariant 7 moving from review-only to a wired check. The check is on
//! the submitted row, not on a wrapper struct a caller could choose to bypass: the test
//! builds an observation whose attribution names a different user and asserts the write
//! fails and no row was persisted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_store::{EventStore, StoreError};
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
