//! Deterministic reactions to observed state transitions (slice 0014, slice 0015, slice 0016).
//!
//! Three rules ship today:
//!
//! - `react`: threads-cleared (slice 0014). Post one marker comment when the unresolved
//!   review-thread count transitions from non-zero to zero.
//! - `react_merge_conflicts_cleared` (slice 0015). Post one marker comment when the stored
//!   mergeable state transitions from `Conflicting` to `Mergeable`, gated on a live
//!   provider re-fetch.
//! - `react_needs_rebase_label` (slice 0016). Apply or remove the `needs-rebase` label
//!   when the stored mergeable state crosses the Mergeable/Conflicting boundary in either
//!   direction, gated on the same live provider re-fetch. Label-level idempotency is
//!   `list_labels` plus a set membership check, so no marker comment is needed.
//!
//! This crate holds the pure logic. The provider client is a trait, not a concrete octocrab
//! client, so every rule is testable without a network and the GitHub-specific wiring
//! stays in `crates/github/` (invariant 1).

use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::pull_request::{Label, Mergeable, ObservedPullRequest};
use contributron_poll::{PullRequestLocator, ReactionReport};
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

/// The `agent_id` written to the store for the needs-rebase-label rule (slice 0016).
pub const LABEL_RULE_AGENT_ID: &str = "rule:needs-rebase-label";

/// The label the needs-rebase-label rule applies and removes (slice 0016).
///
/// A string constant here, wrapped into a validated [`Label`] on use. Configurability is
/// deferred: three workflows before a DSL (ADR-0004).
pub const NEEDS_REBASE_LABEL: &str = "needs-rebase";

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
/// A trait rather than a concrete type so the rules are testable without a network and so a
/// second provider adapter (the GitLab ledger item) can implement it without reshaping
/// this crate.
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

    /// The invariant-4 live read, specialized for the conflicts-cleared and
    /// needs-rebase-label rules: does the pull request *right now* have any merge conflict?
    ///
    /// Returns `Ok(false)` when the provider confirms the merge is NOT conflicting (the
    /// non-dirty settled states: a `clean`/`unstable`/`has_hooks`/`blocked`/`behind` pull
    /// request all have the merge computed and free of conflicts). Returns `Ok(true)` when
    /// the provider says the merge still has a conflict (dirty). For any state the provider
    /// has not yet settled on (its internal unknown/computing state, or a draft/null the REST
    /// payload does not pin down), returns a [`ProviderError::Client`] so the caller's retry
    /// path can park the observation and try again on the next tick.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails OR if the provider has not
    /// yet settled on an answer; both are routed through the reactor's retry path.
    fn live_merge_has_conflict<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool>;

    /// List the labels currently attached to a pull request (slice 0016).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails.
    fn list_labels<'a>(&'a self, locator: &'a PullRequestLocator) -> ClientFuture<'a, Vec<Label>>;

    /// Add `label` to a pull request (slice 0016). Idempotent on the provider side, but the
    /// caller is still expected to short-circuit on an already-present label so the
    /// `AlreadySettled` outcome is distinct from a real write.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails.
    fn add_label<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()>;

    /// Remove `label` from a pull request (slice 0016).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the provider call fails.
    fn remove_label<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()>;
}

/// One concrete write a rule produced (slice 0016).
///
/// A single rule produces either one receipt (`Posted`) or none (`NotApplicable` or
/// `AlreadySettled`). The CLI's combiner concatenates receipts across rules so a tick that
/// posts a comment *and* removes a label records both writes with their own attributions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionReceipt {
    /// Who the store should record the write under (invariant 6).
    pub attribution: Attribution,
    /// What the rule did.
    pub kind: ReactionKind,
}

/// The shape of a single write (slice 0016).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionKind {
    /// A comment was posted.
    Commented {
        /// The posted body (marker plus prose).
        body: String,
    },
    /// A label was applied.
    LabelApplied {
        /// Which label.
        label: Label,
    },
    /// A label was removed.
    LabelRemoved {
        /// Which label.
        label: Label,
    },
}

