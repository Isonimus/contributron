//! Slice 0019: reaction records persist the three-identity tuple, the pull-request
//! locator, and the rule's outcome. One row per rule fire; identity is derived from
//! `(event_id, agent_id)` so a retry is a duplicate, not a new row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty
)]

mod common;

use common::{observation, user};
use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_store::{
    EventStore, ReactionRecord, ReactionRecordOutcome, StoreError, WriteOutcome,
};
use rusqlite::Connection;
use tempfile::TempDir;

fn record(
    owner: &str,
    agent_id: &str,
    outcome: ReactionRecordOutcome,
    pr_number: u64,
    event_id: contributron_domain::event::EventId,
) -> ReactionRecord {
    ReactionRecord {
        attribution: Attribution {
            user: UserId::new(owner).expect("test user"),
            worker: WorkerId::new("cli").expect("static test id"),
            agent: ExecutingAgent::Agent {
                id: AgentId::new(agent_id).expect("test agent"),
            },
        },
        pr_owner: "octo-org".to_owned(),
        pr_repo: "project".to_owned(),
        pr_number,
        outcome,
        event_id,
    }
}

#[test]
fn write_then_list_round_trips_every_column() {
    let tmp = TempDir::new().expect("tmp");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("open");

    let obs = observation("owner", "alice", 7, "hello");
    let event_id = obs.event_id().expect("event id");

    let row = record(
        "owner",
        "rule:threads-cleared",
        ReactionRecordOutcome::Posted,
        7,
        event_id,
    );
    assert_eq!(
        store.write_reaction(&row).expect("write"),
        WriteOutcome::Fresh,
    );

    let rows = store.list_reactions().expect("list");
    assert_eq!(rows.len(), 1);
    let stored = &rows[0];
    assert_eq!(stored.attribution.user.as_str(), "owner");
    assert_eq!(stored.attribution.worker.as_str(), "cli");
    match &stored.attribution.agent {
        ExecutingAgent::Agent { id } => assert_eq!(id.as_str(), "rule:threads-cleared"),
        ExecutingAgent::NoAgent => panic!("expected an Agent attribution"),
    }
    assert_eq!(stored.pr_owner, "octo-org");
    assert_eq!(stored.pr_repo, "project");
    assert_eq!(stored.pr_number, 7);
    assert_eq!(stored.outcome, ReactionRecordOutcome::Posted);
    assert!(!stored.observed_at.is_empty());
    assert!(!stored.reaction_id.is_empty());
}

#[test]
fn rewriting_the_same_reaction_id_is_a_duplicate_not_a_new_row() {
    let tmp = TempDir::new().expect("tmp");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("open");

    let obs = observation("owner", "alice", 7, "hello");
    let event_id = obs.event_id().expect("event id");
    let row = record(
        "owner",
        "rule:threads-cleared",
        ReactionRecordOutcome::Posted,
        7,
        event_id.clone(),
    );

    assert_eq!(
        store.write_reaction(&row).expect("write 1"),
        WriteOutcome::Fresh,
    );
    // Same event_id + agent_id → same reaction_id. The retry is idempotent: the first row
    // stands, no second row is created.
    let retry = record(
        "owner",
        "rule:threads-cleared",
        ReactionRecordOutcome::AlreadySettled,
        7,
        event_id,
    );
    assert_eq!(
        store.write_reaction(&retry).expect("write 2"),
        WriteOutcome::Duplicate,
    );

    assert_eq!(store.list_reactions().expect("list").len(), 1);
}

#[test]
fn two_rules_on_the_same_event_each_get_their_own_row() {
    let tmp = TempDir::new().expect("tmp");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("open");

    let obs = observation("owner", "alice", 7, "hello");
    let event_id = obs.event_id().expect("event id");

    let r1 = record(
        "owner",
        "rule:threads-cleared",
        ReactionRecordOutcome::Posted,
        7,
        event_id.clone(),
    );
    let r2 = record(
        "owner",
        "rule:needs-rebase-label",
        ReactionRecordOutcome::AlreadySettled,
        7,
        event_id,
    );
    assert_eq!(
        store.write_reaction(&r1).expect("write 1"),
        WriteOutcome::Fresh,
    );
    assert_eq!(
        store.write_reaction(&r2).expect("write 2"),
        WriteOutcome::Fresh,
    );
    assert_eq!(store.list_reactions().expect("list").len(), 2);
}

