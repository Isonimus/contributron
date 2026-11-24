---
id: '0010'
title: "Local SQLite is the durable event store for M1"
type: architecture
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# ADR-0010: Local SQLite is the durable event store for M1

## Context

M1 is observation: every pull request and review thread the operator wants to watch must
survive the process that produced it, and re-observing an unchanged state must be a no-op.
Slice 0007 produced observations in memory and printed them; slice 0009 persists them.

ADR-0008 already decided that an event's identity is a SHA-256 digest of its observed
content: redelivery is the same event, and attribution does not change it. That decision is
the one this ADR builds the storage schema around.

The roadmap defers PostgreSQL to "when state is shared across machines, not before" (see
[docs/roadmap.md](../docs/roadmap.md), Locked decisions). The ledger carries that deferral.
The question this ADR answers is what fills the gap on a single operator's workstation, now
that something must.

## Decision

A new crate, `contributron-store`, persists [`PullRequestObservation`][obs] values in a
local SQLite file via `rusqlite` with the `bundled` feature. The schema is one table:

```sql
CREATE TABLE events (
  event_id    TEXT PRIMARY KEY NOT NULL,
  owner_user  TEXT NOT NULL,
  worker      TEXT NOT NULL,
  agent       TEXT NOT NULL,
  observed_at TEXT NOT NULL,
  payload     TEXT NOT NULL
);
```

Columns:

- `event_id` is the hex of the content digest from ADR-0008. Making it the primary key is
  how deduplication becomes the database's job: a second `INSERT` returns
  `SQLITE_CONSTRAINT_PRIMARYKEY`, which the store maps to
  `WriteOutcome::Duplicate` instead of to an error.
- `owner_user`, `worker`, `agent` carry the three identities from ADR-0005. `agent` stores
  the JSON of `ExecutingAgent` so the two variants (`NoAgent` and `Agent { id }`) are
  distinguishable without a sentinel string that an agent id could collide with.
- `observed_at` is RFC 3339 UTC, written by the writing process, and is deliberately not
  part of the identity. Two polls of the same unchanged state produce the same identity,
  and the second poll's timestamp is a fact about the poll (ADR-0008), not about the
  observation.
- `payload` is the same canonical JSON encoding the identity was derived from, so a reader
  can verify the identity rather than trust it.

The store is opened with a `UserId` and refuses to insert a row whose `owner_user` differs
from it. This is how standing invariant 7 (persisted rows are owner-scoped) moves from a
review-only convention to a wired check: the write path inspects the submitted row's
attribution, not a wrapper struct a caller could sidestep.

The crate depends on `contributron-domain` and on nothing else in the workspace. The CLI
gains the dependency; the GitHub adapter does not. The dependency direction is the same
argument ADR-0001 makes for the provider boundary, applied to persistence: a storage layer
that knows which adapter produced a row, or an adapter that knows how a row is persisted,
is a boundary leak the next adapter would pay for.

No migration framework is introduced. There is one schema and no history to migrate from;
a framework added to serve one `CREATE TABLE` is the speculative generality the quality
bar forbids. If a second schema version is ever needed, the ledger gains an item and this
ADR is superseded.

The `bundled` feature compiles SQLite from source rather than linking a host library, so
the build is reproducible across machines and CI containers without a `sqlite3-dev`
dependency on the host. Reproducibility between the operator's workstation and CI is the
property this chooses; a shared system SQLite is not worth sharing.

## Consequences

For later work:

