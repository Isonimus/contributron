---
id: '0017'
title: "Request reviewers when a draft PR is marked ready for review"
type: slice
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Goal

Add the fourth deterministic rule and the first NOTIFY-type reaction: when a pull request
transitions from draft to ready-for-review, call `request_reviewers` on an operator-
configured list so the targeted accounts receive a GitHub notification. The rule reuses the
`draft: bool` field already captured on `ObservedPullRequest` (unused by any rule until
now), fires at most once per draft→ready transition, and is idempotent against the current
review-request list (a reviewer already on it is not re-requested). This is the first rule
whose write targets a third party (the reviewer being asked), so idempotency matters for
reasons stronger than tick repetition: double-requesting a human is a user-visible defect,
not just a wasted API call.

No ledger item is closed by this slice. The anonymous-client [feature] in the ledger is
honoured by having the rule report `NotApplicable` when no reviewers are configured, which
keeps the read path intact for token-less watches.

## Definition of Done

- **Given** a pull request whose last observation had `draft: true` and whose new
  observation has `draft: false`, and at least one configured reviewer is not currently
  on the review-request list
- **When** the reactor runs on that tick
- **Then** the rule calls `request_reviewers` with exactly the missing reviewers, and the
  tick reports `Posted` with a `ReviewRequested` receipt listing those reviewers.

- **Given** a pull request whose last observation had `draft: false` (no transition),
  regardless of the current review-request list
- **When** the reactor runs on that tick
- **Then** the rule reports `NotApplicable` and no provider write happens.

- **Given** a pull request whose last observation had `draft: true` and whose new
  observation has `draft: false`, where every configured reviewer is already on the
  review-request list
- **When** the reactor runs on that tick
- **Then** the rule reports `AlreadySettled` and no provider write happens.

- **Given** a pull request whose in-memory `prior` is `None` (fresh task after restart)
  and whose new observation has `draft: false`
- **When** the reactor runs on that tick
- **Then** the rule walks the store's observation history for the same locator, treats the
  last stored `draft` as the effective prior, and decides the transition from there; if no
  prior row exists in the store, the rule reports `NotApplicable` (no transition can be
  evidenced).

- **Given** the operator started the CLI without any `--reviewer` and without
  `CONTRIBUTRON_REVIEWERS`
- **When** any tick fires the needs-rebase or threads-cleared rule
- **Then** the reviewer rule reports `NotApplicable` on every tick and never touches the
  provider (read or write), so a token-less public-repo watch continues to work.

- **Given** the operator configured one or more reviewers and the write path has no token
- **When** a qualifying transition fires
- **Then** the rule returns a `ReactError` carrying the ADR-0013 message, the combiner
  classifies the tick, and the partial-failure retry contract from the 2026-11-25 review
  parks the observation so a later tick (with a token or without the transition) can retry
  cleanly.

## Design

### Signal

`ObservedPullRequest.draft: bool` has been captured since slice 0014 but no rule has used
it. The transition gate is `prior.draft && !new.draft`; the reverse (ready→draft) is a
different signal and out of scope here.

### History-walk (invariant 4 interaction and the Unknown-bridge)

Slice 0015 introduced `MergeableHistory` so a fresh task after restart could still decide
a transition against the last stored row rather than silently miss one. Draft is binary
(`bool`, not tri-state like `Mergeable`), so there is no `Unknown` to bridge — but the
history-walk itself is still needed for the same operator-restart reason. Introduce
`DraftHistory` as a sibling trait in the react crate, with `StoreDraftHistory` bridging
onto the store, mirroring the `MergeableHistory` / `StoreMergeableHistory` pair. The
history-walk returns `Option<bool>` (None = no stored row); None at the gate reports
`NotApplicable`.

Invariant 4 (re-fetch before sensitive operations) was considered. It specifically names
mergeability, approval and branch-protection as the state an event payload cannot
authoritatively source; `draft` is none of those, and `request_reviewers` is not a
sensitive write in that sense (the operator opted into the reviewer list at startup). The
rule therefore does *not* live-refetch before writing. Documenting the decision here so a
later operator does not "fix" the absence.

