//! Turning a GitHub payload into domain values, or failing in a way that names the cause.
//!
//! Pure: no clock, no filesystem, no network. That is what lets every acceptance scenario
//! but the live capture be asserted offline against recorded fixtures.

use contributron_domain::pull_request::{
    AuthorLogin, ObservedPullRequest, PullRequestNumber, ReviewThread, ThreadId, ThreadResolution,
};

use crate::payload::{CaptureEnvelope, PullRequestPayload};

/// Why a payload could not become a domain value.
#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    /// The payload was not valid JSON, or did not have the shape of a GraphQL response.
    #[error("the captured payload is not a readable GraphQL response: {0}")]
    Unreadable(#[from] serde_json::Error),

    /// A field the domain model requires was absent, or null where the schema forbids null.
    #[error("the payload is missing a required field: {field}")]
    MissingField {
        /// The path of the absent field within the payload.
        field: String,
    },

    /// A field was present but held a value this domain does not model.
    #[error("the payload field {field} holds a value outside its domain: {detail}")]
    OutOfDomain {
        /// The path of the offending field within the payload.
        field: String,
        /// What was wrong with the value.
        detail: String,
    },
}

const PULL_REQUEST: &str = "data.repository.pullRequest";

fn required<T>(field: &str, value: Option<T>) -> Result<T, NormalizeError> {
    value.ok_or_else(|| NormalizeError::MissingField {
        field: field.to_owned(),
    })
}

fn out_of_domain<T>(field: &str, detail: &str) -> Result<T, NormalizeError> {
    Err(NormalizeError::OutOfDomain {
        field: field.to_owned(),
        detail: detail.to_owned(),
    })
}

/// Normalizes a captured GraphQL response for one pull request.
///
/// # Errors
///
/// Returns [`NormalizeError`] naming the offending field. No partially populated value is
/// produced and nothing is defaulted: a payload that cannot be understood stops here rather
/// than entering the domain as a plausible-looking guess.
pub fn normalize_capture(raw: &str) -> Result<ObservedPullRequest, NormalizeError> {
    let envelope: CaptureEnvelope = serde_json::from_str(raw)?;
    let data = required("data", envelope.data)?;
    let repository = required("data.repository", data.repository)?;
    let payload = required(PULL_REQUEST, repository.pull_request)?;
    normalize_pull_request(&payload)
}

/// Normalizes one deserialized pull request payload.
///
/// # Errors
///
/// Returns [`NormalizeError`] naming the offending field, as [`normalize_capture`] does.
pub fn normalize_pull_request(
    payload: &PullRequestPayload,
) -> Result<ObservedPullRequest, NormalizeError> {
    let number_field = format!("{PULL_REQUEST}.number");
    let raw_number = required(&number_field, payload.number)?;
    let Ok(number) = PullRequestNumber::new(raw_number) else {
        return out_of_domain(&number_field, "pull requests are numbered from one");
    };

    let title_field = format!("{PULL_REQUEST}.title");
    let title = required(&title_field, payload.title.clone())?;

    let draft_field = format!("{PULL_REQUEST}.isDraft");
    let draft = required(&draft_field, payload.is_draft)?;

    let author_field = format!("{PULL_REQUEST}.author.login");
    let author_payload = required(&format!("{PULL_REQUEST}.author"), payload.author.as_ref())?;
    let raw_login = required(&author_field, author_payload.login.clone())?;
    let Ok(author) = AuthorLogin::new(raw_login) else {
        return out_of_domain(&author_field, "an author login cannot be empty");
    };

    let threads = normalize_threads(payload)?;

    Ok(ObservedPullRequest::new(
        number, title, draft, author, threads,
    ))
}

fn normalize_threads(payload: &PullRequestPayload) -> Result<Vec<ReviewThread>, NormalizeError> {
    let connection = required(
        &format!("{PULL_REQUEST}.reviewThreads"),
        payload.review_threads.as_ref(),
    )?;
    let nodes = required(
        &format!("{PULL_REQUEST}.reviewThreads.nodes"),
        connection.nodes.as_ref(),
    )?;

    let mut threads = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let node_path = format!("{PULL_REQUEST}.reviewThreads.nodes[{index}]");

        let id_field = format!("{node_path}.id");
        let raw_id = required(&id_field, node.id.clone())?;
        let Ok(id) = ThreadId::new(raw_id) else {
            return out_of_domain(&id_field, "a thread identifier cannot be empty");
        };

        // GitHub's schema declares isResolved non-null, so a null is the provider reporting a
        // state this domain has no variant for. Defaulting it to unresolved would invent an
        // open thread, and to resolved would hide one.
        let resolved_field = format!("{node_path}.isResolved");
        let resolution = if required(&resolved_field, node.is_resolved)? {
            ThreadResolution::Resolved
        } else {
            ThreadResolution::Unresolved
        };

        threads.push(ReviewThread { id, resolution });
    }

    Ok(threads)
}
