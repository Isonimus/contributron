---
id: '0012'
title: "Polling cadence is per-target interval with jitter and exponential backoff"
type: architecture
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# ADR-0012: Polling cadence is per-target interval with jitter and exponential backoff

## Context

ADR-0003 decided that intake is by polling, with reconciliation against provider state.
The decision answered *whether* to poll. The slice that ships the loop (slice 0011)
answers *how often*, and the answer is a decision later work must obey rather than a shape
to reinvent on the rate-limit-aware scheduler.

Three questions want independent answers:

1. **Cadence shape.** One interval across all targets, or one per target? A per-target
   schedule allows a hot pull request to be polled more often than a quiet one, but
   nothing in M1 observes "hotness" and no feature yet uses the degree of freedom. The
   scheduler still *has* to run per-target timers to keep a slow or failing fetch on one
   target from starving another (the fairness scenario in the slice), so the shape is one
   timer per target whether or not the interval itself varies.
2. **Stampede avoidance.** N targets started at the same instant will fire at the same
   second on every tick if they share one wake-up moment, which turns a workstation CLI
   into a mild thundering herd against the provider. A spread across one interval width is
   the minimum that fixes it and the maximum that keeps worst-case per-target latency at
   one interval.
3. **Error response.** A fetch failure is almost always transient (API blip, rate limit,
   network). Retrying at the same cadence turns a five-minute provider outage into three
   hundred failures per target. Backing off exponentially caps the total failure cost at
   `O(log)` of the outage, which is the usual answer for the usual failure.

## Decision

Each target has its own timer. All timers share one operator-configured base interval
(default sixty seconds, overridable per invocation). On every target, the next scheduled
poll is:

- On startup: `now + jitter(target)`, where `jitter(target)` is uniform on
  `[0, interval)` and chosen once per startup. Two targets started together therefore fire
  at two different offsets, and the maximum first-poll latency is one interval width.
- After a successful poll: `now + interval`. The counter of consecutive failures resets
  to zero.
- After a failed poll: `now + interval * 2.pow(min(consecutive_failures - 1, 6))`. The
  exponent is capped at six, so the worst case is `64 * interval` — at the default
  interval, roughly an hour between attempts for a persistently failing target. A single
  success resets the counter.

Jitter is derived deterministically from a hash of the target's identifier rather than
drawn from a random number generator. This avoids pulling in `rand` for one `u64 % N` and
makes the schedule reproducible across runs: a given target on a given interval has a
given jitter.

The scheduler lives in the `contributron-poll` crate. The CLI wires it; the GitHub
adapter does not know it exists.

## Consequences

1. The scheduler is self-contained. Nothing about its policy reads provider response
   headers, so rate-limit-awareness remains the next slice's problem (the credential
   feature line already carries that work, because it needs the credential decision to
   know whose budget it is spending).
2. Per-target cadence ceases to be a design question when the first workflow needs it: the
   shape already supports a per-target interval override, so the slice that introduces one
   adds a configuration field rather than a scheduler model.
3. A successful poll resets the counter unconditionally. A target that flaps between
   success and failure on alternating polls therefore never backs off. That is the right
   answer for M1 and is called out here so a later "flap suppression" slice knows the
   policy it is changing.
4. Jitter is deterministic per `(target, interval)`. A test fixture can assert the exact
   order and spacing of two targets' first polls without a randomness seam, which the
   quality bar's test-shape rule would otherwise require.
5. The sixty-second default is an operator choice, not a durable decision. It is here as
   the default the CLI ships with, not a value an ADR should pin: tightening it to five
   seconds for a demo is correct behaviour, and no code elsewhere depends on sixty.

## Alternatives considered

**A single global timer that ticks every interval and polls every target in a round.**
Simpler to implement and reason about. Rejected because a slow fetch on one target delays
every other target's tick by exactly its duration, so a provider that returns a slow 429
on one pull request starves every other pull request for as long as the 429 takes. The
fairness scenario in slice 0011 is the one that forbids this.

**Cron expressions as the cadence vocabulary** (`tokio-cron-scheduler` or similar).
Rejected. Cron is wrong in shape: it specifies calendar moments, not per-target offsets
from startup, and an operator restarting the CLI during the one minute of the hour that
has no cron fire would see no polls for a minute. The scheduler's one job is
"wake every N with jitter"; a cron grammar is a vocabulary without a user yet.

**A randomized jitter drawn from `rand::rng()`** rather than hashed from the target.
Rejected on dependency weight: the `rand` crate pulls in a trait tower (`RngCore`,
`SeedableRng`, feature-gated distributions) for one `u64 % N`. The randomness here is a
spreading function, not a security or a statistical one, and a stable hash is a strictly
cheaper spreading function. If a future decision demands a strong RNG (cryptographic
backoff randomisation, say), it supersedes this one; today it would be speculative weight.

**Constant backoff** (a fixed longer delay after any error). Rejected because a single
API blip would push the next attempt out by the backoff, which is pessimistic if the
underlying condition cleared immediately. Exponential starts cheap and grows only if the
condition persists.

**No backoff at all.** Rejected for the inverse reason: a five-minute provider outage at a
sixty-second interval produces five polls per target that all fail, which is five times
the useful load on the provider for zero information gain.

**A rate-limit-aware scheduler now.** Rejected for M1. It depends on who the credential
belongs to (the credential decision is a ledger line), and the store's content-based dedup
already prevents duplicate rows from a chatty cadence. The optimization is scheduled for
M2 by the same ledger line.

## Review-only invariants that stay review-only after this ADR

- Invariant 4 (provider state re-fetched before any sensitive operation): the loop is the
  re-fetch mechanism when webhooks eventually arrive, but no sensitive operation exists in
  M1, so this ADR does not change how the invariant is enforced.
- Invariant 6 (every operation names all three identities): the loop uses `POLL_WORKER`
  (the constant `"poll"`) for every row it writes, distinct from the CLI's `"cli"`. The
  invariant is still enforced at review; it is not possible to express "the loop uses a
  worker string distinct from the interactive CLI's" as a linter rule without pinning the
  specific strings, which is not the invariant.

## Amendment — 2026-11-24: first-poll timing and the determinism claim

Adversarial review of the initial implementation flagged two things this ADR said more
strongly than the code warrants. The record stands as written; the amendment is what a
reader implements against.

- **First poll timing.** The Decision block above says "on startup: `now + jitter(target)`".
  That delays the first observation by anywhere up to one interval, which is the whole
  default poll gap of silence in exchange for a stampede-avoidance property that only
  matters on *subsequent* polls. The code fires the first poll at `now` and applies the
  jitter to the delay between successive ticks. One operator typing `watch` sees activity
  immediately; N targets still fan out across the first interval because each target has
  a distinct jitter applied after its first tick. The stampede argument is unchanged;
  what moved is the moment the spread first takes effect (finding 7a of this review).

- **Determinism of jitter.** The Decision block above calls the jitter "reproducible
  across runs". `DefaultHasher` is not stable across Rust releases, so the schedule a
  given target produces is reproducible only within one toolchain version. This is
  acceptable because the jitter is a *spreading* function here, not an identity or a
  reproducibility promise; two toolchain versions on the same operator's machine would
  simply spread the same two targets to two different offsets, both of which fulfil the
  stampede argument. A future slice that needs stable-across-versions jitter supersedes
  this ADR; today the weaker guarantee is the one the code actually honours and is enough
  for what the scheduler is doing (finding 7b of this review).