### Reviewer configuration

CLI flag `--reviewer <login>` (repeatable) and env var `CONTRIBUTRON_REVIEWERS=login1,login2`
(comma-separated). The flag wins over the env var when both are present. Logins are
validated via the existing `AuthorLogin` newtype rather than introducing a `ReviewerLogin`:
the shape is identical (nonempty GitHub login) and the rule-of-three threshold for a shared
`Login` newtype is not reached yet. The field's semantic meaning in this crate is spelled
out at the use site (`reviewers: Vec<AuthorLogin>`); a renamed newtype can land when a
third consumer appears.

Zero configured reviewers means the rule reports `NotApplicable` unconditionally, keeping
the token-less read path intact.

### Provider surface

Two new `ProviderClient` methods:

- `list_requested_reviewers(&self, locator) -> Result<Vec<AuthorLogin>, ClientError>`
  (read path; token-optional, same treatment as `list_labels`).
- `request_reviewers(&self, locator, reviewers: &[AuthorLogin]) -> Result<(), ClientError>`
  (write path; `MissingTokenClient` surfaces the ADR-0013 message).

No `remove_requested_reviewer` is added: this rule never removes a reviewer, and adding a
method we do not call would violate the global rule-of-three bar. A remove-side rule, if
written later, adds that method then.

### Receipt shape

New `ReactionKind::ReviewRequested { reviewers: Vec<AuthorLogin> }`. The combiner treats it
the same way it treats `Commented` and `LabelApplied`: one agent id per receipt, aggregated
into `ReactionReport::Posted { agent_ids, errors }`. The 2026-11-25 partial-failure fix
(parking a Posted-with-errors tick) carries through unchanged because the fix was made in
`advance_or_park`, not in the rule layer.

### Idempotency

The read-then-write pattern is: call `list_requested_reviewers`, filter the configured list
to those not already present, and only call `request_reviewers` with the remainder. Empty
remainder → `AlreadySettled`. The idempotency witness is the review-request list itself,
not a marker comment.

### What was searched for and not reused

Searched the three rule modules (`react_threads_cleared`, `react_merge_conflicts_cleared`,
`react_needs_rebase_label`) for a shared "transition-then-idempotency" scaffold. The three
are similar in shape but differ in signal (threads vs mergeable vs draft), in the
idempotency witness (marker comment vs label presence vs request-list), and in the receipt
kind. Extracting a shared trait or generic across three rules would hit the rule-of-three
exactly, but the variation is in the body rather than a parameter, so the abstraction
would carry three distinct closures per rule and lose the per-rule error messages. Not
reused; written as a fourth rule mirroring the third's shape.

