//! Deterministic reactions to observed state transitions (slice 0014).
//!
//! The first rule: when a pull request's unresolved review threads transition from non-zero
//! to zero between two consecutive observations, post one comment on that pull request. The
//! comment carries a stable idempotency marker, scanned on the pull request's existing
//! comments before any post, so a restart cannot cause a second comment.
//!
//! This crate holds the pure logic: compare a pair of observations, decide whether to react,
//! query a provider client for existing comments, and ask it to post one. The provider
//! client is a trait, not a concrete octocrab client, so the rule is testable without a
//! network and the GitHub-specific wiring stays in `crates/github/` (invariant 1).

use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::pull_request::{Mergeable, ObservedPullRequest};
use contributron_poll::PullRequestLocator;
use std::future::Future;
use std::pin::Pin;

/// The idempotency marker every post for this rule carries.
///
/// A plain HTML comment, invisible when the comment is rendered on GitHub, so the operator-
/// facing body can read as prose while a later reactor run scanning existing comments can
/// still find it with a substring check. Rule-specific so another rule's post is not treated
/// as this rule's.
pub const RULE_MARKER: &str = "<!-- contributron-rule:threads-cleared -->";

/// The `agent_id` written to the store for every reaction this rule produces.
///
/// Invariant 6 requires every operation to name three identities; this is the stable literal
/// for *which deterministic rule ran*. A later rule gets its own.
pub const RULE_AGENT_ID: &str = "rule:threads-cleared";

/// The idempotency marker for slice 0015's merge-conflicts-cleared rule.
pub const MERGE_RULE_MARKER: &str = "<!-- contributron-rule:merge-conflicts-cleared -->";

/// The `agent_id` written to the store for every reaction the merge-conflicts-cleared rule
/// produces (slice 0015).
pub const MERGE_RULE_AGENT_ID: &str = "rule:merge-conflicts-cleared";

/// A comment this rule would post, or did post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentBody {
    /// The full body as it would appear on the pull request, including the marker.
    pub body: String,
}

impl CommentBody {
    /// The body this rule produces for a qualifying transition. The marker goes first so
    /// the substring scan is positional-stable: a later edit of the human-readable prose
    /// does not move where the marker lives.
    #[must_use]
    pub fn for_threads_cleared() -> Self {
        Self {
            body: format!(
                "{RULE_MARKER}\nAll review threads on this pull request are now resolved."
            ),
        }
    }

    /// The body produced for a qualifying conflicts-cleared transition (slice 0015).
    #[must_use]
    pub fn for_merge_conflicts_cleared() -> Self {
        Self {
            body: format!(
                "{MERGE_RULE_MARKER}\nMerge conflicts on this pull request are now resolved."
            ),
        }
    }
}

/// An existing comment on a pull request, as the reactor cares about one.
///
/// Carries only the fields the rule uses: the body. The provider's identifier, author, and
/// timestamps are not read by this rule, so pulling them across the trait boundary would be
/// speculative generality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingComment {
    /// The comment's full body.
    pub body: String,
}

/// The future a [`ProviderClient`] method returns.
pub type ClientFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

/// Something a provider client could not do.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Any transport, authentication, rate-limit or GraphQL/REST error from the client.
    #[error("{0}")]
    Client(String),
}

/// What the reactor needs from whichever provider is on the other end.
///
/// A trait rather than a concrete type so the rule is testable without a network and so a
/// second provider adapter (the GitLab ledger item) can implement it without reshaping
/// this crate. Two methods, no more: everything invariant 4 requires is either already in
/// the observation this rule was called with, or expressly out of scope (mergeability,
/// approval, branch protection).
pub trait ProviderClient: Send + Sync {
    /// List the comments on a pull request, as the reactor needs them (body only).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails.
    fn list_comments<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<ExistingComment>>;

