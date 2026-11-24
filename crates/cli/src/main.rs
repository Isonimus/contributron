//! The Contributron command line.

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, Subcommand};
use contributron_cli::{render_state, token};
use contributron_domain::actor::UserId;
use contributron_github::{
    PullRequestLocator as GhLocator, capture_pull_request, normalize_capture,
};
use contributron_poll::{
    Fetch, FetchError, FetchFuture, PollConfig, PullRequestLocator, Tick, TickOutcome, poll_once,
    run_loop,
};
use contributron_store::EventStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Until credentials and a real operator identity exist, every local invocation is the same
/// operator on the same worker. These are placeholders for *who*, never for *whether*: the
/// record carries all three identities from the first commit (ADR-0005), and the ledger
/// carries the item that replaces these two constants with an authenticated identity.
const LOCAL_USER: &str = "local-operator";
const CLI_WORKER: &str = "cli";
const DEFAULT_INTERVAL_SECS: u64 = 60;
/// Upper bound on `--interval-secs`. One day is more than any observation workflow needs,
/// and the cap guards against an operator typo turning into an `Instant`-overflow hot loop
/// deeper down (finding 2 of the 2026-11-24 review). Values above this are refused at the
/// boundary rather than silently misinterpreted.
const MAX_INTERVAL_SECS: u64 = 86_400;

#[derive(Debug, Parser)]
#[command(name = "contributron", about = "Observe repository activity", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render observed state from a recorded capture.
    Render {
        /// Path to a recorded GraphQL capture.
        #[arg(long)]
        capture: PathBuf,
    },
    /// Record a pull request from the live API, and check this adapter still reads it.
    Capture {
        /// The repository, as `owner/name`.
        #[arg(long)]
        repo: String,
        /// The pull request number.
        #[arg(long)]
        pull_request: u64,
        /// Where to write the recorded capture.
        #[arg(long)]
        out: PathBuf,
    },
    /// Normalize a recorded capture and persist it in the durable store.
    Record {
        /// Path to a recorded GraphQL capture.
        #[arg(long)]
        capture: PathBuf,
        /// Path to the SQLite store (slice 0009, ADR-0010).
        #[arg(long)]
        store: PathBuf,
    },
    /// Render observed state read from the durable store.
    List {
        /// Path to the SQLite store (slice 0009, ADR-0010).
        #[arg(long)]
        store: PathBuf,
    },
    /// Poll one or more pull requests on a cadence and write each observation to the store.
    ///
    /// Runs until SIGINT (ADR-0012). One stdout line per tick (`fresh owner/name#N` or
    /// `unchanged owner/name#N`); fetch errors are on stderr.
    Watch {
        /// A target to watch, as `owner/name#number`. Repeat for multiple.
        #[arg(long = "target", value_parser = parse_target)]
        targets: Vec<PullRequestLocator>,
        /// Base poll interval in seconds (default: 60).
        #[arg(long = "interval-secs", default_value_t = DEFAULT_INTERVAL_SECS)]
        interval_secs: u64,
        /// Path to the SQLite store (slice 0009, ADR-0010).
        #[arg(long)]
        store: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render { capture } => render(&capture),
        Command::Capture {
            repo,
            pull_request,
            out,
        } => capture(&repo, pull_request, &out),
        Command::Record { capture, store } => record_to_store(&capture, &store),
        Command::List { store } => list_from_store(&store),
        Command::Watch {
            targets,
            interval_secs,
            store,
        } => run_watch(targets, interval_secs, &store),
    }
}

