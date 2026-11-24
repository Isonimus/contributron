//! Scenario 1 of slice 0009: a written observation reads back byte for byte, including its
//! attribution.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{observation, user};
use contributron_store::{EventStore, WriteOutcome};
use tempfile::TempDir;

#[test]
fn round_trip_returns_the_original_observation_including_attribution() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner"))
        .expect("the store opens");

    let obs = observation("owner", "example/repo", 42, "first title");
    assert_eq!(store.write(&obs).expect("write"), WriteOutcome::Fresh);

    let id = obs.event_id().expect("identity");
    let read_back = store.read(&id).expect("read").expect("row present");

    assert_eq!(read_back, obs);
}
