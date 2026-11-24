---
id: '0009'
title: "Durable event store with deduplication"
type: slice
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# Slice 0009: Durable event store with deduplication

## Goal

Make an observation outlive the process that produced it, and make re-observing the same
state a no-op. Slice 0007 produces observations in memory and prints them; this slice writes
them to a local store keyed by the identity ADR-0008 decided on, so that two polls of a pull
request in an unchanged state leave the store with one row rather than two, and a process
restart loses nothing.

This is the second vertical cut of M1 and still not all of it. The polling loop, the
scheduler and reconciliation against provider state are the third slice. Carving it this
way keeps the write path assertable against synthesized observations rather than against a
rate-limited API, and keeps the schedule-and-retry questions out of the write path's own
tests.

The slice closes no open ledger items. It converts standing invariant 7 (persisted rows are
owner-scoped) from review-only to a wired check: there was no persistence before, so there
was nothing to verify.

## Definition of Done

- **Given** a normalized observation
- **When** it is written to the store and read back by its event identity
- **Then** the retrieved observation equals the original byte for byte, including its
  attribution

- **Given** one observation already in the store
- **When** the same observation is written a second time
- **Then** the store still holds exactly one row for that identity, the write reports it as
  a duplicate rather than erroring, and the retained attribution is the first writer's,
  because an observation's identity is its content and re-observing the same content is not
  a new fact (ADR-0008)

- **Given** one observation in the store
- **When** a second observation whose observed content differs is written, under the same
  attribution
- **Then** the store holds two rows with different event identities, so a change that the
  identity reflects is never collapsed into the earlier row

- **Given** two observations in the store, written by one process
- **When** a second process opens the same store file
- **Then** both observations read back byte-identically to the originals, so the store is
  the authority across restarts and not the process that wrote it

- **Given** a store opened under one owner identity
- **When** a row carrying a different owner is written to it
- **Then** the write fails with an error naming the mismatch, no row is persisted, and the
  check runs against whatever row the caller submitted rather than only against a wrapper
  the caller might bypass (invariant 7, ADR-0005)

- **Given** a store holding one observation
- **When** the CLI renders current state from the store
- **Then** the rendered bytes equal what the capture-based render produced for the same
  observation, so the store does not become a second encoding of the same fact

## Design

A new crate, `contributron-store`, depending on `contributron-domain` and nothing else in
the workspace. The CLI gains that dependency; `contributron-github` does not, because an
adapter has no business reading from storage and reading from storage has no business
knowing which adapter produced the row. The dependency direction is the same argument
ADR-0001 makes for the provider boundary, applied to persistence.

SQLite via `rusqlite`. The alternatives weighed:

- **A flat directory of JSON files, one per event**, keyed by the hex of the identity.
  Cheapest to build and defensible for this slice's scenarios, since dedup is an existence
  check on a filename. Rejected because the next slice needs a range read by observed-at
  (to drive reconciliation) and an append-only directory forces the consumer to open every
  file to find anything. The slice's goal is persistence that the next slice sits on; a
  format that the next slice must replace is not persistence, it is a prototype.
- **A proper database server** (PostgreSQL). Rejected for M1: the roadmap defers it to
  "when state is shared across machines, not before" and a single operator on one machine
  pays the cost of a running service with no present benefit. The ledger already carries
  the deferral.
- **sled, redb, or another embedded key-value store**. Rejected because the natural shape
  of this data is rows with secondary indices, which is exactly what SQL exists for.
  `rusqlite` is maintained, present in Cargo for years, and the schema migration story is
  conventional.

Schema, one table:

```
events (
  event_id      TEXT PRIMARY KEY NOT NULL,   -- hex of SHA-256 (ADR-0008)
  owner_user    TEXT NOT NULL,                -- invariant 7
  worker        TEXT NOT NULL,                -- ADR-0005
  agent         TEXT NOT NULL,                -- canonical JSON of ExecutingAgent, never omitted (ADR-0005)
  observed_at   TEXT NOT NULL,                -- RFC 3339, UTC, from the writing process
  payload       TEXT NOT NULL                 -- canonical JSON of the observed content
)
```

The primary key is the content digest, so duplicate detection is the database's job rather
than code's: a second `INSERT` returns `SQLITE_CONSTRAINT`, which the store maps to a
`WriteOutcome::Duplicate` return value rather than to an error. `payload` holds the same
canonical encoding the identity was derived from, so a reader can verify the identity
rather than trust it.

