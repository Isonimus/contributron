---
id: '0015'
title: "React to merge conflicts cleared by posting a comment"
type: slice
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# Slice 0015: React to merge conflicts cleared by posting a comment

## Goal

M2's second deterministic rule. When the durable store records an observation of a pull
request whose mergeability is clean, and the immediately prior observation of the same pull
request recorded a conflicting mergeability, the reactor posts one comment on that pull
request. The comment carries a stable idempotency marker so a restart cannot cause a second
post.

This is the first slice where principle 2 and standing invariant 4 bite literally. The
provider's mergeability is a computed field that races with the push that cleared the
conflict: a payload observed through polling can lag the real repository state. The rule
reacts to a stored transition, and then re-fetches the pull request directly from the
provider in the moment before the write, posting only if the authoritative read confirms a
clean, non-dirty merge state. An event payload is evidence; the provider is authority.

No ledger items close here. Slice 0014's open follow-ups (store-side reaction persistence,
anonymous capture client, comment pagination, marker scan hardening) stay open; this slice
reuses the same surfaces and inherits the same gaps rather than paying them off, and the
amendment on each ledger line will cite 0015 once a later slice closes it.

## Definition of Done

- **Given** a pull request with a conflicting mergeable state has been observed and the
  observation written to the store
- **When** a new observation of the same pull request records a clean mergeable state
- **And** a live re-fetch of the same pull request at the moment of reaction also reports a
  clean mergeable state
- **Then** exactly one comment is posted to that pull request through the provider's
  comments endpoint, and the posted body contains the stable marker
  `<!-- contributron-rule:merge-conflicts-cleared -->`

- **Given** the most recent stored observation of a pull request recorded a clean mergeable
  state
- **When** a further observation also records a clean mergeable state
- **Then** no comment is posted: the rule reacts to the transition, not to the current state

- **Given** no observation of a pull request exists in the store
- **When** the first observation arrives with a clean mergeable state
- **Then** no comment is posted: "conflicts cleared" is a claim about a change, and there
  is no prior state to compare against

- **Given** a stored observation recorded a conflicting mergeable state and a new
  observation records a clean one
- **When** the live re-fetch at the moment of reaction reports a mergeable state that is
  not clean (conflicting, unknown, or a non-clean computed status such as dirty)
- **Then** no comment is posted and the reactor returns `NotApplicable`: the payload
  disagreed with the provider and the provider is authority

- **Given** the reactor has posted the marker comment for a prior transition and the
  process has since restarted
- **When** a further observation arrives, still with a clean mergeable state, and the
  provider reports the previously-posted marker comment still present
- **Then** no second comment is posted: marker-based idempotency holds across restarts the
  same way slice 0014's threads-cleared marker does

## Design

The mergeable state crosses the domain boundary as a tri-valued enum
(`Mergeable::{Mergeable, Conflicting, Unknown}`), added to `ObservedPullRequest` alongside
the existing fields. The GraphQL capture query is extended with the pull request's
`mergeable` field; the REST re-fetch at reaction time reads `mergeable` and
`mergeable_state` and keys the decision on both. The two names in the live read exist
because GitHub's `mergeable` boolean lies while the server is still computing (it returns
`null` during computation and the caller is expected to retry), while `mergeable_state`
distinguishes `dirty` (conflict) from `unstable`/`blocked`/`clean`. The reactor treats
anything other than `mergeable=true && mergeable_state in {clean, unstable, has_hooks}` as
"not yet authoritative, do not post", which is strictly safer than "post on uncertainty".

An `ObservedPullRequest` with a new field changes its content digest (ADR-0008). Pull
requests already observed under the old shape will be re-observed and re-written on the
first poll after this slice merges: the store then holds a legitimate second row whose only
difference is the mergeable field, and the loop will report one `Fresh` outcome per
previously-observed PR in that first tick. That is correct and expected; a transition from
"no prior mergeable field" to "mergeable=clean" is not a conflicts-cleared transition and
the reactor returns `NotApplicable` for it. Scenario 3 (no prior observation) already
covers this on the DoD; the only effect is one extra row per existing PR, which is a cost
the digest contract pays for being honest about field changes.

Existing code considered and reused: `crates/react/src/lib.rs` already models deterministic
reactors returning `ReactionReport`; a second rule is one more constructor-shaped function
alongside the first, not a reactor refactor. `crates/poll/src/lib.rs::handle_fresh_reaction`
dispatches to the reactor; the dispatch signature does not change — the reactor function
itself grows a provider-authoritative re-fetch, which is a dependency, not a new signature.
`crates/github/src/client.rs::post_pr_comment` and `list_comments` already carry the write
path and the marker scan; the marker constant changes, nothing else in the write path
moves. The idempotency scan reuses the same pagination gap ledgered for slice 0014 and does
not re-ledger it.

