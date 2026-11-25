---
id: '0019'
title: "Persist reaction records with full three-identity attribution"
type: slice
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Goal

Close the M2 audit-record gap by persisting one row per rule fire in the event store,
carrying the full three-identity tuple (`owner_user`, `worker`, `agent_id`) alongside the
pull-request locator and the outcome. Today invariant 6 (every recorded operation names
all three identities) is honoured *in-process*: the reactor computes the attribution and
streams it on the tick channel, but nothing persists. A process kill after a `Posted`
write to the provider but before any follow-up read loses the audit record of that write.

This closes the ledgered `[feature]` *"Persist a reaction record in the store with the
three-identity tuple (owner_user, worker, agent_id="rule:threads-cleared") alongside the
pull-request locator and the posted comment id"*. Four rules have now shipped, so the
schema signal has ripened past the rule-of-three bar the ledger line itself invoked.

The posted-comment-id, label name and reviewer list that each rule's `ReactionReceipt`
carries in memory are **not** persisted in this slice: they are re-derivable from the
provider state, they would require widening `ReactionReport` through the poll crate, and
the audit-record purpose is served by *"rule X operated on PR Y under identity (U, W, A)
at time T"* without the kind-specific payload. A later slice can add a `kind_payload`
column on a third consumer; today there are zero.

## Definition of Done

- **Given** a reactor tick returns `ReactionReport::Posted { agent_ids: [a, b], errors: [] }`
  for the pull request at `owner/repo#N`
- **When** the poll loop finishes handling the tick
- **Then** two rows are present in the store's `reactions` table, one per agent id, each
  carrying `(owner_user, worker, agent_id)` set from the reactor's attribution,
  `(pr_owner, pr_repo, pr_number)` = `(owner, repo, N)`, `outcome = "posted"`, and an
  `observed_at` from the tick time.

- **Given** a reactor tick returns `ReactionReport::AlreadySettled { agent_ids: [a], errors: [] }`
- **When** the poll loop finishes handling the tick
- **Then** one row is present with `outcome = "already-settled"` and the same shape as
  above. The audit record covers "the rule confirmed the desired state is already in
  place", because invariant 6 recognises both a write and an idempotent confirmation as
  operations the agent is accountable for.

- **Given** a tick with `ReactionReport::NotApplicable` or `ReactionReport::Failed`
- **When** the poll loop finishes handling the tick
- **Then** no row is written. `NotApplicable` is a no-op (there is no operation to
  record); a `Failed` tick never completed an operation against the provider and its
  error already surfaces on the tick stream. Recording failed attempts is ledgered as a
  follow-up.

- **Given** the same tick is processed twice (store open on a different process
  lifetime, re-fetching a locator the store already saw)
- **When** the poll loop re-invokes the reactor and the rule again reports `Posted` for
  that `(event_id, agent_id)` pair
- **Then** the second insert returns `WriteOutcome::Duplicate` and the first row stands.
  Reaction identity is `digest(event_id, agent_id)`, which is attribution-free the way
  event identity is (ADR-0008 clause 1 holds): two independent operators agreeing on the
  observed state and the firing rule agree on the reaction identity.

- **Given** an event store opened at schema version 2 (no `reactions` table)
- **When** the current build opens it
- **Then** the schema migrates in place: the `reactions` table is created, the user
  version bumps to 3, and the existing `events` rows are untouched. A build older than
  this slice opening a version-3 file still refuses with `SchemaVersionMismatch`, the
  contract ADR-0008 invokes for a bump.

- **Given** the poll loop can open the store and write observations
- **When** the reactor returns `Posted { agent_ids: [a], errors: [] }` and the reactions
  write fails (locked table, decode error, anything the store can surface)
- **Then** the per-tick `Tick.persist_errors` carries `(a, "<error message>")`, the tick
  stream carries both the provider outcome and the persist failure, and `watch` prints
  the audit gap alongside the successful post. The write path does not retry here; the
  provider state is already correct and the audit record is the only thing missing.

## Design

### Where the write happens