fn record_to_store(capture: &Path, store_path: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(capture)
        .with_context(|| format!("reading the capture at {}", capture.display()))?;
    let pull_request = normalize_capture(&raw)
        .with_context(|| format!("normalizing the capture at {}", capture.display()))?;

    // A `record` from a file is a `poll_once` whose fetch reads a file instead of the live
    // API. The shared "fetch one, write one, report outcome" shape has one home in
    // `contributron_poll::poll_once`; the CLI wires one fetch for record and another for
    // watch (slice 0011).
    let fetch: Fetch = {
        let observation = pull_request;
        Arc::new(move |_locator| {
            let o = observation.clone();
            Box::pin(async move { Ok(o) }) as FetchFuture
        })
    };
    let owner = UserId::new(LOCAL_USER);
    let store = EventStore::open(store_path, owner.clone())
        .with_context(|| format!("opening the store at {}", store_path.display()))?;
    // The locator is a placeholder because `record` reads a file rather than a target: the
    // fetch closure ignores it. `watch` is the subcommand whose fetch uses the locator to
    // address a provider.
    let placeholder = PullRequestLocator {
        owner: "local".into(),
        repo: "capture".into(),
        number: 0,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting an async runtime for the record")?;

    let result = runtime
        .block_on(poll_once(&store, &fetch, &placeholder, &owner, CLI_WORKER))
        .with_context(|| format!("writing to the store at {}", store_path.display()))?;
    match result {
        Ok(contributron_store::WriteOutcome::Fresh) => println!("recorded"),
        Ok(contributron_store::WriteOutcome::Duplicate) => println!("already recorded"),
        // Not reachable for the file-reading fetch above (it cannot fail), but matched
        // explicitly rather than unwrapped because the quality bar forbids unreachable
        // paths that depend on a specific wiring remaining in place.
        Err(e) => return Err(anyhow!("recording failed: {e}")),
    }
    Ok(())
}

fn list_from_store(path: &Path) -> Result<()> {
    let store = EventStore::open(path, UserId::new(LOCAL_USER))
        .with_context(|| format!("opening the store at {}", path.display()))?;
    let observations = store
        .list()
        .with_context(|| format!("reading from the store at {}", path.display()))?;
    print!("{}", render_state(&observations));
    Ok(())
}

fn render(capture: &Path) -> Result<()> {
    use contributron_domain::actor::{Attribution, WorkerId};
    use contributron_domain::event::PullRequestObservation;

    let raw = std::fs::read_to_string(capture)
        .with_context(|| format!("reading the capture at {}", capture.display()))?;
    let pull_request = normalize_capture(&raw)
        .with_context(|| format!("normalizing the capture at {}", capture.display()))?;
    let attribution =
        Attribution::local_operator(UserId::new(LOCAL_USER), WorkerId::new(CLI_WORKER));
    let observation = PullRequestObservation::new(attribution, pull_request);
    print!("{}", render_state(&[observation]));
    Ok(())
}

fn capture(repo: &str, number: u64, out: &Path) -> Result<()> {
    let locator = parse_repo(repo, number)?;
    let token = token::resolve()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting an async runtime for the capture")?;

    let fixture = runtime
        .block_on(capture_pull_request(token, &locator))
        .with_context(|| format!("capturing {repo}#{number}"))?;

    std::fs::write(out, &fixture)
        .with_context(|| format!("writing the capture to {}", out.display()))?;

    println!("wrote {}", out.display());
    render(out)
}

fn run_watch(
    targets: Vec<PullRequestLocator>,
    interval_secs: u64,
    store_path: &Path,
) -> Result<()> {
    if targets.is_empty() {
        return Err(anyhow!(
            "watch needs at least one --target (format: owner/name#number)"
        ));
    }
    let interval = validate_interval_secs(interval_secs)?;

    let owner = UserId::new(LOCAL_USER);
    let config = PollConfig {
        targets,
        interval,
        owner: owner.clone(),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting an async runtime for the watch")?;

    runtime.block_on(watch_loop(config, store_path))
}

async fn watch_loop(config: PollConfig, store_path: &Path) -> Result<()> {
    let token = token::resolve()?;
    let fetch: Fetch = Arc::new(move |locator: PullRequestLocator| {
        let token = token.clone();
        Box::pin(async move {
            let gh = GhLocator {
                owner: locator.owner.clone(),
                repo: locator.repo.clone(),
                number: locator.number,
            };
            let raw = capture_pull_request(token, &gh)
                .await
                .map_err(|e| FetchError(format!("{e}")))?;
            normalize_capture(&raw).map_err(|e| FetchError(format!("{e}")))
        }) as FetchFuture
    });

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<Tick>();

    // Translate SIGINT into a cancel flip. Dropping the ctrl_c future when the loop exits
    // is intentional: the signal is only interesting while the loop is running.
    let shutdown = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            // A send failure means the loop already returned; nothing left to cancel.
            match cancel_tx.send(true) {
                Ok(()) | Err(_) => {}
            }
        }
    });

    // Drain tick events to stdout/stderr while the loop runs.
    let reporter = tokio::spawn(async move {
        while let Some(tick) = events_rx.recv().await {
            report_tick(&tick);
        }
    });

    let result = run_loop(config, fetch, store_path, events_tx, cancel_rx).await;

    // Both auxiliary tasks terminate on their own: the signal task on cancel send, the
    // reporter when the channel closes (which happens when `run_loop` drops the sender and
    // all per-target tasks finish). Join them to surface any panic rather than leak it.
    shutdown.abort();
    drop(shutdown.await);
    reporter
        .await
        .map_err(|e| anyhow!("reporter task joined with a panic: {e}"))?;

    result.map_err(|e| anyhow!("polling loop: {e}"))
}

fn report_tick(tick: &Tick) {
    match &tick.outcome {
        TickOutcome::Fresh => println!("fresh {}", tick.locator),
        TickOutcome::Unchanged => println!("unchanged {}", tick.locator),
        TickOutcome::FetchFailed { error, backoff } => {
            eprintln!(
                "fetch-error {}: {} (next attempt in {}s)",
                tick.locator,
                error,
                backoff.as_secs()
            );
        }
        TickOutcome::StoreFailed { error, backoff } => match backoff {
            Some(b) => eprintln!(
                "store-error {}: {} (next attempt in {}s)",
                tick.locator,
                error,
                b.as_secs()
            ),
            None => eprintln!(
                "store-error {}: {} (this target is not being polled further)",
                tick.locator, error,
            ),
        },
    }
}