    /// Post one comment on a pull request. Returns `Ok(())` when the provider accepted it.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails.
    fn post_comment<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        body: &'a str,
    ) -> ClientFuture<'a, ()>;

    /// The invariant-4 live read, specialized for the conflicts-cleared rule: does the
    /// pull request *right now* have any merge conflict?
    ///
    /// Returns [`Ok(false)`] when the provider confirms the merge is NOT conflicting (the
    /// non-dirty settled states: a `clean`/`unstable`/`has_hooks`/`blocked`/`behind` pull request
    /// all have the merge computed and free of conflicts, so the rule may post). Returns
    /// [`Ok(true)`] when the provider says the merge still has a conflict (dirty). For any
    /// state the provider has not yet settled on (its internal unknown/computing state,
    /// or a draft/null the REST payload does not pin down), returns a
    /// [`ProviderError::Client`] so the caller's retry loop can park the observation and
    /// try again on the next tick. The method is deliberately not a bool: a false positive
    /// here (treating "still computing" as "no conflict") would let the rule post on a
    /// pull request whose conflict was in fact still present, and silently losing the
    /// transition on a transient computing state (finding 2 of the 2026-11-25 review)
    /// would miss the common GitHub path.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails OR if the provider has not
    /// yet settled on an answer; both are routed through the reactor's retry path.
    fn live_merge_has_conflict<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool>;
}

/// What a reaction call decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionOutcome {
    /// The prior/new pair did not cross the transition; nothing was done.
    NotApplicable,
    /// The marker was already present on the pull request; nothing was posted.
    AlreadyPosted,
    /// A comment was posted. The attribution recorded alongside.
    Posted {
        /// Who the store should record the write under (invariant 6).
        attribution: Attribution,
        /// What was posted.
        comment: CommentBody,
    },
}

/// Something the reactor could not do.
#[derive(Debug, thiserror::Error)]
pub enum ReactError {
    /// The provider client failed.
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
    /// Reading the observation history failed (slice 0015 looks past the immediate prior
    /// to resolve the `Conflicting → Unknown → Mergeable` chain: finding 1 of the
    /// 2026-11-25 review).
    #[error("history: {0}")]
    History(String),
}

/// Access to settled prior mergeable observations past the immediate `prior` the poll
/// loop caches in memory. Needed because a GitHub `mergeable: null` tick normalizes to
/// [`Mergeable::Unknown`], which the poll loop records as the new in-memory prior; the
/// stored history still carries the earlier `Conflicting` row, and that is the evidence
/// the rule has to walk back to.
pub trait MergeableHistory: Send + Sync {
    /// The most recent stored mergeable observation for this pull request whose value is
    /// not [`Mergeable::Unknown`]. Returns `Ok(None)` when the store has no such row.
    ///
    /// # Errors
    ///
    /// Returns a reader-defined message when the store query failed. The react crate
    /// wraps that into [`ReactError::History`] without reinterpreting it.
    fn latest_settled(&self, locator: &PullRequestLocator) -> Result<Option<Mergeable>, String>;
}

/// Examines a pair of observations and decides whether to react.
///
/// `prior` is the most recent observation of the same `(owner, number)` before `new`, or
/// `None` if `new` is the first observation we hold. The rule fires only when prior is
/// `Some` and its unresolved count is non-zero *and* `new`'s unresolved count is zero.
///
/// The idempotency check runs before any post: the provider is asked for existing comments,
/// and if any carries [`RULE_MARKER`] the reactor returns [`ReactionOutcome::AlreadyPosted`]
/// without posting. One extra provider call per qualifying transition, in exchange for
/// surviving restarts and store wipes without a new schema column.
///
/// # Errors
///
/// Returns [`ReactError::Provider`] if listing existing comments or posting failed.
pub async fn react(
    owner: &UserId,
    worker: &WorkerId,
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    client: &dyn ProviderClient,
) -> Result<ReactionOutcome, ReactError> {
    if !is_qualifying_transition(prior, new) {
        return Ok(ReactionOutcome::NotApplicable);
    }

    let existing = client.list_comments(locator).await?;
    if existing.iter().any(|c| c.body.contains(RULE_MARKER)) {
        return Ok(ReactionOutcome::AlreadyPosted);
    }

    let comment = CommentBody::for_threads_cleared();
    client.post_comment(locator, &comment.body).await?;

    Ok(ReactionOutcome::Posted {
        attribution: Attribution {
            user: owner.clone(),
            worker: worker.clone(),
            // The `rule:` prefix in the id names this as a deterministic rule rather than
            // an AI agent; the enum carries both because the store writes one `agent`
            // column (invariant 6). A later ADR earns a distinct variant if a second
            // deterministic rule ships.
            agent: ExecutingAgent::Agent {
                id: AgentId::new(RULE_AGENT_ID),
            },
        },
        comment,
    })
}

