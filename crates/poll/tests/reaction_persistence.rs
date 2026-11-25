//! Slice 0019: a tick whose reactor returns `Posted` or `AlreadySettled` writes one row per
//! `agent_id` into the store's `reactions` table, with the three-identity tuple, the pull-
//! request locator and the outcome. `NotApplicable` and `Failed` write nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{locator, observation, scripted_fetch, user};
use contributron_domain::actor::{ExecutingAgent, WorkerId};
use contributron_poll::{PollConfig, ReactionReport, Reactor, Tick, run_loop_with_reactor};
use contributron_store::{EventStore, ReactionRecordOutcome};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

fn constant_reactor(report: ReactionReport) -> Reactor {
    Arc::new(move |_, _, _| {
        let r = report.clone();
        Box::pin(async move { r })
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn posted_tick_persists_one_row_per_agent_id() {
    let tmp = TempDir::new().expect("tmp");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    let fetch = scripted_fetch(vec![Ok(observation(7, "the-pr"))]);
    let reactor = constant_reactor(ReactionReport::Posted {
        agent_ids: vec![
            "rule:threads-cleared".into(),
            "rule:needs-rebase-label".into(),
        ],
        errors: Vec::new(),
    });

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: WorkerId::new("cli").expect("static test id"),
    };

    let store_path_for_loop = store_path.clone();
    let handle = tokio::spawn(async move {
        run_loop_with_reactor(
            config,
            fetch,
            Some(reactor),
            &store_path_for_loop,
            events_tx,
            cancel_rx,
        )
        .await
    });

    tokio::time::advance(interval.saturating_mul(2)).await;
    let tick = events_rx.recv().await.expect("tick");

    cancel_tx.send(true).expect("cancel");
    drop(cancel_tx);
    handle.await.expect("join").expect("loop ok");

    assert!(
        tick.persist_errors.is_empty(),
        "persist should succeed, got {:?}",
        tick.persist_errors,
    );

    let store = EventStore::open(&store_path, owner.clone()).expect("open");
    let rows = store.list_reactions().expect("list");
    assert_eq!(rows.len(), 2, "one row per agent id");

    let mut agent_ids: Vec<String> = rows
        .iter()
        .map(|r| match &r.attribution.agent {
            ExecutingAgent::Agent { id } => id.as_str().to_owned(),
            ExecutingAgent::NoAgent => panic!("expected an Agent attribution"),
        })
        .collect();
    agent_ids.sort();
    assert_eq!(
        agent_ids,
        vec!["rule:needs-rebase-label", "rule:threads-cleared"],
    );

    for row in &rows {
        assert_eq!(row.attribution.user.as_str(), "local-operator");
        assert_eq!(row.attribution.worker.as_str(), "cli");
        assert_eq!(row.pr_owner, "alice");
        assert_eq!(row.pr_repo, "project");
        assert_eq!(row.pr_number, 7);
        assert_eq!(row.outcome, ReactionRecordOutcome::Posted);
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn already_settled_tick_persists_one_row_with_the_matching_outcome() {
    let tmp = TempDir::new().expect("tmp");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = user("local-operator");
    let target = locator("alice", "project", 7);

    let fetch = scripted_fetch(vec![Ok(observation(7, "the-pr"))]);
    let reactor = constant_reactor(ReactionReport::AlreadySettled {
        agent_ids: vec!["rule:threads-cleared".into()],
        errors: Vec::new(),
    });

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: WorkerId::new("cli").expect("static test id"),
    };

    let store_path_for_loop = store_path.clone();
    let handle = tokio::spawn(async move {
        run_loop_with_reactor(
            config,
            fetch,
            Some(reactor),
            &store_path_for_loop,
            events_tx,
            cancel_rx,
        )
        .await
    });

    tokio::time::advance(interval.saturating_mul(2)).await;
    let _ = events_rx.recv().await.expect("tick");
    cancel_tx.send(true).expect("cancel");
    drop(cancel_tx);
    handle.await.expect("join").expect("loop ok");

    let store = EventStore::open(&store_path, owner).expect("open");
    let rows = store.list_reactions().expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, ReactionRecordOutcome::AlreadySettled);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn not_applicable_and_failed_ticks_write_nothing() {
    for report in [
        ReactionReport::NotApplicable,
        ReactionReport::Failed {
            error: "boom".into(),
        },
    ] {
        let tmp = TempDir::new().expect("tmp");
        let store_path = tmp.path().join("events.sqlite3");
        let owner = user("local-operator");
        let target = locator("alice", "project", 7);

        let fetch = scripted_fetch(vec![Ok(observation(7, "the-pr"))]);
        let reactor = constant_reactor(report.clone());

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

        let interval = Duration::from_secs(60);
        let config = PollConfig {
            targets: vec![target.clone()],
            interval,
            owner: owner.clone(),
            worker: WorkerId::new("cli").expect("static test id"),
        };

        let store_path_for_loop = store_path.clone();
        let handle = tokio::spawn(async move {
            run_loop_with_reactor(
                config,
                fetch,
                Some(reactor),
                &store_path_for_loop,
                events_tx,
                cancel_rx,
            )
            .await
        });

        tokio::time::advance(interval.saturating_mul(2)).await;
        let tick = events_rx.recv().await.expect("tick");
        cancel_tx.send(true).expect("cancel");
        drop(cancel_tx);
        handle.await.expect("join").expect("loop ok");

        assert!(tick.persist_errors.is_empty(), "no persist attempted");
        let store = EventStore::open(&store_path, owner).expect("open");
        assert!(
            store.list_reactions().expect("list").is_empty(),
            "no rows should be written for {report:?}",
        );
    }
}
