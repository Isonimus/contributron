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

/// How many review threads one GraphQL request reads.
///
/// A hundred is GitHub's maximum page size. The capture walks pages using `$after` /
/// `pageInfo.endCursor` until `hasNextPage` is `false`, so the total is unbounded by
/// this constant; it only controls the per-request round-trip size. The accumulated
/// nodes are then written into one synthetic payload whose `hasNextPage` is `false`, so
/// the normalizer continues to refuse a truncated payload by construction (slice 0021).
const THREAD_PAGE_SIZE: u32 = 100;

const QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!, $threads: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number
      title
      isDraft
      author { login }
      mergeable
      reviewThreads(first: $threads, after: $after) {
        pageInfo { hasNextPage endCursor }
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
    after: Option<String>,
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
    capture_pull_request_with(&client, locator).await
}

async fn capture_pull_request_with(
    client: &octocrab::Octocrab,
    locator: &PullRequestLocator,
) -> Result<String, CaptureError> {
    let mut pages: Vec<serde_json::Value> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let data: serde_json::Value = client
            .graphql(&serde_json::json!({
                "query": QUERY,
                "variables": Variables {
                    owner: locator.owner.clone(),
                    name: locator.repo.clone(),
                    number: locator.number,
                    threads: THREAD_PAGE_SIZE,
                    after: after.clone(),
                },
            }))
            .await?;
        let next = next_cursor_after(&data)?;
        pages.push(data);
        match next {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    let fixture = assemble_fixture(pages)?;
    normalize_capture(&fixture)?;
    Ok(fixture)
}

/// If the just-fetched page says another follows, returns its cursor. If this is the
/// final page, returns `None`. A page that reports `hasNextPage: true` with
/// `endCursor: null` is a shape the walk cannot drive forward, so the capture errors out
/// (rather than looping).
fn next_cursor_after(data: &serde_json::Value) -> Result<Option<String>, CaptureError> {
    let page_info = data
        .pointer("/repository/pullRequest/reviewThreads/pageInfo")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let has_next = page_info
        .get("hasNextPage")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !has_next {
        return Ok(None);
    }
    match page_info
        .get("endCursor")
        .and_then(serde_json::Value::as_str)
    {
        Some(cursor) => Ok(Some(cursor.to_owned())),
        None => Err(CaptureError::Diverged(NormalizeError::Truncated {
            field: "repository.pullRequest.reviewThreads".to_owned(),
        })),
    }
}

/// Stitches `pages` into one synthetic payload: the first page's shape, with the
/// concatenated `nodes` and `pageInfo.hasNextPage: false`. A hand-captured fixture (one
/// page) stays byte-comparable for the common case.
///
/// # Errors
///
/// Returns [`CaptureError::Diverged`] if `pages` is empty or if the first page does not
/// hold `repository.pullRequest`.
pub(crate) fn assemble_fixture(pages: Vec<serde_json::Value>) -> Result<String, CaptureError> {
    let mut iter = pages.into_iter();
    let Some(mut base) = iter.next() else {
        return Err(CaptureError::Diverged(NormalizeError::MissingField {
            field: "repository.pullRequest".to_owned(),
        }));
    };
    let mut accumulated_nodes: Vec<serde_json::Value> = base
        .pointer("/repository/pullRequest/reviewThreads/nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for page in iter {
        if let Some(nodes) = page
            .pointer("/repository/pullRequest/reviewThreads/nodes")
            .and_then(|v| v.as_array())
        {
            accumulated_nodes.extend(nodes.iter().cloned());
        }
    }
    if let Some(threads) = base.pointer_mut("/repository/pullRequest/reviewThreads")
        && let Some(obj) = threads.as_object_mut()
    {
        obj.insert(
            "nodes".to_owned(),
            serde_json::Value::Array(accumulated_nodes),
        );
        obj.insert(
            "pageInfo".to_owned(),
            serde_json::json!({ "hasNextPage": false, "endCursor": serde_json::Value::Null }),
        );
    }
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({ "data": base }),
    )?)
}
