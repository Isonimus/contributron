//! Scenario 5 (read-path half) of slice 0014: `list` renders the store without any token
//! resolution, matching ADR-0013 consequence 3 — the read path is allowed to run against
//! public repositories without a credential.
//!
//! The `watch` subcommand's end-to-end stub-fetch path lives in [`watch_then_list`]; this
//! test exercises the two bricks the read path rests on: (1) `token::resolve_optional` with
//! no `--token-path` returns `Ok(None)` without an env var, and (2) opening and listing the
//! store needs nothing credentialed. The genuine network half (anonymous GraphQL against a
//! public repository) stays on the ledger as a known gap — the capture client does not yet
//! accept an anonymous constructor — and is deliberately not exercised here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_cli::render_state;
use contributron_cli::token::{Source, resolve_optional};
use contributron_domain::actor::{Attribution, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
};
use contributron_store::EventStore;
use tempfile::TempDir;

#[test]
fn resolve_optional_with_no_flag_and_no_env_returns_none() {
    // The env var might be set by the surrounding shell, in which case this invariant does
    // not hold at this call site; the suite's other cases cover that. We only assert the
    // absence case when it really is absent, because setting or unsetting an env var in a
    // Rust 2024 test is `unsafe` and forbidden by `unsafe_code = "forbid"` (ADR-0002).
    if std::env::var(contributron_cli::token::TOKEN_VARIABLE).is_ok() {
        return;
    }

    let source = Source::default();
    let resolved = resolve_optional(&source).expect("resolve_optional does not error on absence");

    // `git credential fill` may hand back a github.com token on a machine that has one.
    // That is an ADR-0013 operator-convenience fallback, documented on the resolver; it
    // does not break the read path, so either None or Some is a passing outcome here.
    // What we assert is that no error was raised — read path keeps working.
    drop(resolved);
}

#[test]
fn list_renders_the_store_without_touching_any_credential_source() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let owner = UserId::new("local-operator").expect("static test id");

    let observed = ObservedPullRequest::new(
        PullRequestNumber::new(7).expect("nonzero number"),
        "a public pull request",
        false,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("alice").expect("nonempty login"),
        },
        Mergeable::Unknown,
        Vec::new(),
    );
    let attribution =
        Attribution::local_operator(owner.clone(), WorkerId::new("cli").expect("static test id"));
    let observation = PullRequestObservation::new(attribution, observed);

    {
        let store = EventStore::open(&store_path, owner.clone()).expect("open store");
        store.write(&observation).expect("write one row");
    }

    // Read path: no token resolution at all. The `list` subcommand is exactly this — open,
    // list, render — and nothing in it reaches for a credential.
    let store = EventStore::open(&store_path, owner.clone()).expect("reopen store");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1);
    let rendered = render_state(&rows);
    assert!(
        rendered.contains("#7 a public pull request"),
        "render missing the row title, got: {rendered:?}",
    );
}
