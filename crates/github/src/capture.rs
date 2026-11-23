//! Fetching one pull request from the live API, in the shape the fixtures record.
//!
//! The only part of this crate that touches a network, kept apart from `normalize` so that
//! normalization stays a pure function and every offline scenario can be asserted without a
//! token or a rate limit.

use serde::Serialize;

use crate::normalize::{NormalizeError, normalize_capture};

/// Which pull request to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestLocator {
    /// The repository owner.
    pub owner: String,
    /// The repository name.
    pub repo: String,
    /// The pull request's number.
    pub number: u64,
}

/// How many review threads one capture reads.
///
/// This slice observes a pull request's threads; it does not claim to paginate them. A
/// hundred is GitHub's maximum page size, and a pull request with more is a capture this code
/// silently truncates, which is why the limitation is a ledger item rather than a comment
/// nobody reads.
const THREAD_PAGE_SIZE: u32 = 100;

const QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!, $threads: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number
      title
      isDraft
      author { login }
      reviewThreads(first: $threads) {
        nodes { id isResolved }
      }
    }
  }
}";

/// Why a capture did not produce a usable fixture.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The API call itself failed: transport, authentication, rate limit or a GraphQL error.
    #[error("the GitHub API call failed: {0}")]
    Api(#[from] octocrab::Error),

    /// The response could not be re-encoded as a fixture.
    #[error("the captured response could not be encoded: {0}")]
    Encoding(#[from] serde_json::Error),

    /// The response was fetched but the normalizer rejected it, which means the live API no
    /// longer has the shape the recorded fixtures assume.
    #[error(
        "the live API returned a payload this adapter cannot normalize, so the recorded \
         fixtures no longer describe GitHub: {0}"
    )]
    Diverged(#[from] NormalizeError),
}

#[derive(Debug, Serialize)]
struct Variables {
    owner: String,
    name: String,
    number: u64,
    threads: u32,
}

/// Captures one pull request and returns it as fixture text.
///
/// The returned text carries the `data` wrapper that a GraphQL response has, even though
/// `octocrab::Octocrab::graphql` strips it: a recorded fixture is then byte-comparable with
/// what `gh api graphql` writes, so a fixture can be produced by hand without this command
/// and still be read by the same normalizer.
///
/// The capture is normalized before it is returned. A fixture that the normalizer would
/// reject is not written, because a recorded payload nothing can read is worse than no
/// recording: it looks like evidence.
///
/// # Errors
///
/// Returns [`CaptureError`] if the API call fails, the response cannot be re-encoded, or the
/// response no longer matches what this adapter normalizes.
pub async fn capture_pull_request(
    token: String,
    locator: &PullRequestLocator,
) -> Result<String, CaptureError> {
    let client = octocrab::Octocrab::builder()
        .personal_token(token)
        .build()?;

    let data: serde_json::Value = client
        .graphql(&serde_json::json!({
            "query": QUERY,
            "variables": Variables {
                owner: locator.owner.clone(),
                name: locator.repo.clone(),
                number: locator.number,
                threads: THREAD_PAGE_SIZE,
            },
        }))
        .await?;

    let fixture = serde_json::to_string_pretty(&serde_json::json!({ "data": data }))?;
    normalize_capture(&fixture)?;
    Ok(fixture)
}