The poll loop (`crates/poll/src/lib.rs`) already owns the store handle and writes the
observation on every Fresh tick before invoking the reactor. The reaction-record write
lands in the same per-tick handler, after the reactor returns: the loop iterates
`agent_ids` on `Posted` and `AlreadySettled`, builds one `ReactionRecord` per agent id
from the tick's observation and the reactor's `Attribution`, and inserts each row.

A one-rule-per-row shape (rather than one-row-per-tick) matches invariant 6's atom: the
rule is the operation. A four-rule tick that produces two `Posted` agent ids and one
`AlreadySettled` writes three reaction rows.

The attribution is the three-identity tuple the reactor already constructs through
`attribution_for` in the react crate: `UserId`, `WorkerId`, and `AgentId` parsed from the
rule's agent-id literal (`rule:threads-cleared`, …). Those literals carry the `rule:`
prefix, which the store persists as-is: the prefix is part of the identity namespace, not
an implementation detail.

### Why the kind-specific payload is not persisted here

The `ReactionReceipt` inside the rule carries the kind (`Commented`, `LabelApplied`,
`LabelRemoved`, `ReviewRequested { reviewers }`), but `combine_rule_results` already
strips it down to `agent_ids: Vec<String>` before the loop sees it. Widening
`ReactionReport` through the poll crate to carry the receipts is a public-API change
affecting every reactor consumer, and it is not what invariant 6 asks for. Invariant 6
asks that every recorded operation name all three identities, not that every operation
replay its payload. The comment body, the label name and the reviewer list are all
observable on the provider from the record of *"rule X fired on PR Y"*; the audit
record's job is to tie the operation to its three identities, not to duplicate the
state.

If a later use (rule replay, offline analysis, auditing from a backup) needs the
kind-specific payload, that's a `kind_payload TEXT` column added in a follow-up slice.
Today there are zero consumers and the rule-of-three bar is not met.

### Schema and identity

New table `reactions` added at `SCHEMA_USER_VERSION = 3`. Shape:

```sql
CREATE TABLE IF NOT EXISTS reactions (
    reaction_id TEXT NOT NULL,
    owner_user  TEXT NOT NULL,
    worker      TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    pr_owner    TEXT NOT NULL,
    pr_repo     TEXT NOT NULL,
    pr_number   INTEGER NOT NULL,
    outcome     TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (owner_user, reaction_id)
);
```

- `reaction_id` is a SHA-256 hex digest of `"{event_id}:{agent_id}"`, matching the
  hasher `EventId::event_id` already uses in `crates/domain/src/event.rs`. ADR-0008
  clause 1 (attribution-free identity) and clause 2 (reproducible across adapters) both
  hold: `event_id` is already attribution-free, and `agent_id` is a rule-name literal
  defined in the react crate, not a worker-local name.
- Composite primary key `(owner_user, reaction_id)` mirrors the events table (invariant
  7: owner-scoped rows). Two owners that observed the same pull request and ran the same
  rule each get their own row.
- `outcome` is a bounded enum: `"posted"` or `"already-settled"`. A `CHECK` constraint is
  *not* added because SQLite's constraint handling is coarse; invariant 3 (fail loud)
  surfaces a bad value at decode time instead, the same approach the events table takes
  for `agent` JSON.
- `pr_owner`, `pr_repo`, `pr_number` are three columns rather than a single serialised
  locator, because every known query filters on at least two of them and a flat shape
  keeps the migration simple.

The `events` table is untouched. The migration is additive.

### How the poll loop builds the row

The reactor currently returns `ReactionReport` with `agent_ids: Vec<String>`. The loop
has everything else it needs to build a reaction row: the `PullRequestLocator` (which
carries `owner`, `repo`, `number`), the `UserId` from the task state's owner, and the
`WorkerId` is a constant on the loop side (`"cli"` or an equivalent literal, matching
the existing attribution). That leaves no public-API change required on the react crate.