1. The polling loop (M1's third slice) reads by `observed_at` in a range to drive
   reconciliation. The column is present and indexable; an index is added at the slice that
   needs it, not now.
2. The audit record (M2) reads by `event_id`, which is already the primary key. No further
   schema design is needed for it.
3. A second provider adapter inherits the same schema. The GitHub adapter is not a
   dependency of the store, so adding GitLab or another source changes no column.
4. A second owner (team model, deferred on the roadmap) is a schema-compatible change:
   every row already carries `owner_user`, and every query is scoped by it.

For the existing code:

- The CLI gains a `list --store <path>` subcommand that prints observations from the store
  using the same renderer as `render --capture`. The slice's scenario 6 asserts both
  produce byte-equal output.
- Workspace `Cargo.toml` gains `rusqlite`, `time` and `tempfile` entries. `time` carries
  only the `formatting` feature so RFC 3339 output works without pulling in parsing or
  macros the store never uses. The JWT backend that `octocrab` would need for a GitHub App
  is still omitted, per ADR-0002's dependency floor.
- Standing invariant 7's enforcement column in [CLAUDE.md](../CLAUDE.md) moves from
  `review-only` to `verified_by: crates/store/tests/owner_scope.rs` in the same commit as
  this ADR.

## Alternatives considered

**A flat directory of JSON files, one per event**, keyed by the hex of the identity.
Cheapest to build; deduplication is an existence check on a filename, which is correct for
this slice. Rejected because the polling-loop slice needs a range read by `observed_at`,
and an append-only directory forces the consumer to open every file to find anything. The
store is meant to be what the next slice sits on; a format the next slice replaces is a
prototype, not persistence.

**PostgreSQL now, instead of SQLite.** Rejected for M1. State is not shared across
machines yet, so a running service is weight without a user. The deferral is already a
ledger line, the ledger line is already the plan for when this stops being true, and
nothing in the current design makes that transition harder: the SQL is standard, the
`event_id`/`owner_user`/`payload` columns are server-agnostic, and no SQLite-specific
feature is in use.

**An embedded key-value store (sled, redb).** Rejected because the natural shape of this
data is rows with secondary indices. `observed_at` range reads and per-owner listings are
what SQL exists for. The ergonomic argument for a key-value store collapses once the
second index is needed.

**A migration framework at slice 0009 time.** Rejected on the quality bar: no second
schema exists to migrate from, so a framework earns its place when it earns it. The
hand-written `CREATE TABLE IF NOT EXISTS` is one statement and ships with one test.

**Storing attribution in `payload` rather than in named columns.** Rejected because
owner-scoping queries would then require parsing JSON on every row read, which is the
database's job inverted. Columns stay flat so the index on `owner_user` (implicit from the
query scoping) works.

**Making `observed_at` part of the identity.** Rejected by ADR-0008: two polls of the same
unchanged state are one observation, not two; a timestamp in the identity makes every poll
distinct and pushes the dedup problem into the layer above the store. This ADR consumes
that decision rather than re-litigating it.

## Review-only invariants that stay review-only after this ADR

- Invariant 1 (no domain type names a provider concept): still enforced by the dependency
  graph; this ADR adds a new crate that depends only on the domain.
- Invariant 6 (every operation names all three identities): now enforced on persisted rows
  too, because the three columns are `NOT NULL`. The in-memory enforcement remains in the
  domain type.

[obs]: ../crates/domain/src/event.rs

## Amendment — 2026-11-24: per-owner deduplication and schema versioning

Adversarial review of the initial implementation found that making `event_id` the sole
primary key collapses the per-owner scoping on the write path. Two owners sharing a store
file would, on the same observed content, compute the same `event_id` (ADR-0008 excludes
attribution from the identity); the second write would hit the primary-key conflict and be
reported as `Duplicate`, and every read path being scoped by `owner_user` would then leave
the second owner with no row at all. Silent data loss across the owner boundary.

Two changes correct that and one guards against a future class of the same mistake:

- The schema's primary key is `(owner_user, event_id)`, not `event_id` alone. Dedup is
  per-owner, which is what invariant 7 means in practice.
- The write path's "this is a duplicate" match arm catches only
  `SQLITE_CONSTRAINT_PRIMARYKEY` (extended code 1555). The bare `SQLITE_CONSTRAINT` (19)
  arm the first version carried is removed: a future NOT NULL, CHECK or UNIQUE violation
  must fail loud rather than be silently reclassified as "already recorded".
- `PRAGMA user_version` is written on first create and checked on every open. A file whose
  version does not match is refused with `StoreError::SchemaVersionMismatch`, because the
  encoding assumptions of this ADR do not apply to a file written under a different one.

The Decision block's schema diagram and its description of the key still show the
pre-amendment shape. That is left as it was: this document records what was decided at each
point, and the correction here is the current state. The code matches this amendment; the
amendment is what a reader implements against.
