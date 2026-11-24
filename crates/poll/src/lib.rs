//! Polling loop for pull-request observation (slice 0011, ADR-0012).
//!
//! Given a list of pull requests and a [`Fetch`] the caller provides, the loop polls each
//! target on its own cadence, writes the resulting observation to the durable store
//! (slice 0009), and keeps running until a shutdown signal arrives. Dedup across unchanged
//! polls is the store's job; the loop's job is cadence, fairness across targets, backoff on
//! error and clean shutdown.
//!
//! No provider crate is a dependency. The GitHub adapter becomes a `Fetch` at wire time, in
//! the CLI; a future second adapter becomes another one. This is the same argument
//! [ADR-0001](../../../adr/0001-github-is-the-only-provider-adapter-in-the-mvp.md) makes
//! for the provider boundary, applied to the scheduler.

use contributron_domain::actor::{Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_domain::pull_request::ObservedPullRequest;
use contributron_store::{EventStore, StoreError, WriteOutcome};
use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// The worker string every loop-produced row carries in its attribution. Distinct from the
/// CLI's `"cli"` so an audit record can tell a loop-initiated write from an operator one
/// without inferring it from the subcommand (invariant 6, ADR-0005).
pub const POLL_WORKER: &str = "poll";

/// Shift cap for the backoff doubler: `2^6 = 64`, so a target failing continuously is
/// polled at most once per `64 * interval`. At the default sixty-second interval that is
/// roughly an hour, which is slow enough not to hammer a provider and fast enough that a
/// transient outage of any plausible duration recovers without operator action.
const MAX_BACKOFF_SHIFT: u32 = 6;

/// One pull request to watch.
///
/// Shape mirrors the GitHub adapter's locator, but this crate deliberately does not import
/// that type: the dependency direction (invariant 1) is adapter → poll only via the
/// [`Fetch`] closure.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PullRequestLocator {
    /// The repository's owner, as the provider reports it.
    pub owner: String,
    /// The repository's name.
    pub repo: String,
    /// The pull request number within that repository.
    pub number: u64,
}

impl std::fmt::Display for PullRequestLocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}#{}", self.owner, self.repo, self.number)
    }
}

/// The future a [`Fetch`] returns.
pub type FetchFuture =
    Pin<Box<dyn Future<Output = Result<ObservedPullRequest, FetchError>> + Send>>;

/// One fetch of a pull request's current state.
///
/// A boxed `Fn` rather than a trait because exactly one real implementation exists today
/// (the GitHub adapter wired by the CLI) plus a test double; the quality bar's rule of
/// three says the second *real* implementation is the one that earns an abstraction. The
/// ledger carries that second adapter as the slice that earns a trait here.
pub type Fetch = Arc<dyn Fn(PullRequestLocator) -> FetchFuture + Send + Sync>;

/// A fetch failed. The store is not touched, the backoff advances, the loop continues.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct FetchError(pub String);

/// Something the loop could not do that is not a fetch failure.
#[derive(Debug, thiserror::Error)]
pub enum PollError {
    /// Opening or writing the store failed.
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// A spawned polling task panicked.
    ///
    /// The quality bar forbids panics in production code, so this variant exists so the
    /// top-level loop surfaces a panic rather than silently drops the target it came from.
    #[error("a polling task joined with a panic: {0}")]
    TaskJoined(String),
}

/// What happened on one tick, reported in order on the channel [`run_loop`] drains to.
#[derive(Debug, Clone)]
pub struct Tick {
    /// Which target the tick was for.
    pub locator: PullRequestLocator,
    /// What happened.
    pub outcome: TickOutcome,
    /// What the reactor decided this tick, when one was attached (slice 0014). `None` when
    /// no reactor was wired (every `run_loop` tick, and every `run_loop_with_reactor` tick
    /// whose outcome did not reach the reactor: a fetch or store failure, or an unchanged
    /// observation, since there is no new state for the reactor to compare against).
    pub reaction: Option<ReactionReport>,
}