Existing code considered and *not* reused: no existing crate models a live pull-request
re-fetch distinct from the normalized capture. A one-off REST call is added inside the
`react` dispatcher rather than extended through `capture.rs`, because `capture.rs` is the
polling adapter and conflating "the record I polled" with "the state I confirm at the
write" would cost slice 0015 the very distinction invariant 4 exists to preserve. If a
third rule needs the same live-read surface, the rule of three (global §3) applies and the
shape is extracted then.

Marker: `<!-- contributron-rule:merge-conflicts-cleared -->`. The marker-scan hardening
work ledgered after slice 0014 (author filter, per-transition semantics) is the same here;
this slice ships the same once-ever semantics and inherits the same ledger line rather
than widening it.

Token resolution follows ADR-0013 unchanged. This slice adds no new secret surface: the
live re-fetch uses the same `CONTRIBUTRON_TOKEN` / `--token-path` the write path already
carries, and `scripts/lint-secrets.mjs` from slice 0014 continues to guard the write
boundary.

## Verification

- `crates/react/tests/merge_conflicts_cleared.rs` (new): the rule's unit surface. Asserts
  all five DoD scenarios against a fake provider that records posted comments and whose
  live-fetch result the test controls, so scenario 4 (payload-vs-live disagreement) can
  flip the live result independently of the stored observations.
- `crates/domain/tests/pull_request.rs` (extension): asserts the new `Mergeable` field
  deserializes from GraphQL-shaped JSON (`MERGEABLE`, `CONFLICTING`, `UNKNOWN`) and that
  `ObservedPullRequest`'s content digest changes with it (ADR-0008 identity contract).
- `crates/poll/tests/merge_conflicts_cleared_end_to_end.rs` (new): drives the poll loop
  with a scripted fetch returning a `Conflicting` observation followed by a `Mergeable`
  one, with the live-fetch stub returning `clean`. Asserts the second tick posts exactly
  one marker comment and the third tick is `Unchanged` with no second post.
- Idempotency-across-restart scenario is covered by the end-to-end test by re-opening the
  store under the same path after a simulated restart and confirming the marker scan
  blocks a second post.

No `scripts/<slice>-verify.mjs` is needed: every acceptance scenario is assertable in a
unit or integration test because the provider boundary is a trait the tests fake. Rule R11
is therefore trivially satisfied (no verify script to wire).

## As built

To be filled in at merge.

## Amendment — 2026-11-25: five correctness gaps from the adversarial review

The adversarial review of the first-cut slice-0015 code surfaced seven findings. Five
needed in-slice fixes; two stay ledgered. The fixes (and their names in the amended test
suite) are below. The pre-amendment implementation is preserved in git history.

**Finding 1: `Conflicting → Unknown → Mergeable` was silently missed.** GitHub's REST
payload lags its GraphQL payload by seconds to minutes: a push that clears a conflict
typically produces one or more `mergeable: null` observations (normalized to
[`Mergeable::Unknown`]) between the last Conflicting and the first Mergeable tick. The
poll loop advances its in-memory `prior` on every `NotApplicable`, so a single Unknown
tick erased the Conflicting evidence. The rule as shipped would almost never fire on the
common path.

Fix: a new [`MergeableHistory`] trait in the react crate exposes the stored history past
the in-memory prior. [`EventStore::latest_settled_mergeable_for_number`] returns the most
recent stored observation of this PR whose `mergeable` is not Unknown. When the immediate
prior is Unknown or absent, the rule walks back to the stored history before deciding. The
CLI's [`StoreMergeableHistory`] opens an `EventStore` on demand; the open is sub-
millisecond and only runs on ticks that actually need the walk. Regression:
`unknown_prior_with_conflicting_history_fires_and_posts` and
`unknown_prior_with_no_conflicting_history_does_not_fire` in
`crates/react/tests/merge_conflicts_cleared.rs`.

**Finding 2: transient "still computing" from the live read was a permanent miss.** The
first-cut `is_mergeable_clean_now` returned `bool`; a GitHub REST payload with
`mergeable: null` or `mergeable_state: unknown` mapped to `false`, the rule returned
`NotApplicable`, and `prior` advanced to Mergeable. The transition was then unrecoverable.

