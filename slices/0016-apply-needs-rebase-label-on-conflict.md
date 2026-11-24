---
id: '0016'
title: "Apply a needs-rebase label when a PR becomes conflicting"
type: slice
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

## Goal

Ship the third deterministic rule, and the first one whose action is a **label** rather
than a comment. When a pull request transitions into `Conflicting`, apply the
`needs-rebase` label; when it transitions back to `Mergeable`, remove the label. The rule
is symmetric to slice 0015 ([[slices/0015-react-to-merge-conflicts-cleared]]) — it reads
the same `Mergeable` field, reuses the `MergeableHistory` Unknown-bridge, and gates its
write on the same live provider re-fetch (invariant 4). A label is its own idempotency
signal: the pre-write check reads the current label set rather than scanning for a marker.

This slice closes the `[feature]` the 2026-11-25 adversarial review of slice 0015 left
open: `ReactionReport::Posted` (or the tick-report surface) must record all firing agent
ids rather than dropping one when multiple rules fire in a tick. With three rules in play
and the first one that labels rather than comments, the overlap stops being improbable:
a tick that observes `Conflicting → Mergeable` plausibly removes the label (rule 3) *and*
posts the merge-cleared comment (rule 2), and invariant 6 demands both writes are
attributed. The report surface is widened once here and the ledger line is deleted.

## Definition of Done

- **Given** a pull request whose stored prior is `Mergeable`
- **When** a new observation reports `Conflicting` and the live provider re-fetch
  confirms the conflict, and the `needs-rebase` label is not already applied
- **Then** the `needs-rebase` label is applied exactly once, the reaction is recorded
  with `agent_id = "rule:needs-rebase-label"`, and a second observation in the same state
  is a no-op

- **Given** a pull request carrying the `needs-rebase` label whose stored prior is
  `Conflicting`
- **When** a new observation reports `Mergeable` and the live provider re-fetch confirms
  no conflict
- **Then** the `needs-rebase` label is removed exactly once, the reaction is recorded
  with `agent_id = "rule:needs-rebase-label"`, and a second observation in the same state
  is a no-op

- **Given** a pull request whose stored prior is `Unknown` and whose next settled history
  row is `Mergeable`
- **When** a new observation reports `Conflicting` and the live read confirms the
  conflict
- **Then** the rule walks the history back to `Mergeable`, recognises the transition and
  applies the label — the Unknown-bridge introduced by slice 0015 is reused symmetrically
  for the apply direction

- **Given** a pull request whose stored prior is `Conflicting`
- **When** a new observation reports `Mergeable` and the live provider re-fetch reports
  "still computing"
- **Then** the rule returns `ReactError` and the poll loop's park-and-retry
  (slice 0014) re-attempts on the next tick; no label change is recorded

- **Given** a tick in which rule 2 (`rule:merge-conflicts-cleared`) posts a comment *and*
  rule 3 (`rule:needs-rebase-label`) removes the label
- **When** the reactor combines the two outcomes
- **Then** the resulting `ReactionReport` carries both agent ids (one agent id per
  write), neither is dropped, and invariant 6 holds for both operations

- **Given** the operator runs the CLI without a token (`CONTRIBUTRON_TOKEN` unset and no
  `--token-path`)
- **When** the poll loop observes a conflict-apply transition
- **Then** the write path aborts with the ADR-0013 missing-token message and the read
  path continues — no label mutation is attempted, mirroring rule 1 and rule 2's
  behaviour

## Design

**What exists already, and the reuse-or-duplicate choice.**

Searched: `crates/react/src/lib.rs` for the rule shape and the `ProviderClient` trait;
`crates/github/src/client.rs` for the live mergeability re-fetch and the comment write
path; `crates/cli/src/main.rs` for `build_reactor`, `combine_rules`, `pick_outcome` and
the `MissingTokenClient`; `crates/store/src/lib.rs` for `latest_settled_mergeable_for_number`
and the schema version; `crates/react/tests/common/mod.rs` for the `FakeClient` double.

**Reused:** the `Mergeable` field on `ObservedPullRequest`, the `MergeableHistory` trait
and its `StoreMergeableHistory` bridge from slice 0015, the live mergeability re-fetch
`ProviderClient::live_merge_has_conflict`, the tri-state semantics (transient → Err →
park-and-retry), and the three-identity attribution plumbing. The schema version 2 shape
from slice 0015 already carries `mergeable`, so this slice does not bump the user_version.

**Not reused, and why:** the comment-marker idempotency path. A label is its own
idempotency signal — if the label is present we do not reapply, if it is absent we do not
remove — so adding a marker comment would be a second source of truth. The pre-write
check reads `ProviderClient::list_labels(locator)` and compares to a constant; no store
read is needed because labels are the provider's authoritative state, not ours.