/// Transition detector, split out so the three "no fire" scenarios can be read at a glance.
fn is_qualifying_transition(
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
) -> bool {
    let Some(prior) = prior else {
        // Scenario 3 of slice 0014: first observation, no transition to detect.
        return false;
    };
    // Scenario 2: steady-state zero does not fire.
    prior.unresolved_thread_count() > 0 && new.unresolved_thread_count() == 0
}

/// Slice 0015 rule: when a pull request's stored mergeability transitions from
/// `Conflicting` to `Mergeable`, re-fetch the provider's authoritative state and post one
/// marker comment if the live read confirms the transition.
///
/// The stored transition is evidence; the live `is_mergeable_clean_now` call is authority
/// (standing invariant 4, ADR-0003). A disagreement between them returns
/// [`ReactionOutcome::NotApplicable`], never a post: the payload is a lagging snapshot and
/// posting on it would call a pull request mergeable at the moment it is not.
///
/// Idempotency follows slice 0014's shape: list existing comments and look for
/// [`MERGE_RULE_MARKER`] before posting. Reuses the same [`ProviderClient::list_comments`]
/// surface, so the slice-0014 pagination gap ledgered on it covers this rule too.
///
/// # Errors
///
/// Returns [`ReactError::Provider`] if the live-mergeability read, listing comments, or
/// posting a comment failed.
pub async fn react_merge_conflicts_cleared(
    owner: &UserId,
    worker: &WorkerId,
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
    client: &dyn ProviderClient,
) -> Result<ReactionOutcome, ReactError> {
    if !is_merge_conflicts_cleared_transition(locator, prior, new, history)? {
        return Ok(ReactionOutcome::NotApplicable);
    }

    // Invariant 4 bite: the stored transition is evidence, this live read is authority.
    // A `Client(...)` error from the provider (including "still computing") bubbles up as
    // [`ReactError::Provider`], which the CLI reactor closure turns into
    // `ReactionReport::Failed` — the poll loop parks the observation and retries on the
    // next tick, so the common GitHub pattern of `Conflicting → Unknown → Mergeable` (the
    // REST payload lagging behind GraphQL) no longer silently loses the transition
    // (finding 2 of the 2026-11-25 review).
    if client.live_merge_has_conflict(locator).await? {
        // Payload said the conflict cleared; the live provider still reports it dirty.
        // Return `NotApplicable` rather than `Failed`: this is genuine disagreement, not
        // a transient non-answer, so a park-and-retry would just re-read the same dirty
        // state each tick. The next Fresh observation (when the conflict really clears)
        // will drive another attempt.
        return Ok(ReactionOutcome::NotApplicable);
    }

    let existing = client.list_comments(locator).await?;
    if existing.iter().any(|c| c.body.contains(MERGE_RULE_MARKER)) {
        return Ok(ReactionOutcome::AlreadyPosted);
    }

    let comment = CommentBody::for_merge_conflicts_cleared();
    client.post_comment(locator, &comment.body).await?;

    Ok(ReactionOutcome::Posted {
        attribution: Attribution {
            user: owner.clone(),
            worker: worker.clone(),
            agent: ExecutingAgent::Agent {
                id: AgentId::new(MERGE_RULE_AGENT_ID),
            },
        },
        comment,
    })
}

