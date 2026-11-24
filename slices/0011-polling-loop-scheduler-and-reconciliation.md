---
id: '0011'
title: "Polling loop, scheduler and reconciliation"
type: slice
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# Slice 0011: Polling loop, scheduler and reconciliation

## Goal

Make the system observe without being asked. Given a list of pull requests to watch, the
CLI's `watch` subcommand polls each from GitHub on a bounded cadence, normalizes the
payload and writes it to the durable store, and keeps running until the operator asks it
to stop. Two polls of an unchanged pull request add one row (the first) and zero
thereafter: slice 0009's dedup is what makes that work, and this slice is what makes it
happen without a human invoking `record` on every tick.

This is the third and final vertical cut of M1. Slice 0007 established the normalization
boundary and the renderer; slice 0009 made an observation survive its process. This slice
turns the pair into a running system: a scheduler controls per-pull-request cadence with
jitter and an exponential backoff on error, and reconciliation against provider state is,
in M1's degenerate shape, the loop itself (ADR-0003 installed the "re-fetch regardless of
what the event stream said" invariant; with no event stream yet, every tick is the
re-fetch).

The slice closes no open ledger items. It opens one by reference rather than inventing it:
GitHub rate-limit headers are not consulted here, so the cadence is an operator-set
interval rather than a rate-limit-aware one. That work is already a consequence of the
credential decision (`[feature] Decide how provider credentials are acquired and held`) and
is routed to M2 for the same reason.

## Definition of Done

- **Given** one pull request to watch and a stub fetch that returns the same observation
  on every call
- **When** the polling loop runs for ten ticks of the configured interval under
  `tokio::time::pause()`
- **Then** the store holds exactly one row for that pull request, the loop reports one
  fresh write and nine duplicates on stdout in that order, and no row written later carries
  a different attribution than the first

- **Given** one pull request and a stub fetch whose observation changes between tick five
  and tick six
- **When** the loop runs for ten ticks
- **Then** the store holds exactly two rows for that pull request with the two expected
  contents, and `list` returns them in a deterministic order that a later diff can rely on

- **Given** two pull requests on the same configured interval and a stub fetch that
  completes instantly for both
- **When** the loop runs for N ticks under paused time
- **Then** each pull request was polled N times, so a slow or failing poll on one pull
  request does not starve the other

- **Given** a stub fetch that returns a transient error on the first call and succeeds
  thereafter
- **When** the loop runs for enough ticks that the backoff deadline for that pull request
  has elapsed
- **Then** the loop did not exit, exactly one error line naming the pull request and the
  applied backoff was written to stderr, and the store holds a row written by a subsequent
  successful call

- **Given** a running polling loop
- **When** the shutdown signal is sent (a cancellation token the test flips, which
  production wires to `ctrl_c`)
- **Then** a poll in flight runs to completion, no further poll is started, the store is
  closed, and the loop returns within one configured interval of wall clock

- **Given** the CLI `watch` subcommand wired through the same code path as production but
  with a stub fetch supplied for the test
- **When** it polls one pull request once and shuts down
- **Then** `contributron list --store <same file>` prints the observation the stub
  produced, so the loop and the earlier CLI paths write rows of the same shape

## Design

A new crate, `contributron-poll`, depending on `contributron-domain` and
`contributron-store` and on nothing in the workspace that knows about GitHub. The CLI
wires the GitHub adapter and the polling crate together; the GitHub crate gains no
scheduler, because an adapter has no business running one.

**The provider seam.** The loop calls a boxed async function the CLI builds, not a trait:

```rust
pub type Fetch = Arc<
    dyn Fn(PullRequestLocator) -> BoxFuture<'static, Result<ObservedPullRequest, FetchError>>
        + Send + Sync,
>;
```

The GitHub adapter is one such function (`capture_pull_request` followed by
`normalize_capture`); tests supply another. A trait would be speculative generality for a
single in-tree implementation, which the quality bar forbids until a second one shows up.
The ledger already carries the second provider (GitLab) as the case that earns the
abstraction: on that slice, the seam becomes a trait; on this one, it is a function
pointer.