/// What a reaction call decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionOutcome {
    /// The prior/new pair did not cross the transition; nothing was done.
    NotApplicable,
    /// The desired post-write state was already in place (marker present for a comment
    /// rule; label already applied or already absent for the label rule). No new write;
    /// invariant 6 is satisfied by the existing state the rule chose not to disturb.
    AlreadySettled,
    /// A write happened. The receipt names what and who.
    Posted {
        /// What was written.
        receipt: ReactionReceipt,
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
/// stored history still carries the earlier settled row, and that is the evidence the
/// rules walk back to.
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

/// Build the standard three-identity [`Attribution`] a rule records with every write.
fn attribution_for(owner: &UserId, worker: &WorkerId, agent_id: &str) -> Attribution {
    Attribution {
        user: owner.clone(),
        worker: worker.clone(),
        agent: ExecutingAgent::Agent {
            id: AgentId::new(agent_id),
        },
    }
}

/// Examines a pair of observations and decides whether to react (slice 0014).
///
/// `prior` is the most recent observation of the same `(owner, number)` before `new`, or
/// `None` if `new` is the first observation we hold. The rule fires only when prior is
/// `Some` and its unresolved count is non-zero *and* `new`'s unresolved count is zero.
///
/// The idempotency check runs before any post: the provider is asked for existing comments,
/// and if any carries [`RULE_MARKER`] the reactor returns [`ReactionOutcome::AlreadySettled`]
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
        return Ok(ReactionOutcome::AlreadySettled);
    }

    let comment = CommentBody::for_threads_cleared();
    client.post_comment(locator, &comment.body).await?;

    Ok(ReactionOutcome::Posted {
        receipt: ReactionReceipt {
            attribution: attribution_for(owner, worker, RULE_AGENT_ID),
            kind: ReactionKind::Commented { body: comment.body },
        },
    })
}

/// Transition detector, split out so the three "no fire" scenarios can be read at a glance.
fn is_qualifying_transition(
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
) -> bool {
    let Some(prior) = prior else {
        return false;
    };
    prior.unresolved_thread_count() > 0 && new.unresolved_thread_count() == 0
}

/// Slice 0015 rule: when a pull request's stored mergeability transitions from
/// `Conflicting` to `Mergeable`, re-fetch the provider's authoritative state and post one
/// marker comment if the live read confirms the transition.
///
/// # Errors
///
/// Returns [`ReactError::Provider`] if the live-mergeability read, listing comments, or
/// posting a comment failed. Returns [`ReactError::History`] if the history walk failed.
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

    if client.live_merge_has_conflict(locator).await? {
        return Ok(ReactionOutcome::NotApplicable);
    }

    let existing = client.list_comments(locator).await?;
    if existing.iter().any(|c| c.body.contains(MERGE_RULE_MARKER)) {
        return Ok(ReactionOutcome::AlreadySettled);
    }

    let comment = CommentBody::for_merge_conflicts_cleared();
    client.post_comment(locator, &comment.body).await?;

    Ok(ReactionOutcome::Posted {
        receipt: ReactionReceipt {
            attribution: attribution_for(owner, worker, MERGE_RULE_AGENT_ID),
            kind: ReactionKind::Commented { body: comment.body },
        },
    })
}

/// Qualifying transition for the merge-conflicts-cleared rule: the new observation records
/// `Mergeable`, and the most recent stored *settled* mergeable recorded `Conflicting`.
fn is_merge_conflicts_cleared_transition(
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
) -> Result<bool, ReactError> {
    if new.mergeable != Mergeable::Mergeable {
        return Ok(false);
    }
    if let Some(prior) = prior {
        if prior.mergeable == Mergeable::Conflicting {
            return Ok(true);
        }
        if prior.mergeable == Mergeable::Mergeable {
            return Ok(false);
        }
    }
    let settled = history
        .latest_settled(locator)
        .map_err(ReactError::History)?;
    Ok(settled == Some(Mergeable::Conflicting))
}

/// Which direction of the Mergeable/Conflicting transition the label rule is reacting to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelTransition {
    /// Mergeable moved to Conflicting: apply the `needs-rebase` label.
    Apply,
    /// Conflicting moved to Mergeable: remove the `needs-rebase` label.
    Remove,
}

/// Slice 0016 rule: when a pull request's stored mergeability crosses the
/// Mergeable/Conflicting boundary in either direction, re-fetch the provider's
/// authoritative state and either apply or remove the `needs-rebase` label.
///
/// Idempotency is label-level: `list_labels` reports what is currently attached, and the
/// rule short-circuits to [`ReactionOutcome::AlreadySettled`] when the desired post-write
/// state already holds. No marker comment is used; the label set is the source of truth.
///
/// # Errors
///
/// Returns [`ReactError::Provider`] if the live-mergeability read, listing labels, or
/// applying/removing a label failed. Returns [`ReactError::History`] if the history walk
/// failed.
pub async fn react_needs_rebase_label(
    owner: &UserId,
    worker: &WorkerId,
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
    client: &dyn ProviderClient,
) -> Result<ReactionOutcome, ReactError> {
    let Some(transition) = needs_rebase_transition(locator, prior, new, history)? else {
        return Ok(ReactionOutcome::NotApplicable);
    };

    let live_conflict = client.live_merge_has_conflict(locator).await?;
    let expected_conflict = match transition {
        LabelTransition::Apply => true,
        LabelTransition::Remove => false,
    };
    if live_conflict != expected_conflict {
        // Payload says we transitioned, live read disagrees. Mirrors slice 0015's
        // disagreement handling: not a retryable non-answer, so `NotApplicable` rather than
        // `ReactError::Provider`.
        return Ok(ReactionOutcome::NotApplicable);
    }

    let label = needs_rebase_label();
    let currently_present = client
        .list_labels(locator)
        .await?
        .into_iter()
        .any(|l| l == label);

    match transition {
        LabelTransition::Apply => {
            if currently_present {
                return Ok(ReactionOutcome::AlreadySettled);
            }
            client.add_label(locator, &label).await?;
            Ok(ReactionOutcome::Posted {
                receipt: ReactionReceipt {
                    attribution: attribution_for(owner, worker, LABEL_RULE_AGENT_ID),
                    kind: ReactionKind::LabelApplied { label },
                },
            })
        }
        LabelTransition::Remove => {
            if !currently_present {
                return Ok(ReactionOutcome::AlreadySettled);
            }
            client.remove_label(locator, &label).await?;
            Ok(ReactionOutcome::Posted {
                receipt: ReactionReceipt {
                    attribution: attribution_for(owner, worker, LABEL_RULE_AGENT_ID),
                    kind: ReactionKind::LabelRemoved { label },
                },
            })
        }
    }
}

