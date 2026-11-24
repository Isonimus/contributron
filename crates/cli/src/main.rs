//! The Contributron command line.

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, Subcommand};
use contributron_cli::{render_state, token};
use contributron_domain::actor::{Attribution, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_github::{PullRequestLocator, capture_pull_request, normalize_capture};
use contributron_store::EventStore;
use std::path::{Path, PathBuf};

/// Until credentials and a real operator identity exist, every local invocation is the same
/// operator on the same worker. These are placeholders for *who*, never for *whether*: the
/// record carries all three identities from the first commit (ADR-0005), and the ledger
/// carries the item that replaces these two constants with an authenticated identity.
const LOCAL_USER: &str = "local-operator";
const LOCAL_WORKER: &str = "cli";

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
    }
}

fn record_to_store(capture: &Path, store_path: &Path) -> Result<()> {
    use contributron_domain::event::PullRequestObservation;
    use contributron_store::WriteOutcome;

    let raw = std::fs::read_to_string(capture)
        .with_context(|| format!("reading the capture at {}", capture.display()))?;
    let pull_request = normalize_capture(&raw)
        .with_context(|| format!("normalizing the capture at {}", capture.display()))?;
    let attribution =
        Attribution::local_operator(UserId::new(LOCAL_USER), WorkerId::new(LOCAL_WORKER));
    let observation = PullRequestObservation::new(attribution, pull_request);

    let store = EventStore::open(store_path, UserId::new(LOCAL_USER))
        .with_context(|| format!("opening the store at {}", store_path.display()))?;
    match store
        .write(&observation)
        .with_context(|| format!("writing to the store at {}", store_path.display()))?
    {
        WriteOutcome::Fresh => println!("recorded"),
        WriteOutcome::Duplicate => println!("already recorded"),
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
    let raw = std::fs::read_to_string(capture)
        .with_context(|| format!("reading the capture at {}", capture.display()))?;
    let pull_request = normalize_capture(&raw)
        .with_context(|| format!("normalizing the capture at {}", capture.display()))?;
    let attribution =
        Attribution::local_operator(UserId::new(LOCAL_USER), WorkerId::new(LOCAL_WORKER));
    let observation = PullRequestObservation::new(attribution, pull_request);
    print!("{}", render_state(&[observation]));
    Ok(())
}

fn capture(repo: &str, number: u64, out: &Path) -> Result<()> {
    let locator = parse_repo(repo, number)?;
    let token = token::resolve()?;

    // A runtime is built only on this path. The render is synchronous and offline, and
    // starting an executor to read a file would be the kind of unexplained complexity the
    // quality bar calls a defect.
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

fn parse_repo(repo: &str, number: u64) -> Result<PullRequestLocator> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("expected a repository as `owner/name`, got {repo:?}"))?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(anyhow!(
            "expected a repository as `owner/name`, got {repo:?}"
        ));
    }
    Ok(PullRequestLocator {
        owner: owner.to_owned(),
        repo: name.to_owned(),
        number,
    })
}