fn parse_repo(repo: &str, number: u64) -> Result<GhLocator> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("expected a repository as `owner/name`, got {repo:?}"))?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(anyhow!(
            "expected a repository as `owner/name`, got {repo:?}"
        ));
    }
    Ok(GhLocator {
        owner: owner.to_owned(),
        repo: name.to_owned(),
        number,
    })
}

/// Validates `--interval-secs`, converting the accepted range `[1, MAX_INTERVAL_SECS]` to a
/// `Duration`. The upper bound is a boundary check against the overflow class the review
/// flagged (finding 2 of 2026-11-24): values high enough to make `Instant + Duration`
/// overflow would silently fall back to "no delay" inside the scheduler.
fn validate_interval_secs(secs: u64) -> Result<Duration> {
    if secs == 0 {
        return Err(anyhow!("--interval-secs must be at least 1"));
    }
    if secs > MAX_INTERVAL_SECS {
        return Err(anyhow!(
            "--interval-secs must be at most {MAX_INTERVAL_SECS} (one day)"
        ));
    }
    Ok(Duration::from_secs(secs))
}

fn parse_target(raw: &str) -> Result<PullRequestLocator, String> {
    let (repo, number_str) = raw
        .split_once('#')
        .ok_or_else(|| format!("expected `owner/name#number`, got {raw:?}"))?;
    let number: u64 = number_str
        .parse()
        .map_err(|e| format!("number in {raw:?} is not a positive integer: {e}"))?;
    // Providers number pull requests from one; zero is a payload that did not mean what it
    // said. Rejecting at the boundary (finding 7c of the 2026-11-24 review) stops a
    // mis-typed target from sitting in the backoff path forever.
    if number == 0 {
        return Err(format!(
            "number in {raw:?} must be at least 1 (providers number pull requests from 1)"
        ));
    }
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("expected `owner/name#number`, got {raw:?}"))?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(format!("expected `owner/name#number`, got {raw:?}"));
    }
    Ok(PullRequestLocator {
        owner: owner.to_owned(),
        repo: name.to_owned(),
        number,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod parse_target_tests {
    use super::{MAX_INTERVAL_SECS, parse_target, validate_interval_secs};
    use std::time::Duration;

    #[test]
    fn a_zero_interval_is_refused() {
        validate_interval_secs(0).expect_err("zero");
    }

    #[test]
    fn an_oversized_interval_is_refused_at_the_boundary() {
        // The overflow class the review flagged: a very large value would make
        // `Instant::now().checked_add(Duration::from_secs(_))` return `None` in the
        // scheduler. The CLI must stop that at parse time rather than at runtime.
        let e = validate_interval_secs(u64::MAX).expect_err("u64::MAX");
        assert!(e.to_string().contains("at most"), "message was: {e:?}");
        validate_interval_secs(MAX_INTERVAL_SECS.saturating_add(1)).expect_err("one over");
    }

    #[test]
    fn the_accepted_range_includes_one_and_the_max() {
        assert_eq!(
            validate_interval_secs(1).expect("one"),
            Duration::from_secs(1),
        );
        assert_eq!(
            validate_interval_secs(MAX_INTERVAL_SECS).expect("max"),
            Duration::from_secs(MAX_INTERVAL_SECS),
        );
    }

    #[test]
    fn a_well_formed_target_parses() {
        let t = parse_target("alice/project#42").expect("parses");
        assert_eq!(t.owner, "alice");
        assert_eq!(t.repo, "project");
        assert_eq!(t.number, 42);
    }

    #[test]
    fn a_missing_hash_is_refused_naming_the_shape() {
        let e = parse_target("alice/project").expect_err("no `#`");
        assert!(e.contains("owner/name#number"), "message was: {e:?}");
    }

    #[test]
    fn a_non_numeric_number_is_refused() {
        let e = parse_target("alice/project#forty-two").expect_err("not a number");
        assert!(e.contains("not a positive integer"), "message was: {e:?}");
    }

    #[test]
    fn zero_is_refused_at_the_boundary_not_at_normalize_time() {
        let e = parse_target("alice/project#0").expect_err("zero");
        assert!(e.contains("at least 1"), "message was: {e:?}");
    }

    #[test]
    fn an_empty_owner_is_refused() {
        let e = parse_target("/project#1").expect_err("no owner");
        assert!(e.contains("owner/name#number"), "message was: {e:?}");
    }

    #[test]
    fn an_empty_name_is_refused() {
        let e = parse_target("alice/#1").expect_err("no name");
        assert!(e.contains("owner/name#number"), "message was: {e:?}");
    }

    #[test]
    fn a_missing_slash_is_refused() {
        let e = parse_target("alice#1").expect_err("no slash");
        assert!(e.contains("owner/name#number"), "message was: {e:?}");
    }

    #[test]
    fn a_trailing_slash_in_the_name_is_refused() {
        let e = parse_target("alice/project/sub#1").expect_err("second slash");
        assert!(e.contains("owner/name#number"), "message was: {e:?}");
    }
}