/// The four outcomes of one tick.
#[derive(Debug, Clone)]
pub enum TickOutcome {
    /// The store accepted a new row.
    Fresh,
    /// The store already held a row for this identity: re-observing the same state is not
    /// a new fact (ADR-0008), reported so the caller can log it.
    Unchanged,
    /// The fetch returned an error. The next attempt is after `backoff`.
    FetchFailed {
        /// The error the [`Fetch`] produced.
        error: FetchError,
        /// How long until the next attempt for this target.
        backoff: Duration,
    },
    /// The store could not open or could not accept the row for this tick.
    ///
    /// Treated as an operational event rather than a loop-terminating error: the operator
    /// sees it on stderr in real time via the channel, the per-target backoff advances
    /// just as for a fetch failure, and the loop keeps running so a transient condition
    /// (SQLite busy, a brief IO glitch) resolves on retry. A store-open failure at task
    /// startup is final for that target; the task reports the error once and then exits
    /// cleanly, which [`run_loop`] surfaces as Ok because other targets still work.
    StoreFailed {
        /// The error the store produced.
        error: String,
        /// How long until the next attempt for this target, or `None` if the task is
        /// exiting because the store could not be opened at all.
        backoff: Option<Duration>,
    },
}

/// The future a [`Reactor`] returns.
pub type ReactorFuture = Pin<Box<dyn Future<Output = ReactionReport> + Send>>;

/// Called after a fresh observation is written to the store (slice 0014). The reactor is
/// handed the prior observation held in memory by the task (or `None` for the first fresh
/// tick in this process's lifetime), the newly-written observation, and the locator. What
/// it does on the other side is the reactor crate's concern; this crate only forwards the
/// decision into the tick stream.
///
/// `Arc<dyn Fn ...>` rather than a trait because the shape mirrors [`Fetch`] and the quality
/// bar's rule of three says one real implementation does not earn a trait.
pub type Reactor = Arc<
    dyn Fn(PullRequestLocator, Option<ObservedPullRequest>, ObservedPullRequest) -> ReactorFuture
        + Send
        + Sync,
>;

/// What the reactor decided for one tick.
///
/// Kept as plain strings and variants rather than the reactor crate's own `ReactionOutcome`
/// so the dependency direction stays `react -> poll` only — this crate does not know the
/// shape of a provider attribution, only that a reactor returned a decision.
#[derive(Debug, Clone)]
pub enum ReactionReport {
    /// The prior/new pair did not qualify; nothing was done.
    NotApplicable,
    /// A prior post for this rule was already visible on the target; nothing was posted.
    AlreadyPosted,
    /// A comment was posted. `agent_id` names which rule ran (invariant 6).
    Posted {
        /// The rule's `agent_id`, as the reactor reported it.
        agent_id: String,
    },
    /// The reactor failed. The error is surfaced on the tick stream and the loop continues;
    /// per slice 0014 the write-path failure does not halt the read-path (the next tick's
    /// fetch and store write still run).
    Failed {
        /// The reactor's error, stringified. The reactor is responsible for not leaking a
        /// credential into this string.
        error: String,
    },
}

/// Settings every polling task shares.
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// Targets to poll.
    pub targets: Vec<PullRequestLocator>,
    /// The base poll interval.
    ///
    /// Each target's actual delay is the interval plus its startup jitter, and after a
    /// failed tick the delay becomes `interval * 2.pow(min(consecutive_failures - 1,
    /// MAX_BACKOFF_SHIFT))`. A successful tick resets the counter. The policy is recorded
    /// in [ADR-0012](../../../adr/0012-polling-cadence-is-per-target-interval-with-jitter-and-backoff.md).
    pub interval: Duration,
    /// The operator the store is scoped to (invariant 7).
    pub owner: UserId,
}

