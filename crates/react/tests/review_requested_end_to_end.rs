//! Slice 0017 end-to-end: the poll loop writes distinct observations across ticks, the
//! reactor sees the draft→ready transition and the fake provider receives a single
//! `request_reviewers` call whose argument matches the configured reviewer list. A second
//! scenario fails every client call (the `MissingTokenClient` shape) and asserts the rule
//! surfaces the ADR-0013 error through the poll loop's tick report.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown
)]

mod common;

use common::{FakeClient, login};
use contributron_domain::actor::{UserId, WorkerId};
use contributron_domain::pull_request::{
    AuthorLogin, Label, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
};
use contributron_poll::{
    Fetch, FetchError, FetchFuture, PollConfig, PullRequestLocator, ReactionReport, Reactor,
    ReactorFuture, Tick, TickOutcome, run_loop_with_reactor,
};
use contributron_react::{
    ClientFuture, DraftHistory, ExistingComment, ProviderClient, ProviderError,
    REVIEW_REQUESTED_RULE_AGENT_ID, ReactionOutcome, react_review_requested,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};

const MISSING_CREDENTIAL: &str = "no GitHub token available for the write path. Pass --token-path, set CONTRIBUTRON_TOKEN, \
     or store a github.com credential (ADR-0013).";

fn obs(number: u64, title: &str, draft: bool) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(number).expect("nonzero pr number"),
        title,
        draft,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("alice").expect("nonempty login"),
        },
        Mergeable::Unknown,
        Vec::new(),
    )
}

fn pull_locator(number: u64) -> PullRequestLocator {
    PullRequestLocator {
        owner: "alice".into(),
        repo: "project".into(),
        number,
    }
}

fn scripted_fetch(script: Vec<Result<ObservedPullRequest, &'static str>>) -> Fetch {
    let script = Arc::new(Mutex::new(script));
    Arc::new(move |_locator| {
        let script = Arc::clone(&script);
        Box::pin(async move {
            let mut guard = script.lock().expect("scripted_fetch mutex");
            let entry = if guard.len() > 1 {
                guard.remove(0)
            } else {
                guard
                    .first()
                    .cloned()
                    .unwrap_or(Err("scripted_fetch: empty"))
            };
            entry.map_err(|m| FetchError(m.to_string()))
        }) as FetchFuture
    })
}

fn reactor_from(client: Arc<FakeClient>, reviewers: Arc<Vec<AuthorLogin>>) -> Reactor {
    let owner = UserId::new("local-operator");
    let worker = WorkerId::new("cli");
    Arc::new(move |locator, prior, new| {
        let client = Arc::clone(&client);
        let reviewers = Arc::clone(&reviewers);
        let owner = owner.clone();
        let worker = worker.clone();
        Box::pin(async move {
            match react_review_requested(
                &owner,
                &worker,
                &locator,
                prior.as_ref(),
                &new,
                client.as_ref(),
                client.as_ref(),
                &reviewers,
            )
            .await
            {
                Ok(ReactionOutcome::NotApplicable) => ReactionReport::NotApplicable,
                Ok(ReactionOutcome::AlreadySettled) => ReactionReport::AlreadySettled {
                    agent_ids: vec![REVIEW_REQUESTED_RULE_AGENT_ID.to_owned()],
                    errors: Vec::new(),
                },
                Ok(ReactionOutcome::Posted { .. }) => ReactionReport::Posted {
                    agent_ids: vec![REVIEW_REQUESTED_RULE_AGENT_ID.to_owned()],
                    errors: Vec::new(),
                },
                Err(e) => ReactionReport::Failed {
                    error: format!("{e}"),
                },
            }
        }) as ReactorFuture
    })
}

/// DoD scenario 1 driven end-to-end: a draft→ready transition is written to the store,
/// the reactor fires the rule, and the fake provider receives a single
/// `request_reviewers` call with the full configured list.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn draft_to_ready_transition_requests_the_configured_reviewers() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(11);

    let script = vec![Ok(obs(11, "drafting", true)), Ok(obs(11, "ready", false))];
    let fetch = scripted_fetch(script);

    let client = Arc::new(FakeClient::default());
    client.set_requested_reviewers([]);
    let reviewers = Arc::new(vec![login("reviewer-one"), login("reviewer-two")]);
    let reactor = reactor_from(Arc::clone(&client), Arc::clone(&reviewers));

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
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

    let mut ticks: Vec<Tick> = Vec::new();
    for _ in 0..2 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        ticks.push(events_rx.recv().await.expect("a tick"));
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    assert!(matches!(ticks[0].outcome, TickOutcome::Fresh));
    assert!(matches!(
        ticks[0].reaction,
        Some(ReactionReport::NotApplicable),
    ));

    assert!(matches!(ticks[1].outcome, TickOutcome::Fresh));
    assert!(
        matches!(ticks[1].reaction, Some(ReactionReport::Posted { .. })),
        "tick 2 reaction should be Posted, got {:?}",
        ticks[1].reaction,
    );

    let calls = client.request_reviewers_calls();
    assert_eq!(
        calls,
        vec![vec!["reviewer-one".to_owned(), "reviewer-two".to_owned()]],
    );
    assert_eq!(
        client.requested_reviewers_snapshot(),
        vec!["reviewer-one".to_owned(), "reviewer-two".to_owned()],
    );
}