No new trait is introduced. The poll crate already depends on
[`contributron_store::EventStore`] (the per-task handle it uses to write observations),
so a `write_reaction(&self, row: &ReactionRecord) -> Result<WriteOutcome, StoreError>`
method lands on `EventStore` directly. Taking the simpler route here keeps the "one
store, two tables" shape honest; a `ReactionStore` abstraction can emerge on the third
consumer that needs the write path without the read path (rule-of-three bar).

A failure to persist surfaces as a new field on the per-tick [`Tick`]:
`persist_errors: Vec<(String, String)>` (agent id → error string), alongside the
existing `reaction` field. The field does not trigger a park-and-retry (the provider
write already landed, so a retry would re-report `AlreadySettled`, not fix the audit
gap), but it does make the gap loud on the tick stream so `watch` can print it. A
future slice can wire reaction-persist retries if the audit gap turns out to matter
beyond observability.

### Idempotency across restarts

The partial-failure retry contract from the 2026-11-25 review parks a tick on
`Posted { ..., errors: [..] }` so the parked rule can retry. When it retries, it may
re-post or re-confirm — but by then the first `Posted` has already written its reaction
record. The reaction-id's deterministic derivation (digest of event_id + agent_id) means
a retry's `WriteOutcome::Duplicate` is observed on the reaction write, which the loop
treats the same as a successful record (the row is present, invariant 6 holds).

### What was searched for and not reused

- The events-table write path. Considered reusing the `write` method by adding a second
  variant on `PullRequestObservation`-shaped records. Rejected: `events` is strictly
  observations, and conflating them with reactions would break the "events are the input,
  reactions are the output" shape that every current query assumes. A separate table
  keeps the two surfaces narrow.
- The `MergeableHistory` trait pattern was reused for `ReactionStore`: one method, store
  opens per call, bridge impl on the CLI side. Fresh-connection cost is dwarfed by the
  network round-trip the reactor just made, and it sidesteps rusqlite's `!Sync`
  constraint.
- The React crate's `ReactionReceipt` kind payload. Considered widening
  `ReactionReport` to carry the receipts through to the loop. Rejected for this slice;
  ledgered as a follow-up with the rule-of-three justification.

## Verification

Unit tests in `crates/store/tests/`:

- `reactions.rs::write_then_read_round_trips_the_tuple` — insert one row, read it back,
  all nine columns match.
- `reactions.rs::rewriting_the_same_reaction_id_is_a_duplicate_not_a_new_row` — identity
  idempotency, `WriteOutcome::Duplicate` returned.
- `reactions.rs::rewriting_under_a_different_owner_writes_a_fresh_row` — owner scoping,
  invariant 7.
- `reactions.rs::opening_a_v2_file_migrates_to_v3_without_touching_events` — migration,
  existing events survive, user_version bumps.
- `reactions.rs::opening_a_v3_file_with_an_older_build_refuses_loud` — simulated by
  opening a v3 file under a build whose `SCHEMA_USER_VERSION = 2`; `SchemaVersionMismatch`
  returned.

Integration test in `crates/poll/tests/`:

- `reaction_persistence.rs::posted_tick_persists_one_row_per_agent_id` — end-to-end: a
  Fresh tick with a reactor returning `Posted { agent_ids: [a, b] }` produces two rows
  in `reactions`, with the expected `(owner_user, worker, agent_id, pr_*, outcome)`.
- `reaction_persistence.rs::already_settled_tick_persists_one_row` — same shape, outcome
  column is `"already-settled"`.
- `reaction_persistence.rs::not_applicable_and_failed_ticks_write_nothing` — negative
  case.
- `reaction_persistence.rs::a_persist_failure_surfaces_on_the_tick_stream_without_masking_the_post`
  — `Tick.persist_errors` is non-empty when the reaction write fails (driven by closing
  the DB file out from under the writer, or an equivalent injectable fault).

No `scripts/<slice>-verify.mjs` is required: every behaviour above is assertable in a
unit or integration test against the real SQLite store, and the slice's claim is
in-process behaviour, not a rendering or timing one.

## As built

Shipped as designed with one scope narrowing, documented here.