/// Runs the polling loop until `cancel` reports `true` or its sender is dropped.
///
/// Spawns one task per target, each with its own [`EventStore`] connection to the file at
/// `store_path`. Every tick's outcome is sent on `events`; the caller is responsible for
/// draining the receiver. The CLI writes one stdout line per tick; tests collect into a
/// `Vec`. The function returns when every spawned task has finished.
///
/// # Errors
///
/// Returns [`PollError::Store`] if any task could not open the store or write a row, and
/// [`PollError::TaskJoined`] if a spawned task panicked. Only the first such error is
/// returned; later ones are discarded because any of them are already a bug worth stopping
/// on.
pub async fn run_loop(
    config: PollConfig,
    fetch: Fetch,
    store_path: &Path,
    events: mpsc::UnboundedSender<Tick>,
    cancel: watch::Receiver<bool>,
) -> Result<(), PollError> {
    run_loop_with_reactor(config, fetch, None, store_path, events, cancel).await
}

/// Same as [`run_loop`] but with a reactor attached (slice 0014). After each fresh store
/// write the task consults `reactor` with the prior observation and the new one; the
/// resulting [`ReactionReport`] is attached to the tick. Passing `None` is equivalent to
/// [`run_loop`].
///
/// The prior is seeded at task startup from [`EventStore::latest_for_number`], so a
/// transition whose two halves were observed in different process lifetimes is still seen.
/// A failed reactor call parks the new observation as pending; the next tick's reactor call
/// uses the same unchanged prior and retries with the parked new, so a transient provider
/// error no longer silently drops the transition (both findings from the 2026-11-24
/// adversarial review).
///
/// # Errors
///
/// See [`run_loop`].
pub async fn run_loop_with_reactor(
    config: PollConfig,
    fetch: Fetch,
    reactor: Option<Reactor>,
    store_path: &Path,
    events: mpsc::UnboundedSender<Tick>,
    cancel: watch::Receiver<bool>,
) -> Result<(), PollError> {
    let mut set = tokio::task::JoinSet::new();
    for target in &config.targets {
        let jitter = jitter_for(target, config.interval);
        let state = TaskState {
            locator: target.clone(),
            fetch: Arc::clone(&fetch),
            reactor: reactor.as_ref().map(Arc::clone),
            interval: config.interval,
            jitter,
            owner: config.owner.clone(),
            store_path: store_path.to_path_buf(),
            events: events.clone(),
            cancel: cancel.clone(),
        };
        set.spawn(run_one(state));
    }
    // Dropping this crate's handle on the sender means that once every spawned task
    // finishes, the receiver the caller holds sees a closed channel and can stop draining.
    // The clones inside each task keep the channel alive while the task runs.
    drop(events);

    let mut first: Option<PollError> = None;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first.is_none() {
                    first = Some(e);
                }
            }
            Err(join_err) => {
                if first.is_none() {
                    first = Some(PollError::TaskJoined(join_err.to_string()));
                }
            }
        }
    }
    match first {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct TaskState {
    locator: PullRequestLocator,
    fetch: Fetch,
    reactor: Option<Reactor>,
    interval: Duration,
    jitter: Duration,
    owner: UserId,
    store_path: PathBuf,
    events: mpsc::UnboundedSender<Tick>,
    cancel: watch::Receiver<bool>,
}

async fn run_one(mut state: TaskState) -> Result<(), PollError> {
    // A fresh `watch::Receiver` treats its initial value as unseen, so `changed()` would
    // resolve immediately on the first call. Marking it seen up front means the `select!`
    // below races the sleep against an *actual* change, not against noise.
    state.cancel.mark_unchanged();

    let Some((store, mut prior)) = open_store_and_seed_prior(&state) else {
        return Ok(());
    };
    // A fresh observation whose reactor call returned `Failed`, parked here so the next tick
    // can retry the reactor against the same unchanged prior. Cleared on reactor success, on
    // any subsequent Fresh (the world moved on; the parked observation is stale), and on any
    // task exit. See finding 1 of the 2026-11-24 adversarial review.
    let mut pending_retry: Option<ObservedPullRequest> = None;
    let mut consecutive_failures: u32 = 0;
    // First poll fires immediately rather than after a full jitter: an operator typing
    // `watch` expects to see activity, not sixty seconds of silence. The jitter is applied
    // once, between the first and second ticks, which spreads the per-target schedules
    // without drifting the cadence (finding 7a of the 2026-11-24 review).
    let mut next_delay: Duration = Duration::ZERO;
    let mut jitter_pending: bool = true;

    loop {
        if *state.cancel.borrow() {
            break;
        }
        tokio::select! {
            changed = state.cancel.changed() => {
                // `Err` means the sender was dropped; treat it the same as a cancel.
                if changed.is_err() || *state.cancel.borrow() {
                    break;
                }
                continue;
            }
            () = tokio::time::sleep(next_delay) => {}
        }

        // Once the fetch starts, cancel no longer interrupts it: the Definition of Done's
        // shutdown scenario requires a poll in flight to run to completion.
        let fetch_result = (state.fetch)(state.locator.clone()).await;
        let step = apply_fetch_to_store(fetch_result, &store, &state.owner);
        let (outcome, reaction, delay) = match step {
            StepAfterWrite::Fresh(observed) => {
                let reaction = handle_fresh_reaction(
                    &state.locator,
                    state.reactor.as_ref(),
                    &mut prior,
                    &mut pending_retry,
                    observed,
                )
                .await;
                consecutive_failures = 0;
                (
                    TickOutcome::Fresh,
                    reaction,
                    success_delay(&state, &mut jitter_pending),
                )
            }
            StepAfterWrite::Unchanged => {
                let reaction = handle_unchanged_reaction(
                    &state.locator,
                    state.reactor.as_ref(),
                    &mut prior,
                    &mut pending_retry,
                )
                .await;
                consecutive_failures = 0;
                (
                    TickOutcome::Unchanged,
                    reaction,
                    success_delay(&state, &mut jitter_pending),
                )
            }
            StepAfterWrite::StoreFailed(msg) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let backoff = backoff_delay(state.interval, consecutive_failures);
                (
                    TickOutcome::StoreFailed {
                        error: msg,
                        backoff: Some(backoff),
                    },
                    None,
                    backoff,
                )
            }
            StepAfterWrite::FetchFailed(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let backoff = backoff_delay(state.interval, consecutive_failures);
                (TickOutcome::FetchFailed { error, backoff }, None, backoff)
            }
        };

        let tick = Tick {
            locator: state.locator.clone(),
            outcome,
            reaction,
        };
        // A send failure means the receiver was dropped: the caller has moved on, so there
        // is no one to report further ticks to. Exit cleanly.
        if state.events.send(tick).is_err() {
            break;
        }

        next_delay = delay;
    }

    Ok(())
}

