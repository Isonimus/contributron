//! Scenario 6 of slice 0009: the CLI renders current state from the store, and the bytes
//! equal what a capture-based render produced for the same observation.
//!
//! The store is not permitted to become a second encoding of the same fact. If this ever
//! diverges, one of the two render paths (store `list` or capture `render`) is wrong.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::actor::{Attribution, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_github::normalize_capture;
use contributron_store::{EventStore, WriteOutcome};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn recorded_capture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("github")
        .join("tests")
        .join("fixtures")
        .join("pull_request_with_threads.json")
}

#[test]
fn render_from_the_store_matches_the_capture_based_render() {
    let capture_path = recorded_capture();
    let capture_render = Command::new(env!("CARGO_BIN_EXE_contributron"))
        .arg("render")
        .arg("--capture")
        .arg(&capture_path)
        .output()
        .expect("the binary runs");
    assert!(
        capture_render.status.success(),
        "capture render failed: {}",
        String::from_utf8_lossy(&capture_render.stderr),
    );

    // Build the same observation the CLI would and persist it, under the same identity the
    // CLI uses on this capture ("local-operator").
    let raw = std::fs::read_to_string(&capture_path).expect("read fixture");
    let pull_request = normalize_capture(&raw).expect("normalize");
    let attribution = Attribution::local_operator(
        UserId::new("local-operator").expect("static test id"),
        WorkerId::new("cli").expect("static test id"),
    );
    let observation = PullRequestObservation::new(attribution, pull_request);

    let tmp = TempDir::new().expect("temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let store = EventStore::open(
        &store_path,
        UserId::new("local-operator").expect("static test id"),
    )
    .expect("open");
    assert_eq!(
        store.write(&observation).expect("write"),
        WriteOutcome::Fresh
    );
    drop(store);

    let store_render = Command::new(env!("CARGO_BIN_EXE_contributron"))
        .arg("list")
        .arg("--store")
        .arg(&store_path)
        .output()
        .expect("the binary runs");
    assert!(
        store_render.status.success(),
        "store render failed: {}",
        String::from_utf8_lossy(&store_render.stderr),
    );

    assert_eq!(
        store_render.stdout, capture_render.stdout,
        "store render differs from capture render",
    );
}
