//! Scenario five of slice 0007: the render names the pull request, exits zero, and is
//! deterministic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_cli::render_state;
use contributron_domain::actor::{Attribution, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
    ReviewThread, ThreadId, ThreadResolution,
};
use std::path::PathBuf;
use std::process::Command;

/// Written from the Definition of Done before the renderer existed: the operator must be
/// able to read the number, the title and how many threads are still open.
const EXPECTED_RENDER: &str = "#1 fix(index): escape backslashes before pipes in table cells\n  \
                               unresolved threads: 1\n";

fn recorded_capture() -> PathBuf {
    // The adapter crate owns the fixtures, since it owns the payload shape they record.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("github")
        .join("tests")
        .join("fixtures")
        .join("pull_request_with_threads.json")
}

fn one_open_pull_request() -> PullRequestObservation {
    let pull_request = ObservedPullRequest::new(
        PullRequestNumber::new(1).expect("one is a valid pull request number"),
        "fix(index): escape backslashes before pipes in table cells",
        false,
        PullRequestAuthor::Account {
            login: AuthorLogin::new("Isonimus").expect("a non-empty login"),
        },
        Mergeable::Unknown,
        vec![
            ReviewThread {
                id: ThreadId::new("PRRT_kwDOKt3bHM5TqX1a").expect("a non-empty identifier"),
                resolution: ThreadResolution::Resolved,
            },
            ReviewThread {
                id: ThreadId::new("PRRT_kwDOKt3bHM5TqX2b").expect("a non-empty identifier"),
                resolution: ThreadResolution::Unresolved,
            },
        ],
    );
    PullRequestObservation::new(
        Attribution::local_operator(
            UserId::new("local-operator").expect("static test id"),
            WorkerId::new("cli").expect("static test id"),
        ),
        pull_request,
    )
}

#[test]
fn the_render_names_the_number_the_title_and_the_unresolved_count() {
    assert_eq!(render_state(&[one_open_pull_request()]), EXPECTED_RENDER);
}

#[test]
fn the_render_is_byte_identical_on_a_second_call() {
    let observations = [one_open_pull_request()];
    assert_eq!(render_state(&observations), render_state(&observations));
}

#[test]
fn an_empty_observation_set_says_so_instead_of_rendering_nothing() {
    // An empty string and "nothing was observed" look the same in a terminal, and only one of
    // them tells the operator the poll ran.
    assert_eq!(render_state(&[]), "No open pull requests observed.\n");
}

#[test]
fn the_binary_renders_a_recorded_capture_exits_zero_and_repeats_byte_for_byte() {
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_contributron"))
            .arg("render")
            .arg("--capture")
            .arg(recorded_capture())
            .output()
            .expect("the contributron binary runs")
    };

    let first = run();
    assert!(
        first.status.success(),
        "exit status {:?}, stderr: {}",
        first.status.code(),
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&first.stdout), EXPECTED_RENDER);

    let second = run();
    assert_eq!(
        first.stdout, second.stdout,
        "two runs over one capture differ"
    );
}

/// A pull request title cannot forge lines in the operator's view.
///
/// Found by adversarial review. Anyone may open a pull request, so a title is
/// attacker-controlled input, and SECURITY.md states that repository content is untrusted.
/// The render is line-oriented, so a title holding a newline wrote lines the operator reads
/// as separate pull requests, and an escape sequence would reach their terminal.
#[test]
fn a_title_cannot_forge_a_line_or_reach_the_terminal() {
    let forged = "x\n#999 forged\n  unresolved threads: 0";
    let observation = PullRequestObservation::new(
        Attribution::local_operator(
            UserId::new("local-operator").expect("static test id"),
            WorkerId::new("cli").expect("static test id"),
        ),
        ObservedPullRequest::new(
            PullRequestNumber::new(1).expect("one is a valid pull request number"),
            forged,
            false,
            PullRequestAuthor::Account {
                login: AuthorLogin::new("attacker").expect("a non-empty login"),
            },
            Mergeable::Unknown,
            vec![ReviewThread {
                id: ThreadId::new("T1").expect("a non-empty identifier"),
                resolution: ThreadResolution::Unresolved,
            }],
        ),
    );

    let rendered = render_state(&[observation]);

    // Two lines per pull request and no more, whatever the title contains.
    assert_eq!(rendered.lines().count(), 2, "rendered as: {rendered:?}");
    assert!(rendered.starts_with("#1 x"));
    assert!(
        rendered.ends_with("  unresolved threads: 1\n"),
        "rendered as: {rendered:?}"
    );

    let escape_sequence = "\u{1b}[2Jcleared";
    let observation = PullRequestObservation::new(
        Attribution::local_operator(
            UserId::new("local-operator").expect("static test id"),
            WorkerId::new("cli").expect("static test id"),
        ),
        ObservedPullRequest::new(
            PullRequestNumber::new(2).expect("two is a valid pull request number"),
            escape_sequence,
            false,
            PullRequestAuthor::Account {
                login: AuthorLogin::new("attacker").expect("a non-empty login"),
            },
            Mergeable::Unknown,
            vec![],
        ),
    );

    let rendered = render_state(&[observation]);
    assert!(
        !rendered.contains('\u{1b}'),
        "an escape character reached the operator's terminal: {rendered:?}"
    );
}
