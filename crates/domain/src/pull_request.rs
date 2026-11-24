//! A pull request as the system observes it, and the review threads hanging off it.

use serde::{Deserialize, Serialize};

/// A pull request's number within its repository.
///
/// A newtype rather than an integer, so that a thread identifier and a pull request number
/// cannot be passed in each other's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PullRequestNumber(u64);

impl PullRequestNumber {
    /// Wraps a pull request number.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPullRequestNumber`] for zero. Providers number pull requests from
    /// one, so zero is a payload that did not mean what it said rather than an edge case to
    /// carry forward.
    pub fn new(value: u64) -> Result<Self, InvalidPullRequestNumber> {
        if value == 0 {
            Err(InvalidPullRequestNumber)
        } else {
            Ok(Self(value))
        }
    }

    /// The number as an integer.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A pull request number outside the domain of valid numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPullRequestNumber;

/// The login of the account that opened a pull request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthorLogin(String);

impl AuthorLogin {
    /// Wraps an author login.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyAuthorLogin`] for an empty or whitespace-only login: an account with
    /// no name is a normalization failure, not an anonymous author.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyAuthorLogin> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EmptyAuthorLogin)
        } else {
            Ok(Self(value))
        }
    }

    /// The login as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An author login that held no name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyAuthorLogin;

/// Who opened a pull request.
///
/// Not an [`Option<AuthorLogin>`], for the same reason
/// [`ExecutingAgent`](crate::actor::ExecutingAgent) is not an option: an absent value and a
/// value nobody recorded are different facts. A provider reports a deleted account as a null
/// author, which is an ordinary state for an old pull request, so the domain names it rather
/// than treating the pull request as unobservable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PullRequestAuthor {
    /// The account that opened it, which still exists.
    Account {
        /// Its login.
        login: AuthorLogin,
    },
    /// The account that opened it has been deleted.
    DeletedAccount,
}

/// A review thread's provider-assigned identifier.
///
/// Opaque by design: the system compares and orders these but never parses one, so a
/// provider is free to change its encoding without reaching the domain.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ThreadId(String);

impl ThreadId {
    /// Wraps a thread identifier.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyThreadId`] for an empty identifier, which would silently collapse two
    /// distinct threads into one when threads are ordered or deduplicated.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyThreadId> {
        let value = value.into();
        if value.is_empty() {
            Err(EmptyThreadId)
        } else {
            Ok(Self(value))
        }
    }

    /// The identifier as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A thread identifier that held nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyThreadId;

/// Whether a review thread has been resolved.
///
/// Two states and no third. A provider that reports something else is reporting a state this
/// domain does not model, which the adapter must reject rather than guess at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadResolution {
    /// Someone marked the thread resolved.
    Resolved,
    /// The thread is still open.
    Unresolved,
}

/// One review thread on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReviewThread {
    /// The provider's identifier for the thread.
    pub id: ThreadId,
    /// Whether it is resolved.
    pub resolution: ThreadResolution,
}

/// A pull request and its review threads, as observed at one moment.
///
/// `threads` is kept sorted by identifier so that two observations of the same state are
/// equal, and hash equally, regardless of the order the provider happened to list them in.
///
/// Sorting happens in [`ObservedPullRequest::new`], not in the derived `Deserialize`, and
/// that is deliberate. The normalizer and the store both go through `new`: the stored
/// `payload` is always a serialization of a sorted vector, and a `Deserialize` round-trip
/// preserves vector order, so a stored row read back has the same bytes it was written
/// with. A hand-edited payload with unsorted threads would deserialize unsorted, and its
/// recomputed identity would then differ from what is keyed on in the store. That is a
/// corrupted row and the store's `Decode` path handles it loud; it is not a case this type
/// defends against by re-sorting on deserialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedPullRequest {
    /// The pull request's number in its repository.
    pub number: PullRequestNumber,
    /// Its title, as written.
    pub title: String,
    /// Whether it is a draft.
    pub draft: bool,
    /// The account that opened it, or a statement that it no longer exists.
    pub author: PullRequestAuthor,
    /// Its review threads, ordered by identifier.
    threads: Vec<ReviewThread>,
}

impl ObservedPullRequest {
    /// Builds an observation, ordering the threads canonically.
    #[must_use]
    pub fn new(
        number: PullRequestNumber,
        title: impl Into<String>,
        draft: bool,
        author: PullRequestAuthor,
        mut threads: Vec<ReviewThread>,
    ) -> Self {
        threads.sort();
        Self {
            number,
            title: title.into(),
            draft,
            author,
            threads,
        }
    }

    /// The review threads, ordered by identifier.
    #[must_use]
    pub fn threads(&self) -> &[ReviewThread] {
        &self.threads
    }

    /// How many of the threads are unresolved.
    #[must_use]
    pub fn unresolved_thread_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| t.resolution == ThreadResolution::Unresolved)
            .count()
    }
}