/// The canonical [`Label`] value for the needs-rebase rule. Panics are impossible here
/// because [`NEEDS_REBASE_LABEL`] is a trimmed non-empty literal: a `const` cannot run the
/// validator, so the first call to this function is the check.
#[allow(clippy::expect_used)]
fn needs_rebase_label() -> Label {
    Label::new(NEEDS_REBASE_LABEL).expect("NEEDS_REBASE_LABEL is a valid Label by construction")
}

/// Qualifying transition for the needs-rebase-label rule: either Mergeable→Conflicting
/// (Apply) or Conflicting→Mergeable (Remove). Uses the history-walk bridge when the
/// in-memory prior is `Unknown` or absent, mirroring slice 0015.
fn needs_rebase_transition(
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
) -> Result<Option<LabelTransition>, ReactError> {
    let settled_prior = match prior.map(|p| p.mergeable) {
        Some(Mergeable::Conflicting) => Some(Mergeable::Conflicting),
        Some(Mergeable::Mergeable) => Some(Mergeable::Mergeable),
        Some(Mergeable::Unknown) | None => history
            .latest_settled(locator)
            .map_err(ReactError::History)?,
    };
    match (settled_prior, new.mergeable) {
        (Some(Mergeable::Mergeable), Mergeable::Conflicting) => Ok(Some(LabelTransition::Apply)),
        (Some(Mergeable::Conflicting), Mergeable::Mergeable) => Ok(Some(LabelTransition::Remove)),
        _ => Ok(None),
    }
}

/// Fold a slice of per-rule results into the single [`ReactionReport`] the poll loop
/// carries per tick (slice 0016).
///
/// Every rule's outcome is independent: a rule that posted and a rule that errored coexist
/// in the same report, with the write's `agent_id` in `agent_ids` and the error's
/// `agent_id` in `errors`. Classification into `Posted` / `AlreadySettled` /
/// `NotApplicable` / `Failed` follows the highest-value outcome: at least one write =>
/// `Posted`; else at least one idempotent settle => `AlreadySettled`; else every rule
/// errored => `Failed`; else `NotApplicable`.
///
/// Each entry pairs a *fallback* agent id (the rule's stable literal, used when a rule
/// errored before producing a receipt or when a `NoAgent` attribution somehow reached the
/// combiner) with that rule's `Result`. The combiner is total on the input: an empty slice
/// reports `NotApplicable`.
#[must_use]
pub fn combine_rule_results(
    results: &[(&'static str, Result<ReactionOutcome, ReactError>)],
) -> ReactionReport {
    let mut posted_ids: Vec<String> = Vec::new();
    let mut settled_ids: Vec<String> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();

    for (fallback_id, outcome) in results {
        match outcome {
            Ok(ReactionOutcome::Posted { receipt }) => {
                posted_ids.push(agent_id_of(receipt, fallback_id));
            }
            Ok(ReactionOutcome::AlreadySettled) => {
                settled_ids.push((*fallback_id).to_owned());
            }
            Ok(ReactionOutcome::NotApplicable) => {}
            Err(e) => {
                errors.push(((*fallback_id).to_owned(), format!("{e}")));
            }
        }
    }

    if !posted_ids.is_empty() {
        return ReactionReport::Posted {
            agent_ids: posted_ids,
            errors,
        };
    }
    if !settled_ids.is_empty() {
        return ReactionReport::AlreadySettled {
            agent_ids: settled_ids,
            errors,
        };
    }
    if !errors.is_empty() {
        let aggregated = errors
            .into_iter()
            .map(|(id, msg)| format!("{id}: {msg}"))
            .collect::<Vec<_>>()
            .join("; ");
        return ReactionReport::Failed { error: aggregated };
    }
    ReactionReport::NotApplicable
}

fn agent_id_of(receipt: &ReactionReceipt, fallback: &str) -> String {
    match &receipt.attribution.agent {
        ExecutingAgent::Agent { id } => id.as_str().to_owned(),
        ExecutingAgent::NoAgent => fallback.to_owned(),
    }
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