/// A client that fails every call with the ADR-0013 message. Used to drive DoD scenario 6
/// end-to-end: a draft→ready transition without a credential surfaces the error through
/// the tick report rather than silently no-opping.
#[derive(Debug, Default)]
struct MissingTokenClient;

impl ProviderClient for MissingTokenClient {
    fn list_comments<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<ExistingComment>> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn post_comment<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _body: &'a str,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn live_merge_has_conflict<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn list_labels<'a>(&'a self, _locator: &'a PullRequestLocator) -> ClientFuture<'a, Vec<Label>> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn add_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn remove_label<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn list_requested_reviewers<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<AuthorLogin>> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }

    fn request_reviewers<'a>(
        &'a self,
        _locator: &'a PullRequestLocator,
        _reviewers: &'a [AuthorLogin],
    ) -> ClientFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::Client(MISSING_CREDENTIAL.to_owned())) })
    }
}

/// A trivial [`DraftHistory`] that says the stored prior was draft.
#[derive(Debug, Default)]
struct DraftHistoryWasDraft;

impl DraftHistory for DraftHistoryWasDraft {
    fn last_draft(&self, _locator: &PullRequestLocator) -> Result<Option<bool>, String> {
        Ok(Some(true))
    }
}

/// DoD scenario 6 driven end-to-end: with a configured reviewer and no credential, a
/// draft→ready transition is reported as a failed reaction (the error carries the
/// ADR-0013 message) rather than silently no-opping.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn draft_to_ready_without_a_credential_surfaces_the_adr_0013_error() {
    let tmp = TempDir::new().expect("a temporary directory");
    let store_path = tmp.path().join("events.sqlite3");
    let target = pull_locator(13);

    let script = vec![Ok(obs(13, "drafting", true)), Ok(obs(13, "ready", false))];
    let fetch = scripted_fetch(script);

    let reviewers = Arc::new(vec![login("reviewer-one")]);

    let owner = UserId::new("local-operator");
    let worker = WorkerId::new("cli");
    let client: Arc<dyn ProviderClient> = Arc::new(MissingTokenClient);
    let history: Arc<dyn DraftHistory> = Arc::new(DraftHistoryWasDraft);
    let reactor: Reactor = {
        let reviewers = Arc::clone(&reviewers);
        let client = Arc::clone(&client);
        let history = Arc::clone(&history);
        Arc::new(move |locator, prior, new| {
            let client = Arc::clone(&client);
            let history = Arc::clone(&history);
            let reviewers = Arc::clone(&reviewers);
            let owner = owner.clone();
            let worker = worker.clone();
            Box::pin(async move {
                match react_review_requested(
                    &owner,
                    &worker,
                    &locator,
                    prior.as_ref(),
                    &new,
                    history.as_ref(),
                    client.as_ref(),
                    &reviewers,
                )
                .await
                {
                    Ok(ReactionOutcome::NotApplicable) => ReactionReport::NotApplicable,
                    Ok(ReactionOutcome::AlreadySettled) => ReactionReport::AlreadySettled {
                        agent_ids: vec![REVIEW_REQUESTED_RULE_AGENT_ID.to_owned()],
                        errors: Vec::new(),
                    },
                    Ok(ReactionOutcome::Posted { .. }) => ReactionReport::Posted {
                        agent_ids: vec![REVIEW_REQUESTED_RULE_AGENT_ID.to_owned()],
                        errors: Vec::new(),
                    },
                    Err(e) => ReactionReport::Failed {
                        error: format!("{e}"),
                    },
                }
            }) as ReactorFuture
        })
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Tick>();

    let interval = Duration::from_secs(60);
    let config = PollConfig {
        targets: vec![target.clone()],
        interval,
        owner: UserId::new("local-operator"),
    };

    let store_path_for_loop = store_path.clone();
    let loop_handle = tokio::spawn(async move {
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

    let mut ticks: Vec<Tick> = Vec::new();
    for _ in 0..2 {
        tokio::time::advance(interval.saturating_mul(2)).await;
        ticks.push(events_rx.recv().await.expect("a tick"));
    }

    cancel_tx.send(true).expect("cancel receiver still alive");
    drop(cancel_tx);
    loop_handle.await.expect("join").expect("loop ok");

    match &ticks[1].reaction {
        Some(ReactionReport::Failed { error }) => {
            assert!(
                error.contains("ADR-0013"),
                "error should cite ADR-0013, got: {error}",
            );
            assert!(
                error.contains("token"),
                "error should name the missing token, got: {error}",
            );
        }
        other => panic!("tick 2 should be Failed, got {other:?}"),
    }
}