**Cadence.** Each watched pull request gets its own `tokio::time::interval` set to the
operator's `--interval`, plus a per-pull-request uniform jitter in `[0, interval)` chosen
once at startup so N pull requests started together do not stampede the provider on the
same second. A failed fetch increments a per-pull-request backoff counter and the next
deadline becomes `interval * 2.pow(min(counter, 6))`, capped at `interval * 64`. A
successful fetch resets the counter. This is the policy ADR-0012 records: scheduler
decisions a second slice would otherwise relitigate from nobody's memory.

**Reconciliation.** There is nothing to reconcile *against* yet: no webhook stream and no
cached "we believed this was true" pointer other than the store itself. The polling loop
*is* the reconciliation ADR-0003 names, in its M1 shape. When webhooks arrive, that slice
adds a second source feeding the same store and owns the drift report; until then the
invariant is carried by the mechanism itself.

**Time control in tests.** `tokio::time::pause()` already provides deterministic virtual
time, so there is no `Clock` trait. A trait introduced to re-expose a seam the runtime
already offers is the test-shaped production code the quality bar forbids.

**Shutdown.** `tokio::signal::ctrl_c` feeds a `CancellationToken`; the loop's top-level
`select!` races the next tick against the token. A poll in flight runs to completion; a
poll that has not started is not started. The deadline ("within one poll interval") is a
scenario above, so the race is testable rather than asserted by reading.

**Error channel.** Provider errors go to stderr as one line per failure, naming the
pull-request locator and the backoff applied. They are not persisted: the store is for
observations, not operational telemetry. A metrics plane with durable error history is a
later concern (not yet a ledger line; a probe or first bug report owns raising it).

**Attribution of the poll.** Every write the loop produces carries
`Attribution { user: LOCAL_USER, worker: WorkerId::new("poll"), agent: NoAgent }`.
The CLI's existing interactive subcommands keep `worker: "cli"`. The two strings are two
facts (invariant 6 names three identities; "which worker" is one of the three), so the
audit record distinguishes an operator-initiated `record` from a loop-initiated poll
without inferring it from the subcommand. The constant moves into `contributron-poll`
rather than being spelled in the CLI, so a future second worker type (webhook ingestion)
picks up the same channel.

**CLI surface.**

```
contributron watch \
  --target owner/name#N  \
  --target owner/name#M  \
  --interval 60s         \
  --store ./events.sqlite3
```

Repeatable `--target`, default interval `60s`. The command runs until SIGINT. One stdout
line per tick (`fresh <locator>` or `unchanged <locator>`), so a test can assert the full
sequence by reading captured output; errors are on stderr.

### What existing code was considered

- **`contributron-github::capture_pull_request` and `normalize_capture`** — reused
  directly as the function the CLI closes over to build the production `Fetch`. The
  adapter boundary is already in place; the polling crate stays unaware of GitHub.
- **`contributron-store::EventStore::write`** — reused directly. Its
  `WriteOutcome::{Fresh, Duplicate}` return is exactly the signal the loop needs, so the
  loop reports what the store reported rather than re-deriving "did anything change".
- **The CLI's `Command::Record` body** — considered and partly reused by extraction. Its
  shape (open the store, fetch, write, report outcome) is one tick of the loop. The
  shared piece ("fetch one, write one, report") moves into
  `contributron_poll::poll_once`; `record` becomes a one-call wrapper over it, which is
  the rule-of-three second caller the extraction needs. Calling `record`'s module from
  the loop crate directly would have coupled the loop to the CLI's layout and is
  rejected.
- **A real scheduler crate (`tokio-cron-scheduler`, `cron`)** — rejected. Cron is the
  wrong shape (calendar times, not per-target offsets) and the scheduler's one job is
  "wake every N with jitter," which `tokio::time::interval` plus a `rand` call already
  does. Pulling in a scheduler framework for one timer per pull request is dependency
  weight without a user.