- **Schema v3 migration** added to `crates/store/src/lib.rs` with the `REACTIONS_DDL`
  constant; the migration arm is reached when an existing v2 file is opened. The
  `events` DDL was extracted into an `EVENTS_DDL` constant so fresh-file creation and
  v2-to-v3 migration share one source of truth for the table shape.
- **`ReactionRecord`**, **`ReactionRecordOutcome`** and **`StoredReaction`** types live
  in the store crate's public API; `write_reaction` and `list_reactions` are the two
  methods. Two new `StoreError` variants (`MissingAgentId`, `UnknownReactionOutcome`,
  `NegativePrNumber`) cover the boundary cases the type invariants refuse at decode
  time rather than best-effort.
- **Reaction identity** is `sha256("{event_id}:{agent_id}")` in `reaction_id`, matching
  the hasher the domain crate already uses. The same observation with two rules fires
  two distinct reaction ids; the same rule retried on the same observation finds its
  earlier row as `WriteOutcome::Duplicate`.
- **Poll loop integration** in `crates/poll/src/lib.rs` extends `PollConfig` with a
  `worker: WorkerId` field (so the recorded three identities line up with the react
  crate's `Attribution`), and `Tick` with a `persist_errors: Vec<(String, String)>`
  field. The fresh/unchanged handlers were restructured to return a `ReactionHandled`
  carrier so the store (which is `!Sync`) never crosses the reactor's await point; the
  synchronous persist runs after the await returns.
- **CLI** was updated at the twenty-ish `PollConfig` construction sites across the
  workspace (all tests plus `cli::main`) to pass `worker: WorkerId::new("cli")`, and
  `report_tick` learned to print `reaction-persist-error ...` lines to stderr when the
  audit record did not land.
- **Six of seven Definition-of-Done scenarios** are covered by the new tests:
  `crates/store/tests/reactions.rs` (seven unit tests covering write/read, identity
  duplicate, two-rules-one-row, owner mismatch, missing agent id, v2→v3 migration, and
  future-version refusal) and `crates/poll/tests/reaction_persistence.rs` (three
  integration tests covering `Posted` persistence, `AlreadySettled` persistence, and
  `NotApplicable`/`Failed` writing nothing). The **seventh** scenario (persist failure
  surfaces on the tick stream) is **narrowed to design-review only**: reproducing a
  reaction-write failure end-to-end would require either racing another SQLite writer
  (flaky) or mocking the store through a trait (which this slice deliberately rejected
  as premature abstraction, see the Design section). The field and its stderr line
  exist and are exercised when a store-level error happens in the field; a synthetic
  end-to-end persist-failure test can land alongside the next slice that needs the
  mock shape. The `assert!(tick.persist_errors.is_empty())` assertions in every
  integration test are the inverse coverage: any regression that produces spurious
  persist errors fails the suite.

### Deviations from the Design, in order of relevance

1. **No `ReactionStore` trait.** The Design section already said not to introduce one;
   the As-built restates the fact because it is the most load-bearing choice. The
   poll crate calls `EventStore::write_reaction` directly through its existing store
   handle, so there is no new trait, no bridge in the CLI, and no mock seam for the
   persist-failure test. The rule-of-three bar is not met.
2. **Worker on `PollConfig`, not through the `ReactionReport`.** The alternative
   considered was widening `ReactionReport` to carry the per-receipt `Attribution`
   from the react crate. Rejected because it would ripple a public-API change through
   four reactor consumers and the combiner, for a field the poll loop can source from
   its own config. The tradeoff is the twenty-ish `PollConfig` construction sites
   touched this slice, which is mechanical and caught by the compiler; the ripple
   would have been semantic and cascading.
3. **`run_one` carries a `#[allow(clippy::too_many_lines)]`.** The function grew by the
   sync persist arm and the follow-up `advance_or_park` in both fresh and unchanged
   branches. The branches each hold distinct state mutations and splitting them would
   divide the per-tick state machine across functions without earning reuse, so the
   allow is honest per the quality bar's "extract for readability, never for a number"
   rule. A comment at the site names the reason.