**Not reused:** the `ReactionReport::Posted { agent_id }` single-slot shape. The report
widens to carry the set of writes that happened in a tick: a thin struct
`ReactionReceipt { agent_id, kind }` (where `kind` is `Commented { comment_id } |
LabelApplied { label } | LabelRemoved { label }`) and `Posted { receipts: Vec<ReactionReceipt> }`.
`AlreadyPosted` becomes `AlreadySettled` with the same receipt shape (empty receipts =
nothing to do). This is one representation change, not two (one for labels, one later for
multi-posts). The migration is confined to the react crate and the CLI's
`combine_rules`/`outcome_to_report` adapters.

**New surface on `ProviderClient`:**

- `fn list_labels(&self, locator: &PullRequestLocator) -> impl Future<Output = Result<Vec<Label>, ProviderError>>`
- `fn add_label(&self, locator: &PullRequestLocator, label: &Label) -> impl Future<Output = Result<(), ProviderError>>`
- `fn remove_label(&self, locator: &PullRequestLocator, label: &Label) -> impl Future<Output = Result<(), ProviderError>>`

`Label` is a newtype around `String` with constructor validation (nonempty, no leading or
trailing whitespace) following the pattern of `AuthorLogin` / `ThreadId` (closing the
boundary-validation gap flagged for `UserId`/`WorkerId`/`AgentId` is out of scope here but
`Label` itself is validated). The GitHub implementation uses octocrab's `issues().add_labels`
/`remove_label` — PRs are issues on the labels endpoint.

**Rule function signature:**

```rust
pub async fn react_needs_rebase_label(
    owner: &UserId,
    worker: &WorkerId,
    locator: &PullRequestLocator,
    prior: Option<&ObservedPullRequest>,
    new: &ObservedPullRequest,
    history: &dyn MergeableHistory,
    client: &dyn ProviderClient,
) -> Result<ReactionOutcome, ReactError>
```

Mirrors `react_merge_conflicts_cleared` exactly; identical history-walk logic (recognises
both directions of Mergeable↔Conflicting transitions with Unknown bridging via
`latest_settled`).

**Constants:**

- `LABEL_RULE_AGENT_ID = "rule:needs-rebase-label"`
- `NEEDS_REBASE_LABEL = "needs-rebase"` (configurability is deferred — this is the method
  we ship; three workflows before a DSL, per ADR-0004)

**Combiner:** `combine_rules` becomes `combine_rule_results(results: &[Result<ReactionOutcome, ReactError>])`
folding to a single `ReactionReport` whose receipts are the concatenation of each Ok
outcome's receipts, and whose error class is `Failed` only when every rule errored or
every write errored. A rule that posts and a rule that errors coexist: the report is
`Posted { receipts: [<the comment>] }` plus a per-rule error record. The surface for
per-rule error reporting is the smallest extension: one `errors: Vec<(AgentId, String)>`
field on `ReactionReport::Posted` and `ReactionReport::AlreadySettled`, logged but
non-fatal.

**Fail-loud posture.** Missing GraphQL `mergeable` field remains the ledgered F6 item —
this slice does not fix it, because it is orthogonal to the label write path and the
risk surface is unchanged. If a future query edit drops `mergeable`, slice 0015 and
slice 0016 silently stop firing together; the cost of the shared gap is one ledger line,
not two.

## Verification

Unit tests in `crates/react/tests/needs_rebase_label.rs`:

1. `conflict_transition_with_live_read_and_absent_label_applies_once` — DoD 1.
2. `mergeable_transition_with_present_label_removes_once` — DoD 2.
3. `conflict_transition_with_label_already_present_is_no_op` — label-level idempotency.
4. `mergeable_transition_with_label_already_absent_is_no_op` — symmetric.
5. `unknown_prior_with_mergeable_history_and_conflicting_new_applies_label` — DoD 3.
6. `unknown_prior_with_conflicting_history_and_mergeable_new_removes_label` — symmetric
   Unknown-bridge.
7. `live_read_computing_errors_so_retry_path_fires` — DoD 4.
8. `history_error_on_unknown_prior_surfaces_as_react_error` — fail-loud history read.
9. `first_observation_with_conflicting_and_no_history_does_not_fire` — no evidence of
   transition.

Combiner tests in `crates/react/tests/report_combiner.rs`:

10. `two_rule_writes_in_one_tick_both_appear_in_receipts` — DoD 5. Rule 2 posts a
    comment, rule 3 removes the label; the resulting report carries two receipts, each
    with its own agent id, neither clobbered.
11. `one_rule_posts_and_one_errors_still_reports_the_post` — the Posted side survives
    peer errors.
12. `all_rules_error_reports_failed_with_each_agent_id` — fail-loud aggregation.

End-to-end tests in `crates/react/tests/needs_rebase_label_end_to_end.rs`:

13. `conflict_cycle_applies_then_removes_across_two_ticks` — drives `run_loop_with_reactor`
    through Mergeable→Conflicting→Mergeable and asserts exactly one apply and one remove.
14. `live_fake_label_set_matches_rule_state_after_restart` — marker-equivalent: on
    restart with the label already present, a re-observation of Conflicting does not
    reapply.

CLI wiring: `missing_token_fails_loud` extended with a `ReactError::Provider` arm
asserting the add/remove-label calls under `MissingTokenClient` surface the ADR-0013
message — DoD 6.