/// Handle the reactor call for a Fresh tick: discard any stale parked retry (the world
/// moved on), invoke the reactor if one is attached, and settle `prior`/`pending_retry`
/// through [`advance_or_park`]. Returns the `ReactionReport` when a reactor ran.
/// Opens the task's store and seeds `prior` from the last observation of this PR.
///
/// A store-open failure is final for this target (`SchemaVersionMismatch`, file-permission
/// errors, a missing directory): the task reports the error once on the channel and the
/// caller returns `Ok`. `run_loop` still finishes cleanly for the other targets.
/// Finding 1 of the 2026-11-24 review: a prior version returned `Err` here, which held
/// the error until SIGINT drained every sibling task (the silent-partial-outage class).
///
/// A failure seeding the prior is reported on the channel but non-fatal: the task
/// continues with `None` prior. Finding 3 of the 2026-11-25 review: a bare
/// `.unwrap_or_default()` here swallowed a Decode error silently, breaking the fail-loud
/// contract; emitting a `StoreFailed` tick surfaces the gap at its first occurrence.
fn open_store_and_seed_prior(
    state: &TaskState,
) -> Option<(EventStore, Option<ObservedPullRequest>)> {
    let store = match EventStore::open(&state.store_path, state.owner.clone()) {
        Ok(s) => s,
        Err(e) => {
            let tick = Tick {
                locator: state.locator.clone(),
                outcome: TickOutcome::StoreFailed {
                    error: e.to_string(),
                    backoff: None,
                },
                reaction: None,
            };
            drop(state.events.send(tick));
            return None;
        }
    };

    let prior = match store.latest_for_number(state.locator.number) {
        Ok(found) => found,
        Err(e) => {
            let tick = Tick {
                locator: state.locator.clone(),
                outcome: TickOutcome::StoreFailed {
                    error: format!("seeding reactor prior: {e}"),
                    backoff: None,
                },
                reaction: None,
            };
            drop(state.events.send(tick));
            None
        }
    };

    Some((store, prior))
}