/// Qualifying transition for the merge-conflicts-cleared rule: the new observation
/// records `Mergeable`, and the most recent stored *settled* mergeable (one that is not
/// [`Mergeable::Unknown`]) recorded `Conflicting`.
///
/// Walking the history past the immediate `prior` is what fixes the `Conflicting →
/// Unknown → Mergeable` miss (finding 1 of the 2026-11-25 review): the poll loop
/// overwrites `prior` on every `NotApplicable`, so a single `Unknown` tick erases the
/// Conflicting evidence from memory. The store still carries the earlier row, and that
/// is what the rule reads here.
fn is_merge_conflicts_cleared_transition(
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
) -> Result<bool, ReactError> {
    if new.mergeable != Mergeable::Mergeable {
        return Ok(false);
    }
    // Cheap path: the in-memory prior already carries a settled Conflicting state, so
    // the store call is skipped. Rules of this type run every tick; a store read per
    // tick on the hot path would cost nothing visible but saving it is cheap.
    if let Some(prior) = prior {
        if prior.mergeable == Mergeable::Conflicting {
            return Ok(true);
        }
        if prior.mergeable == Mergeable::Mergeable {
            return Ok(false);
        }
    }
    // Immediate prior is either absent or `Unknown`. Walk back through the store.
    let settled = history
        .latest_settled(locator)
        .map_err(ReactError::History)?;
    Ok(settled == Some(Mergeable::Conflicting))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{CommentBody, RULE_AGENT_ID, RULE_MARKER, is_qualifying_transition};
    use contributron_domain::pull_request::{
        Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber, ReviewThread,
        ThreadId, ThreadResolution,
    };

    fn pr(unresolved: usize, resolved: usize) -> ObservedPullRequest {
        let mut threads = Vec::new();
        for i in 0..unresolved {
            threads.push(ReviewThread {
                id: ThreadId::new(format!("u-{i}")).expect("id"),
                resolution: ThreadResolution::Unresolved,
            });
        }
        for i in 0..resolved {
            threads.push(ReviewThread {
                id: ThreadId::new(format!("r-{i}")).expect("id"),
                resolution: ThreadResolution::Resolved,
            });
        }
        ObservedPullRequest::new(
            PullRequestNumber::new(1).expect("number"),
            "example",
            false,
            PullRequestAuthor::DeletedAccount,
            Mergeable::Unknown,
            threads,
        )
    }

    #[test]
    fn body_carries_the_marker() {
        let body = CommentBody::for_threads_cleared();
        assert!(body.body.contains(RULE_MARKER), "was: {}", body.body);
    }

    #[test]
    fn transition_fires_on_two_to_zero() {
        assert!(is_qualifying_transition(Some(&pr(2, 0)), &pr(0, 2)));
    }

    #[test]
    fn steady_state_zero_does_not_fire() {
        assert!(!is_qualifying_transition(Some(&pr(0, 0)), &pr(0, 0)));
    }

    #[test]
    fn first_observation_with_zero_does_not_fire() {
        assert!(!is_qualifying_transition(None, &pr(0, 0)));
    }

    #[test]
    fn same_non_zero_to_same_non_zero_does_not_fire() {
        assert!(!is_qualifying_transition(Some(&pr(2, 0)), &pr(2, 0)));
    }

    #[test]
    fn going_up_does_not_fire() {
        assert!(!is_qualifying_transition(Some(&pr(0, 1)), &pr(1, 0)));
    }

    #[test]
    fn agent_id_literal_matches_slice_frontmatter() {
        assert_eq!(RULE_AGENT_ID, "rule:threads-cleared");
    }
}