- **A `Clock` trait** — rejected. `tokio::time::pause()` is already a deterministic virtual
  clock in test; production uses real time with no abstraction in between.
- **A `Provider` trait** — rejected for the same reason. One in-tree implementation, one
  test double, and a boxed function at the seam is simpler at every call site than a
  trait. The second real provider is the case that earns the abstraction.
- **GitHub conditional-request headers (`If-None-Match`, `Last-Modified`)** — not used
  here. They would reduce API cost but the store already dedups on content, so they are
  an optimization, not a correctness lever. Not yet a ledger line; it earns one the day a
  measurement shows the cost matters.
- **The existing `Attribution` and `WorkerId` newtypes** — reused directly for the poll
  worker. The `[bug]` ledger line about empty-string identifiers is an upstream fix in
  the domain crate; this slice does not re-validate at its own boundary.

### On measured numbers

No design question in this slice has an answer a probe would change today. The default
interval (60s) is an operator-configurable product choice; the jitter window
(uniform on `[0, interval)`) is the minimum that prevents simultaneous stampede and the
maximum that keeps worst-case latency at one interval. A rate-limit probe becomes worth
running when the rate-limit-aware scheduler lands; the credential ledger line is the
natural place that work surfaces.

## Verification

Unit and integration tests. Every scenario above is assertable in Rust under
`tokio::time::pause()` and a stub `Fetch`; no `scripts/*-verify.mjs` is introduced because
no behaviour here needs runtime visual inspection (stele:ADR-0004 — a verify script is
for behaviour a unit test cannot assert).

| scenario | proof |
|---|---|
| Unchanged state: ten ticks, one fresh and nine duplicates | `crates/poll/tests/unchanged_is_deduped.rs` |
| Changed state: new content between ticks produces a second row | `crates/poll/tests/change_produces_new_row.rs` |
| Fairness: two pull requests each polled N times under paused time | `crates/poll/tests/fairness.rs` |
| Transient error: the loop survives one failure and writes on the next success | `crates/poll/tests/transient_error_backoff.rs` |
| Shutdown: cancellation drains the in-flight poll and exits within one interval | `crates/poll/tests/shutdown.rs` |
| CLI end-to-end: `watch` then `list` matches the stubbed observation | `crates/cli/tests/watch_then_list.rs` |

An ADR ships in the same commit (`adr/0012-polling-cadence-is-per-target-interval-with-jitter-and-backoff.md`):
the cadence policy is a decision later work must obey, and recording it now is cheaper
than re-deriving it on the rate-limit slice. The id jumps past 0011 for the same reason
slice 0009 cites ADR-0010 (stele:R2, shared ADR + slice id space).

**Not verified by this slice, and deliberately:**

- **Rate-limit arithmetic against real GitHub headers.** The adapter does not yet read
  `X-RateLimit-*`. A token bucket is not here, and the feature line routes through the
  credential decision rather than being filed as a standalone optimization.
- **Webhook ingestion as a second event source.** Deferred by ADR-0003; the loop's design
  leaves the store open to a second writer when that slice arrives.
- **Multi-worker polling of the same pull request.** M1 is single-operator (ADR-0005), so
  there is at most one writer at a time; invariant 7's owner-scoping already covers the
  row half, and "who polls which pull request" is scheduling work for the team-model
  slice the ledger defers.
- **Credential rotation during a long-running loop.** Rotation is a consequence of
  whichever credential shape wins the open ledger decision, not this slice's concern.
- **The empty-string identifier gap in `string_id!`.** It is a ledger bug on the domain
  crate; the poll worker's `WorkerId::new("poll")` is a literal and cannot hit it, so the
  fix is not held up here.

## As built

<!-- Filled at merge, in past tense, confirming each Definition-of-Done scenario was met or
     recording the deviation. This is the freeze step. -->