async fn handle_fresh_reaction(
    locator: &PullRequestLocator,
    reactor: Option<&Reactor>,
    prior: &mut Option<ObservedPullRequest>,
    pending_retry: &mut Option<ObservedPullRequest>,
    observed: ObservedPullRequest,
) -> Option<ReactionReport> {
    *pending_retry = None;
    if let Some(r) = reactor {
        let decision = (r)(locator.clone(), prior.clone(), observed.clone()).await;
        advance_or_park(prior, pending_retry, &decision, observed);
        Some(decision)
    } else {
        *prior = Some(observed);
        None
    }
}

/// Handle the reactor call for an Unchanged tick: if a previous Fresh parked a retry,
/// re-invoke the reactor against the still-unchanged prior and the parked observation.
async fn handle_unchanged_reaction(
    locator: &PullRequestLocator,
    reactor: Option<&Reactor>,
    prior: &mut Option<ObservedPullRequest>,
    pending_retry: &mut Option<ObservedPullRequest>,
) -> Option<ReactionReport> {
    let parked = pending_retry.clone()?;
    let r = reactor?;
    let decision = (r)(locator.clone(), prior.clone(), parked.clone()).await;
    advance_or_park(prior, pending_retry, &decision, parked);
    Some(decision)
}

/// The delay to use after a successful tick: the base interval, plus the per-target jitter
/// on the first successful tick only (ADR-0012).
fn success_delay(state: &TaskState, jitter_pending: &mut bool) -> Duration {
    if *jitter_pending {
        *jitter_pending = false;
        state.interval.saturating_add(state.jitter)
    } else {
        state.interval
    }
}

/// Advance `prior` on reactor success, or park `observed` for retry on reactor failure.
///
/// A `NotApplicable`, `AlreadyPosted` or `Posted` decision means the reactor compared the
/// prior against the new and reached a conclusion the next tick's comparison can build on;
/// `prior` moves forward. A `Failed` decision means the comparison never completed, so
/// `prior` stays where it was and the new observation parks as `pending_retry` for the next
/// tick to retry (see the review's finding 1). Clearing `pending_retry` on success closes
/// the retry loop.
fn advance_or_park(
    prior: &mut Option<ObservedPullRequest>,
    pending_retry: &mut Option<ObservedPullRequest>,
    decision: &ReactionReport,
    observed: ObservedPullRequest,
) {
    match decision {
        ReactionReport::Failed { .. } => {
            *pending_retry = Some(observed);
        }
        ReactionReport::NotApplicable
        | ReactionReport::AlreadyPosted
        | ReactionReport::Posted { .. } => {
            *prior = Some(observed);
            *pending_retry = None;
        }
    }
}

/// What the fetch-plus-store step produced, before the reactor is consulted.
///
/// Split from `run_one` so the store reference does not live across the reactor's await
/// point (the store's internal statement cache is `!Sync`, which would otherwise make the
/// task future non-`Send`). The reactor call happens up in `run_one` with an owned prior.
enum StepAfterWrite {
    Fresh(ObservedPullRequest),
    Unchanged,
    StoreFailed(String),
    FetchFailed(FetchError),
}