Fix: renamed to [`live_merge_has_conflict`] and widened the return semantics. `Ok(false)`
is now any non-dirty settled state (clean / unstable / has_hooks / blocked / behind — the
rule is about conflicts, not whether the PR is ready to merge in all other senses);
`Ok(true)` is only `dirty`; every transient state (`mergeable: null`, `mergeable_state`
unknown or draft) returns a [`ProviderError::Client`] which the reactor turns into
`ReactionReport::Failed`. Slice 0014's park-and-retry machinery then drives another
attempt on the next tick. Regression:
`live_read_computing_errors_so_the_retry_path_fires`.

**Finding 3: existing slice-0014 stores decoded old rows as a new shape, silently.** The
digest-bearing payload grew a `mergeable` field without a schema-version bump, so a store
written by slice 0014 would open on version 1 and then surface `StoreError::Decode` when
`EventStore::list` read an old row. `latest_for_number(...).unwrap_or_default()` in the
poll loop also swallowed the Decode error silently.

Fix: `SCHEMA_USER_VERSION` is now 2. An old store refuses at open time with
[`StoreError::SchemaVersionMismatch`], which the task reports as `StoreFailed` — the
operator deletes `events.sqlite3` (pre-production slice, no durable identity to preserve)
and restarts. A `#[serde(default)]` on `mergeable` keeps hand-constructed test fixtures
from crashing the decoder. And `open_store_and_seed_prior` now emits an explicit
`StoreFailed` tick on a seed-prior error instead of silently defaulting to `None`.
Regression: the existing `schema_version_is_refused` test in the store suite covers the
open-time refusal unchanged.

**Finding 4: the two-rule combiner dropped Posted outcomes and starved rule 2 on rule-1
errors.** `second?` in `combine_outcomes` discarded a first-rule `Posted`; the
`if let Err(e) = first { return Failed }` guard skipped rule 2 entirely when rule 1
errored.

Fix: both rules now run on every tick (the closure does not short-circuit), and
[`combine_rules`] folds the two `Result<ReactionOutcome, ReactError>` values into one
`ReactionReport` with these precedences: a real `Posted` wins over any error; an
`AlreadyPosted` beats an error on the other side; otherwise a `Failed` beats
`NotApplicable`. The remaining audit gap — both rules posting in the same tick drops one
`agent_id`, because `ReactionReport::Posted` carries one — is small (the two rules fire on
orthogonal observed fields) and is ledgered below.

**Finding 5: `run_one` tripped `-D warnings` on `too_many_lines`.** Pure rustfmt churn
had pushed the function from at-or-under 100 lines to 101. Fix: extracted
`open_store_and_seed_prior` as a helper, which also serves finding 3's fail-loud fix for
the seed path. No behaviour change.

The two findings NOT fixed in this slice:

- **Finding 6**: `mergeable` is normalized as `Option<Mergeable>` so a missing field at
  the payload level (not a `null` value — a genuinely absent key) silently defaults to
  Unknown rather than erroring at the normalizer. The code path to reach this is a
  self-inflicted GraphQL query change (the query must have selected `mergeable` for any
  observation to carry it), so it is a nit; it is ledgered as a follow-up for when the
  normalizer gains a double-Option deserializer or the payload shape is widened.
- **Finding 7**: the marker scan still reads a single comment-page and still matches on
  body substring regardless of author. Both are inherited from slice 0014 and already
  carry ledger lines; this slice does not re-ledger them.

### Updated Verification

The Verification section above names three suite files. After the amendment, the full set
is:

- `crates/react/tests/merge_conflicts_cleared.rs` — nine scenarios: the original five
  (scenario 3 refined to "no stored history" rather than "no immediate prior") plus
  `unknown_prior_with_conflicting_history_fires_and_posts`,
  `unknown_prior_with_no_conflicting_history_does_not_fire`,
  `live_read_computing_errors_so_the_retry_path_fires`,
  `history_error_on_unknown_prior_surfaces_as_react_error`.
- `crates/react/tests/merge_conflicts_cleared_end_to_end.rs` — unchanged apart from the
  signature update and the `LiveMerge` rename.
- `crates/domain/tests/identity.rs` — the mergeable-sensitivity assertion asserts the
  content digest moves with a flip of `Mergeable`; the hardcoded expected SHA-256 was
  recomputed for the new canonical encoding.
- `crates/github/tests/normalize.rs` — now asserts the fixture's new `MERGEABLE` field
  normalizes through.

