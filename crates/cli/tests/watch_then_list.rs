//! Slice 0011, scenario "CLI end-to-end": a watch run with a stub fetch writes the same
//! shape of row as the earlier CLI paths, so a `list` of the resulting store renders it.
//!
//! The scenario says "wired through the same code path as production but with a stub fetch
//! supplied for the test": production's `watch` subcommand calls [`run_loop`] after
//! building a fetch from the GitHub adapter; this test calls `run_loop` directly with a
//! stub fetch. The list half uses the same `render_state` + `EventStore::list` the
//! production `list` subcommand uses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_cli::render_state;
use contributron_domain::actor::UserId;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
};
use contributron_poll::{
    Fetch, FetchFuture, PollConfig, PullRequestLocator, Tick, TickOutcome, run_loop,
};
use contributron_store::EventStore;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn watch_then_list_renders_the_stubbed_observation() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = UserId::new("local-operator").expect("static test id");
    let target = PullRequestLocator {
        owner: "alice".into(),
        repo: "project".into(),
        number: 101,
    };

    let observed = ObservedPullRequest::new(
        PullRequestNumber::new(101).expect("nonzero number"),
        "watched from a stub",
        false,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("alice").expect("nonempty login"),
        },
        Mergeable::Unknown,
        Vec::new(),
    );

    let fetch: Fetch = {
        let obs = observed.clone();
        Arc::new(move |_locator| {
            let o = obs.clone();
            Box::pin(async move { Ok(o) }) as FetchFuture
        })
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();
    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: owner.clone(),
        worker: contributron_domain::actor::WorkerId::new("cli").expect("static test id"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
        run_loop(config, fetch, &store_path_for_loop, events_tx, cancel_rx).await
    });

    // One tick, then shut down.
    tokio::time::advance(interval.saturating_mul(2)).await;
    let tick = events_rx.recv().await.expect("one tick");
    assert!(matches!(tick.outcome, TickOutcome::Fresh));

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle
        .await
        .expect("loop joined without panic")
        .expect("loop returned Ok");

    // List side: open the store the loop wrote to and render exactly as the `list`
    // subcommand does.
    let store = EventStore::open(&store_path, owner.clone()).expect("store reopens");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1, "exactly one row from the single tick");
    let rendered = render_state(&rows);
    assert!(
        rendered.contains("#101 watched from a stub"),
        "render missing the stubbed title, got: {rendered:?}",
    );
}