Fake provider in `crates/react/tests/common/mod.rs` gains `labels: Mutex<BTreeSet<String>>`
with `set_labels`, `labels_snapshot`, and implements `list_labels` / `add_label` /
`remove_label` deterministically.

Quality gates per `/wrap-up`: `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt
--check`, `npm run lint`, `npm run index`.

## As built

Shipped as scoped, with three small deviations from the pre-written Design, each
recorded here rather than silently absorbed.

**The `needs-rebase` label rule.** `react_needs_rebase_label` in
`crates/react/src/lib.rs` fires on both directions of the Mergeable↔Conflicting
transition, bridging through `Unknown` via the `MergeableHistory` trait exactly as rule 2
does. The idempotency check reads the live label set via `ProviderClient::list_labels`;
no marker comment is scanned. Live-read disagreement (`live_merge_has_conflict`) blocks
the write in both directions — the apply direction will not label without a confirmed
conflict, the remove direction will not remove without a confirmed clean read.
Transient "still computing" from the live read surfaces as `ReactError::Provider` so the
slice-0014 park-and-retry contract re-attempts. Constants: `LABEL_RULE_AGENT_ID =
"rule:needs-rebase-label"`, `NEEDS_REBASE_LABEL = "needs-rebase"`.

**`Label` newtype** in `crates/domain/src/pull_request.rs`, validated at construction
(nonempty; no leading/trailing whitespace; inner whitespace allowed, so GitHub's
`help wanted` round-trips). Four unit tests pinned.

**Three new `ProviderClient` methods** (`list_labels`, `add_label`, `remove_label`)
implemented against octocrab's `issues()` surface — pull requests are issues on the
labels endpoint. `MissingTokenClient` surfaces the ADR-0013 message on each of the three.

**The `ReactionReport` surface widened.** `ReactionOutcome::Posted` now carries a
`ReactionReceipt { agent_id, kind }` where kind is one of
`Commented { comment_id } | LabelApplied { label } | LabelRemoved { label }`. The
report (`ReactionReport::Posted` / `AlreadySettled` / `Failed`) carries the concatenated
receipts from all rules that ran in a tick, plus a per-rule `errors` vec on the
non-Failed variants so a Posted write and a peer error coexist. The F4-residual ledger
item is closed by deletion in the same commit.

**Deviation 1 — combiner location.** The slice placed `combine_rule_results` in the CLI.
It landed in `crates/react/src/lib.rs` instead, so the integration tests in
`crates/react/tests/report_combiner.rs` can exercise it (binaries have no library
surface the `tests/` directory can import). The CLI imports it.

**Deviation 2 — per-rule receipt shape.** Rules emit `Posted { receipt }` (single
receipt per rule, since each rule performs one write) rather than
`Posted { receipts: Vec<_> }`. The combiner is what concatenates. Simpler at the rule
layer, same report shape at the combiner.

**Deviation 3 — `ReactionReport::Failed` is single-error.** Per-rule errors only
aggregate into the `errors` field on `Posted` / `AlreadySettled`. When every rule
errored the combiner folds them into a single `Failed { error }` string that lists each
agent id. No audit information is lost.

Definition of Done, scenario by scenario:

1. Conflict transition applies the label once — `conflict_transition_with_live_read_and_absent_label_applies_once`
   in `crates/react/tests/needs_rebase_label.rs`.
2. Mergeable transition removes the label once — `mergeable_transition_with_present_label_removes_once`.
3. Unknown-bridge in the apply direction — `unknown_prior_with_mergeable_history_and_conflicting_new_applies_label`.
4. Live-read "computing" surfaces as `ReactError` — `live_read_computing_errors_so_retry_path_fires`.
5. Two rules writing in one tick both appear in the receipts — `two_rule_writes_in_one_tick_both_appear_in_receipts`
   in `crates/react/tests/report_combiner.rs`, exercising the widened surface.
6. Missing token aborts the write path — `add_label_fails_loud_without_token` and
   `remove_label_fails_loud_without_token` in `crates/react/tests/missing_token_fails_loud.rs`.

**Verification counts.** 113 tests pass across the workspace (slice 0015 shipped with
89; the +24 are the slice-0016 additions plus a handful of symmetry and empty-input
tests the fork wrote beyond the slice's named 14). `cargo clippy --all-targets -- -D
warnings` clean, `cargo fmt --check` clean, `npm run lint` 0 error / 0 warning,
`npm run index` regenerated, `lint-secrets` ok.

**Not covered in this slice (ledgered or still open):**
- F6 (missing-field normalization) — unchanged. Dropping the `mergeable` field from the
  GraphQL query still silently stops rules 2 and 3 together; the shared fail-loud
  guardrail is one ledger line, not two.
- Boundary validation for `UserId`/`WorkerId`/`AgentId` — orthogonal to this slice;
  still ledgered.
- Multi-label configurability (`NEEDS_REBASE_LABEL` is hardcoded) — deliberate per
  ADR-0004 ("three workflows before a DSL").
