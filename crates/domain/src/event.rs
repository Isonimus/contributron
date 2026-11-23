//! Observations and their identity.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

use crate::actor::Attribution;
use crate::pull_request::ObservedPullRequest;

/// The identity of an observation, derived from what was observed.
///
/// Derived rather than assigned, so that two processes observing the same state agree on the
/// identity without coordinating, and a later slice can deduplicate redeliveries on it. A
/// counter or a UUID would make every poll a new event and push the deduplication problem
/// into whatever stores them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventId(String);

impl EventId {
    /// The identity as lowercase hexadecimal.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An observation could not be encoded canonically, so no identity could be derived for it.
///
/// Unreachable for the types in this crate, whose `Serialize` implementations are derived and
/// hold no map with non-string keys. It is propagated rather than collapsed because an
/// identity that silently falls back to something else is a deduplication bug that surfaces
/// as a duplicate action taken against a repository.
#[derive(Debug, thiserror::Error)]
#[error("an observation could not be encoded canonically: {0}")]
pub struct CanonicalEncodingError(#[from] serde_json::Error);

/// One observation of a pull request, and who is accountable for having made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestObservation {
    /// The three identities behind this record (ADR-0005).
    pub attribution: Attribution,
    /// What was observed.
    pub pull_request: ObservedPullRequest,
}

impl PullRequestObservation {
    /// Builds an observation.
    #[must_use]
    pub fn new(attribution: Attribution, pull_request: ObservedPullRequest) -> Self {
        Self {
            attribution,
            pull_request,
        }
    }

    /// The observation's identity, derived from the observed content.
    ///
    /// [`Attribution`] is deliberately excluded. Two workers observing the same pull request
    /// state have seen one fact, not two, and an identity that varied by observer would
    /// defeat the deduplication it exists for. Who observed it stays on the record; it just
    /// does not decide what the record *is*.
    ///
    /// The canonical form is JSON over the observed content. JSON escapes its strings, so no
    /// field value can impersonate a field boundary, which is the property a concatenation
    /// of raw bytes would lack.
    ///
    /// # Errors
    ///
    /// Returns [`CanonicalEncodingError`] if the observed content cannot be encoded.
    pub fn event_id(&self) -> Result<EventId, CanonicalEncodingError> {
        let canonical = serde_json::to_vec(&self.pull_request)?;
        let digest = Sha256::digest(&canonical);
        Ok(EventId(hex(&digest)))
    }
}

/// Lowercase hexadecimal, written by hand rather than with `format!` per byte.
///
/// The idiom clippy suggests for the latter folds a `write!` whose `Result` is discarded with
/// `let _ =`, which this repo forbids (docs/quality-bar.md, "fail loud"). A nibble table needs
/// no error path at all.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];

    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let (high, low) = (usize::from(byte / 16), usize::from(byte % 16));
        out.push(DIGITS[high]);
        out.push(DIGITS[low]);
    }
    out
}
