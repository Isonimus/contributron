//! Rendering observed state as text.
//!
//! A library rather than code inside `main`, so the render is assertable as a string. That
//! is the whole reason the roadmap puts a CLI before any GUI: standard output is a value a
//! test can compare, and a window is not.

pub mod token;

use contributron_domain::event::PullRequestObservation;
use std::fmt::Write as _;

/// Renders the observed pull requests as the operator sees them.
///
/// Deterministic for a given input: no timestamp, no elapsed time, no iteration over an
/// unordered collection. Two runs over the same observations produce the same bytes, which
/// is what scenario five of slice 0007 asserts.
#[must_use]
pub fn render_state(observations: &[PullRequestObservation]) -> String {
    if observations.is_empty() {
        return "No open pull requests observed.\n".to_owned();
    }

    let mut out = String::new();
    for observation in observations {
        let pull_request = &observation.pull_request;
        // Writing into a String cannot fail, and `write!` returns a Result that must not be
        // discarded; `let _ =` is forbidden here, so the result is folded into the output
        // instead of being dropped.
        if write!(
            out,
            "#{number} {title}\n  unresolved threads: {unresolved}\n",
            number = pull_request.number.get(),
            title = as_one_inert_line(&pull_request.title),
            unresolved = pull_request.unresolved_thread_count(),
        )
        .is_err()
        {
            return String::new();
        }
    }
    out
}

/// Replaces every control character with the Unicode replacement character.
///
/// A pull request title is written by whoever opened the pull request, and SECURITY.md states
/// that repository content is untrusted input. This view is line-oriented, so a title holding
/// a newline would write what the operator reads as a second pull request, and an escape
/// sequence would reach their terminal. Neutralized at the render rather than at
/// normalization, because the title *is* what the provider reported: it is this presentation
/// that cannot carry it verbatim.
fn as_one_inert_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}