Reused directly: the `combine_rule_results` combiner from the react crate, the
`StoreDraftHistory` pattern cloned from `StoreMergeableHistory`, the `MissingTokenClient`
classification pattern from slice 0016, the `advance_or_park` partial-failure fix from the
2026-11-25 review (no change needed here; it already covers this rule's errors).

## Verification

Unit tests in `crates/react/tests/`:

- `review_requested.rs::fires_on_draft_to_ready_transition` — happy path, the first DoD
  scenario.
- `review_requested.rs::no_transition_on_stable_ready` — second DoD scenario.
- `review_requested.rs::already_settled_when_every_reviewer_present` — third DoD scenario.
- `review_requested.rs::walks_history_when_prior_is_none` — fourth DoD scenario, positive
  case (history has a draft row, transition fires).
- `review_requested.rs::not_applicable_when_history_is_empty` — fourth DoD scenario,
  negative case.
- `review_requested.rs::not_applicable_when_no_reviewers_configured` — fifth DoD scenario,
  rule layer.
- `review_requested.rs::subset_requests_only_missing_reviewers` — partial overlap between
  configured and already-on-list, calls `request_reviewers` with only the gap.
- `review_requested.rs::ready_to_draft_is_not_applicable` — reverse transition does
  nothing, explicit negative.
- `review_requested_end_to_end.rs` — two end-to-end tests exercising the real combiner,
  advance_or_park and reactor wiring: happy-path write and `MissingTokenClient` surfacing
  the ADR-0013 message (sixth DoD scenario).
- `missing_token_fails_loud.rs` extended with a `request_reviewers` arm; `list_requested_
  reviewers` added to the read-path-still-works arm.
- Domain crate: no new tests (AuthorLogin is already covered).
- Poll crate: no new tests; the partial-failure retry test from 2026-11-25 already covers
  this rule's error class.

## As built

Shipped as designed. The four-rule reactor now folds a `review-requested` outcome
alongside threads-cleared, merge-conflicts-cleared and needs-rebase-label through the
existing `combine_rule_results` combiner. The pre-written Definition-of-Done scenarios all
hold:

- Scenario 1 (happy-path transition) verified by `review_requested.rs::
  fires_on_draft_to_ready_transition` and by the end-to-end
  `review_requested_end_to_end.rs::draft_to_ready_transition_requests_the_configured_reviewers`.
- Scenario 2 (no transition) verified by `review_requested.rs::no_transition_on_stable_ready`
  and by the non-qualifying-tick arm of `missing_token_fails_loud.rs::
  review_requested_without_a_transition_does_not_touch_the_client`.
- Scenario 3 (already-settled) verified by `review_requested.rs::
  already_settled_when_every_reviewer_present`.
- Scenario 4 (history-walk, positive and negative) verified by
  `review_requested.rs::walks_history_when_prior_is_none` and
  `review_requested.rs::not_applicable_when_history_is_empty`.
- Scenario 5 (no reviewers configured) verified by `review_requested.rs::
  not_applicable_when_no_reviewers_configured`. The write path's shape was exercised at
  the CLI boundary by the new `resolve_reviewers_with_env` unit tests.
- Scenario 6 (write without a token) verified by
  `missing_token_fails_loud.rs::review_requested_without_a_credential_errors_before_any_write`
  and by the end-to-end
  `review_requested_end_to_end.rs::draft_to_ready_without_a_credential_surfaces_the_adr_0013_error`.

Deviations from the Design, in order of relevance:

1. `react_review_requested` carries eight arguments (owner, worker, locator, prior, new,
   history, client, reviewers), tripping `clippy::too_many_arguments`. The extra argument
   relative to `react_needs_rebase_label` is the reviewer slice, and splitting the owner,
   worker and attribution into a struct would be premature generalisation of what is still
   a per-rule signature (four rules, four shapes). Marked with `#[allow(clippy::
   too_many_arguments)]` at the function with no change in call sites. Later rules may
   justify an `Invocation` carrier if this grows again.
2. `DraftHistory::last_draft` is built on the store's existing `latest_for_number` (which
   returns the whole observation and whose `draft` field the bridge reads), rather than a
   new store method mirroring `latest_settled_mergeable_for_number`. Draft has no settled
   filter to apply, so the general `latest_for_number` is the right axis: adding a
   `latest_draft_for_number` method would duplicate the index path with no new semantics.
3. `report_reaction` was not changed. The `ReactionReport` carrier records agent ids and
   errors, not per-receipt kinds; the review-requested write is reported as
   `reaction-posted <locator> by rule:review-requested`, consistent with how
   `LabelApplied` and `LabelRemoved` are already surfaced. A per-receipt report line
   (with the requested reviewer logins) would require widening the report, which the
   2026-11-25 widening already accomplished at the agent-id layer only. Printing the
   reviewer logins belongs behind a configurable verbosity toggle that is out of scope
   here.
4. The CLI tests exercise `resolve_reviewers_with_env` rather than `resolve_reviewers`
   directly because the workspace forbids `unsafe_code` and `std::env::set_var` is
   `unsafe` in edition 2024; the env-reading side is covered by the `resolve_reviewers`
   wrapper, which has a single `std::env::var` call site. This is the standard pattern
   for env-facing boundary code under the forbid.
