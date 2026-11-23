//! GitHub's own shapes, as they arrive over GraphQL.
//!
//! Every field a normalized value requires is an [`Option`] here, which is not the
//! "unwrap it later" pattern the quality bar forbids but its opposite. A non-optional field
//! makes serde fail during deserialization with a message about the whole document, so the
//! adapter cannot say *which* field was wrong. Taking the field as optional and checking it
//! one line later in `normalize` is what lets the error name it. Nothing reads one of these
//! options without validating it.

use serde::Deserialize;

/// A captured GraphQL response for one pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct CaptureEnvelope {
    /// The `data` member of a GraphQL response.
    pub data: Option<EnvelopeData>,
}

/// The `data` member of a captured response.
#[derive(Debug, Clone, Deserialize)]
pub struct EnvelopeData {
    /// The queried repository.
    pub repository: Option<RepositoryPayload>,
}

/// A repository, as far as this query selects it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryPayload {
    /// The queried pull request.
    pub pull_request: Option<PullRequestPayload>,
}

/// A pull request, as far as this query selects it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestPayload {
    /// `number`.
    pub number: Option<u64>,
    /// `title`.
    pub title: Option<String>,
    /// `isDraft`.
    pub is_draft: Option<bool>,
    /// `author`, which GitHub reports as null for a deleted account.
    pub author: Option<AuthorPayload>,
    /// `reviewThreads`.
    pub review_threads: Option<ReviewThreadsPayload>,
}

/// The author of a pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorPayload {
    /// `login`.
    pub login: Option<String>,
}

/// The `reviewThreads` connection.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewThreadsPayload {
    /// `nodes`.
    pub nodes: Option<Vec<ReviewThreadPayload>>,
}

/// One review thread.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadPayload {
    /// `id`.
    pub id: Option<String>,
    /// `isResolved`. GitHub's schema declares this non-null; a null here is the provider
    /// reporting a thread state this domain does not model.
    pub is_resolved: Option<bool>,
}
