//! `EventStore::latest_for_number` returns the most recent observation for a PR number in
//! owner scope, and returns `None` when nothing matches. Added for finding 2 of the
//! 2026-11-24 adversarial review of slice 0014: the polling task seeds its reactor prior
//! from this query at startup.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_store::EventStore;
use tempfile::TempDir;

#[test]
fn returns_none_when_the_store_has_no_row_for_that_number() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner"))
        .expect("the store opens");

    let obs = observation("owner", "alice", 7, "title");
    store.write(&obs).expect("write");

    let miss = store
        .latest_for_number(42)
        .expect("query runs without error");
    assert!(miss.is_none(), "no row for #42, got {miss:?}");
}

#[test]
fn returns_the_most_recent_row_for_the_number_in_owner_scope() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner"))
        .expect("the store opens");

    // Two distinct observations for #7: different titles make different content digests,
    // so both land. The second write happens later, so its observed_at is larger.
    let first = observation("owner", "alice", 7, "first");
    let second = observation("owner", "alice", 7, "second");
    // A row for a different number must not be picked up.
    let other = observation("owner", "alice", 99, "unrelated");

    store.write(&first).expect("first write");
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.write(&second).expect("second write");
    store.write(&other).expect("other write");

    let latest = store
        .latest_for_number(7)
        .expect("query")
        .expect("a row for #7");
    assert_eq!(latest.title, "second");
}
