//! Scenario 4 of slice 0009: a store file survives the writing process closing.
//!
//! Two store handles over the same file within one test. The first writes and is dropped,
//! which closes the SQLite connection; the second opens the file fresh and must see the
//! same rows. That is the strongest proof possible without spawning a subprocess, and it
//! fails exactly where a process-death scenario would: an in-memory-only store, or a schema
//! that does not synchronise to disk.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_store::{EventStore, WriteOutcome};
use tempfile::TempDir;

#[test]
fn observations_survive_dropping_and_reopening_the_store() {
    let tmp = TempDir::new().expect("a temporary directory");
    let path = tmp.path().join("events.sqlite3");

    let one = observation("owner", "someone", 1, "first");
    let two = observation("owner", "someone", 2, "second");

    {
        let writer = EventStore::open(&path, user("owner")).expect("open for write");
        assert_eq!(writer.write(&one).expect("write one"), WriteOutcome::Fresh);
        assert_eq!(writer.write(&two).expect("write two"), WriteOutcome::Fresh);
    }

    let reader = EventStore::open(&path, user("owner")).expect("reopen");

    let one_back = reader
        .read(&one.event_id().expect("id"))
        .expect("read one")
        .expect("one present");
    let two_back = reader
        .read(&two.event_id().expect("id"))
        .expect("read two")
        .expect("two present");

    assert_eq!(one_back, one, "first observation survived");
    assert_eq!(two_back, two, "second observation survived");
}
