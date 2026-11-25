//! GitHub's own shapes, as they arrive over GraphQL.
//!
//! Every field a normalized value requires is an [`Option`] here, which is not the
//! "unwrap it later" pattern the quality bar forbids but its opposite. A non-optional field
//! makes serde fail during deserialization with a message about the whole document, so the
//! adapter cannot say *which* field was wrong. Taking the field as optional and checking it
//! one line later in `normalize` is what lets the error name it. Nothing reads one of these
//! options without validating it.

use contributron_domain::pull_request::Mergeable;
use serde::{Deserialize, Deserializer};

/// serde adapter that distinguishes an absent key from a present-but-null value.
///
/// Without this, `#[serde(default)] Option<T>` collapses both into the same `None`, and
/// the normalizer cannot tell the slice-0022 case apart ("field absent" is a payload the
/// rule cannot read; "field null" is a legitimate "still computing" state).
#[allow(clippy::option_option)]
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

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
    /// `mergeable`. GitHub's schema declares this nullable (null while mergeability is
    /// being computed), and the slice-0015 rule depends on seeing that transient state.
    /// The outer `Option` is the "field absent" signal (slice 0022): `None` means the
    /// key was missing from the payload (the normalizer errors), `Some(None)` means
    /// the key was present with a null value (legitimate "computing"), and
    /// `Some(Some(v))` is a concrete mergeability.
    // `Option<Option<_>>` is the pattern serde uses to distinguish "absent" from
    // "present-but-null". A custom three-variant enum would read the same way and cost a
    // deserializer impl this payload does not need.
    #[allow(clippy::option_option)]
    #[serde(default, deserialize_with = "double_option")]
    pub mergeable: Option<Option<Mergeable>>,
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
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadsPayload {
    /// `nodes`.
    pub nodes: Option<Vec<ReviewThreadPayload>>,
    /// `pageInfo`, which is how the provider says whether this page is the whole set.
    pub page_info: Option<PageInfoPayload>,
}

/// A connection's pagination state.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfoPayload {
    /// `hasNextPage`.
    pub has_next_page: Option<bool>,
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