#[test]
fn an_owner_mismatch_refuses_the_write_at_the_store_boundary() {
    let tmp = TempDir::new().expect("tmp");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("alice")).expect("open");

    let obs = observation("alice", "alice", 7, "hello");
    let event_id = obs.event_id().expect("event id");
    // The row names a different owner than the store.
    let row = record(
        "bob",
        "rule:threads-cleared",
        ReactionRecordOutcome::Posted,
        7,
        event_id,
    );
    match store.write_reaction(&row) {
        Err(StoreError::OwnerMismatch { expected, actual }) => {
            assert_eq!(expected, "alice");
            assert_eq!(actual, "bob");
        }
        other => panic!("expected OwnerMismatch, got {other:?}"),
    }
    assert!(store.list_reactions().expect("list").is_empty());
}

#[test]
fn a_reaction_with_no_agent_id_is_refused() {
    let tmp = TempDir::new().expect("tmp");
    let store = EventStore::open(&tmp.path().join("events.sqlite3"), user("owner")).expect("open");

    let obs = observation("owner", "alice", 7, "hello");
    let event_id = obs.event_id().expect("event id");
    let row = ReactionRecord {
        attribution: Attribution {
            user: UserId::new("owner").expect("static test id"),
            worker: WorkerId::new("cli").expect("static test id"),
            agent: ExecutingAgent::NoAgent,
        },
        pr_owner: "octo-org".to_owned(),
        pr_repo: "project".to_owned(),
        pr_number: 7,
        outcome: ReactionRecordOutcome::Posted,
        event_id,
    };
    match store.write_reaction(&row) {
        Err(StoreError::MissingAgentId) => {}
        other => panic!("expected MissingAgentId, got {other:?}"),
    }
    assert!(store.list_reactions().expect("list").is_empty());
}

#[test]
fn opening_a_v2_file_migrates_to_v3_without_touching_events() {
    let tmp = TempDir::new().expect("tmp");
    let path = tmp.path().join("events.sqlite3");

    // Build a v2 file by hand: schema at version 2, one event row.
    {
        let c = Connection::open(&path).expect("open raw");
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                event_id    TEXT NOT NULL,
                owner_user  TEXT NOT NULL,
                worker      TEXT NOT NULL,
                agent       TEXT NOT NULL,
                observed_at TEXT NOT NULL,
                payload     TEXT NOT NULL,
                PRIMARY KEY (owner_user, event_id)
            );
            PRAGMA user_version = 2;
            INSERT INTO events (event_id, owner_user, worker, agent, observed_at, payload)
            VALUES ('fakeid', 'owner', 'cli', '{\"kind\":\"no-agent\"}', '2026-01-01T00:00:00Z', '{}');",
        )
        .expect("write v2");
    }

    // Opening with the current build migrates to v3 and leaves the event row alone.
    let store = EventStore::open(&path, user("owner")).expect("open migrates");
    assert!(store.list_reactions().expect("list reactions").is_empty());

    let c = Connection::open(&path).expect("open raw");
    let version: i32 = c
        .query_row("SELECT user_version FROM pragma_user_version", [], |r| {
            r.get(0)
        })
        .expect("version");
    assert_eq!(version, 3);
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 1, "the v2 event row survived the migration");
}

#[test]
fn opening_a_future_version_file_refuses_rather_than_best_effort_reading() {
    let tmp = TempDir::new().expect("tmp");
    let path = tmp.path().join("events.sqlite3");
    {
        let c = Connection::open(&path).expect("open raw");
        // A hypothetical v4 file: current build only knows how to migrate up to v3.
        c.execute_batch("PRAGMA user_version = 4;").expect("v4");
    }
    match EventStore::open(&path, user("owner")) {
        Err(StoreError::SchemaVersionMismatch { expected, found }) => {
            assert_eq!(expected, 3);
            assert_eq!(found, 4);
        }
        other => panic!("expected SchemaVersionMismatch, got {other:?}"),
    }
}