fn apply_fetch_to_store(
    fetch_result: Result<ObservedPullRequest, FetchError>,
    store: &EventStore,
    owner: &UserId,
) -> StepAfterWrite {
    match fetch_result {
        Ok(observed) => {
            let attribution = Attribution {
                user: owner.clone(),
                worker: WorkerId::new(POLL_WORKER),
                agent: ExecutingAgent::NoAgent,
            };
            let observation = PullRequestObservation::new(attribution, observed.clone());
            match store.write(&observation) {
                Ok(WriteOutcome::Fresh) => StepAfterWrite::Fresh(observed),
                Ok(WriteOutcome::Duplicate) => StepAfterWrite::Unchanged,
                Err(e) => StepAfterWrite::StoreFailed(e.to_string()),
            }
        }
        Err(error) => StepAfterWrite::FetchFailed(error),
    }
}

fn backoff_delay(interval: Duration, consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures
        .saturating_sub(1)
        .min(MAX_BACKOFF_SHIFT);
    let multiplier: u32 = 1u32.checked_shl(shift).unwrap_or(1u32 << MAX_BACKOFF_SHIFT);
    interval.saturating_mul(multiplier)
}

fn jitter_for(locator: &PullRequestLocator, interval: Duration) -> Duration {
    let interval_ms = interval.as_millis();
    if interval_ms == 0 {
        return Duration::ZERO;
    }
    let mut hasher = DefaultHasher::new();
    locator.hash(&mut hasher);
    // `interval_ms > 0` guards the modulus; `checked_rem` keeps clippy's arithmetic-side-
    // effects lint happy without any run-time cost. The result is already in
    // `[0, interval_ms)`, which fits a `u64` for any `Duration` that fits one.
    let remainder = u128::from(hasher.finish())
        .checked_rem(interval_ms)
        .unwrap_or(0);
    let offset_ms = u64::try_from(remainder).unwrap_or(0);
    Duration::from_millis(offset_ms)
}

/// Single-shot poll: fetch once, write once, return the outcome.
///
/// Used by the CLI's `record` subcommand. The loop's per-target task
/// ([`run_one`], private) does *not* call this function: a shared body would need to
/// return the outcome plus a backoff decision plus a tick payload, which is exactly the
/// shape `run_one` already has in closed form. Finding 6 of the 2026-11-24 adversarial
/// review flagged the duplication; it is rejected here because the shapes diverge where it
/// matters — `poll_once` reports a write outcome back to its caller as a `Result`, and
/// `run_one` translates that same outcome into a `Tick` for the scheduler. Collapsing them
/// would force a translation layer in one direction or the other, which is net code added
/// to remove code.
///
/// The `worker` parameter is the identity the attribution carries: `POLL_WORKER` from the
/// loop (built inline in `run_one`), `"cli"` from `record`, so invariant 6's three-
/// identity record distinguishes who initiated the row without inferring it from the
/// subcommand (ADR-0005).
///
/// # Errors
///
/// Returns [`PollError::Store`] if the write fails. A fetch error is not a [`PollError`]
/// (the loop treats it as a transient condition to back off on), so it is returned in the
/// inner `Result` via [`FetchError`].
pub async fn poll_once(
    store: &EventStore,
    fetch: &Fetch,
    locator: &PullRequestLocator,
    owner: &UserId,
    worker: &str,
) -> Result<Result<WriteOutcome, FetchError>, PollError> {
    match (fetch)(locator.clone()).await {
        Ok(observed) => {
            let attribution = Attribution {
                user: owner.clone(),
                worker: WorkerId::new(worker),
                agent: ExecutingAgent::NoAgent,
            };
            let observation = PullRequestObservation::new(attribution, observed);
            Ok(Ok(store.write(&observation)?))
        }
        Err(e) => Ok(Err(e)),
    }
}