`observed_at` is not part of the identity: two polls of the same unchanged state produce the
same identity, and the second poll's timestamp is a fact about the poll, not a fact about
the observation. The first writer's timestamp wins, because keeping both would amount to a
log of polls and ADR-0008 already rejected that.

**On the identity collision problem.** A SHA-256 collision is not addressed. One would
require a different `payload` to hash to the same `event_id`, which is the property this
project trusts SHA-256 for in ADR-0008; if it does not hold, invariant 9's enforcement is
void across the whole system and a store-side check is not where that gets caught.

**Owner scoping.** The store is opened with a `user_id` and refuses to insert a row whose
`owner_user` differs. This is the invariant 7 enforcement: a reviewer can read "owner-scoped
rows" in the schema, but what makes it a guarantee is that the write path checks. The check
happens on the row the caller submits, not on a wrapper struct that the caller could choose
to bypass; this is what the scenario above asserts.

An ADR is written in the same commit as the implementation (`adr/0010-...md`): the storage
format is a decision all future work must obey, and two future slices already need it
(reconciliation reads by `observed_at` range, and the audit record reads by `event_id`). The
id jumps past 0009 because `slice-0009` already occupies that id in this repo's shared ADR +
slice numbering (stele:R2).

### What existing code was considered

- **`contributron-domain`'s `PullRequestObservation` and `EventId`**. Reused directly;
  the store takes an observation and derives the identity itself rather than accepting one
  computed elsewhere, so the identity in the row always matches the payload stored beside
  it.
- **`contributron-github`'s capture-file format**. Not reused: it is a provider-shaped
  payload (GraphQL envelope wrapping a response) and the store holds normalized domain
  observations. Mixing the two would put a GitHub field name into `events.payload`, which
  invariant 1 forbids.
- **Serde's default serialization order for the stored payload**. Not relied on: ADR-0008
  requires a canonical encoding, and the identity test (`crates/domain/tests/identity.rs`)
  asserts the hex digest against an independently computed value, so the storage round-trip
  is checked against that same oracle.
- **A migration framework (`refinery`, `rusqlite_migration`)**. Not pulled in: there is one
  schema and no history to migrate. A migration tool added to serve one `CREATE TABLE` is
  the speculative generality the quality bar forbids. The ledger gains an item if a second
  schema version is ever needed.

### On measured numbers

No design question in this slice has a measurable answer that would change the approach.
The row count is single-operator and bounded by the rate of pull-request activity on the
repositories the operator watches, so neither index design nor storage size turns on a
probe. Stating this explicitly is the point: the one measurement this repository owes is
the stack-cycle re-measurement, which remains a ledger item.

## Verification

Unit and integration tests, named per scenario. Each lives beside the code it exercises and
derives from the Definition of Done rather than from the implementation.

| scenario | proof |
|---|---|
| Round-trip: write then read by identity returns the original | `crates/store/tests/round_trip.rs` |
| Duplicate: a second write of the same observation is a no-op | `crates/store/tests/dedup.rs` |
| Changed content: a different observation writes a second row | `crates/store/tests/dedup.rs` |
| Cross-process: a store survives the writing process closing | `crates/store/tests/cross_process.rs`, which opens the file twice from the same test and asserts byte equality |
| Owner scoping: a row with the wrong owner is rejected at write | `crates/store/tests/owner_scope.rs` |
| CLI render from the store matches the capture-based render | `crates/cli/tests/render_from_store.rs` |

The owner-scoping scenario is the one that converts standing invariant 7 from review-only
to `verified_by:`. The slice is not done until `CLAUDE.md`'s invariant table is updated in
the freeze commit with the test file name, exactly as invariant 1's row was in slice 0007.

**Not verified by this slice, and deliberately:** nothing is polled, nothing is scheduled,
no reconciliation against provider state happens, and the store's behaviour under
concurrent writers is not tested because a single-operator CLI has at most one writer at a
time. If the polling-loop slice introduces a second writer, that is the slice that owns the
concurrency test.

## As built

<!-- Filled at merge, in past tense, confirming each Definition-of-Done scenario was met or
     recording the deviation. This is the freeze step. -->
