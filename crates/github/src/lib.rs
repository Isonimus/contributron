//! The GitHub adapter: the one place where GitHub's vocabulary is spoken.
//!
//! Everything leaving this crate is a `contributron_domain` type. A caller cannot obtain a
//! GitHub payload struct and a domain value from the same call and get them confused,
//! because normalization consumes the former and returns the latter.

pub mod capture;
pub mod client;
pub mod normalize;
pub mod payload;

pub use capture::{CaptureError, PullRequestLocator, capture_pull_request};

/// Test hook exposing the GraphQL-page assembler so a slice-0021 regression test can
/// drive the pagination logic without an HTTP mock. The real walk calls this through
/// the private `capture_pull_request_with` loop.
#[doc(hidden)]
pub fn assemble_fixture_for_test(
    pages: Vec<serde_json::Value>,
) -> Result<String, capture::CaptureError> {
    capture::assemble_fixture(pages)
}
pub use client::{ClientBuildError, GitHubClient};
pub use normalize::{NormalizeError, normalize_capture, normalize_pull_request};
