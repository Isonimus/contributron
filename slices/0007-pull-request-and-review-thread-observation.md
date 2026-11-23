---
id: '0007'
title: "Pull request and review thread observation"
type: slice
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# Slice 0007: Pull request and review thread observation

## Goal

Make the system able to see. Given recorded GitHub payloads, produce normalized domain
events describing open pull requests and their review threads, and render that state for an
operator. Read-only, offline in test, and with a live capture command that keeps the recorded
fixtures honest against the real API.

This is the first vertical cut of M1 and deliberately not all of it. The durable event store,
deduplication across polls, the polling loop and reconciliation are a later slice: this one
establishes the domain model, the adapter boundary and the render, which everything else sits
on. Carving M1 into three slices rather than one is the same argument ADR-0004 makes about
the rule language, applied to a milestone.

It closes one ledger item at merge, not now: the audit line recording that CI green proves
only that the toolchain is wired, since `crates/domain` holds no types and `cargo test` runs
zero tests. That line is deleted in the freeze commit, once there are tests behind the
badges. Two audit items stay open and are not addressed here: the stack-cycle re-measurement,
and whether stele rule R11 behaves with `package.json` present (this slice adds no
`scripts/*-verify.mjs`, so R11 still has nothing to catch).

## Definition of Done

- **Given** a recorded GitHub payload for a pull request carrying two review threads, one
  resolved and one unresolved
- **When** it is normalized
- **Then** the result is one pull request holding two threads, exactly one of which reports
  as unresolved, and the pull request's number, title, draft state and author are carried
  through unchanged

- **Given** the dependency graph of the workspace
- **When** `crates/domain`'s dependencies are resolved
- **Then** it depends on neither the GitHub adapter crate nor any GitHub client library, so
  the normalization boundary ADR-0001 requires is enforced by the compiler rather than by
  review

- **Given** a payload missing a field the domain model requires, or holding a value outside
  its domain
- **When** it is normalized
- **Then** normalization returns an error naming the offending field, no partially populated
  event is produced, and nothing is defaulted to a plausible-looking value

- **Given** the same recorded payload normalized twice in separate processes
- **When** the two results' event identities are compared
- **Then** they are equal, and the identity differs for any payload whose observable content
  differs, so a later slice can deduplicate on it without revisiting this one

- **Given** a normalized observation holding one open pull request with one unresolved thread
- **When** the CLI renders current state
- **Then** standard output names the pull request's number, its title and the count of
  unresolved threads, the process exits zero, and the same input renders byte-identically on
  a second run

- **Given** an observation recorded by the local operator
- **When** its audit record is inspected
- **Then** it names all three identities ADR-0005 requires, with the agent recorded as
  explicitly absent rather than the field being omitted, because an absent field and an
  operation with no agent are different facts

- **Given** a GitHub token with read access to a repository
- **When** the capture command runs against it
- **Then** it writes a fixture that the normalizer accepts without error, so a change in the
  live API's shape fails here instead of silently diverging from the recorded fixtures the
  tests trust

## Design

Three crates, and the dependency direction is the point rather than the count.

| crate | holds | depends on |
|---|---|---|
| `contributron-domain` | events, the three-way actor identity, pull request and thread types, event identity | nothing in this workspace |
| `contributron-github` | the GitHub client call, payload structs, normalization into domain types | `contributron-domain` |
| `contributron-cli` | the binary, the render, the capture command | both |

Standing invariant 1 says no domain type names a provider concept. As a convention that is a
review question; as a crate boundary it is a compile error, because `contributron-domain`
cannot name a type it has no dependency on. That is the whole reason the adapter is a crate
rather than a module, and it is why the workspace does not stay flat here despite
`Cargo.toml`'s preference for flatness: this boundary has earned itself.

Normalization is a pure function from a deserialized payload to a `Result`. It touches no
clock, no filesystem and no network, so every scenario above except the capture one is
assertable offline against fixtures, which is what lets this slice be iterated against
`cargo test` rather than against a rate-limited API.

Event identity is derived from observable content rather than assigned, so two processes
normalizing the same payload agree without coordinating. Deriving it here, before any store
exists, is cheaper than retrofitting it under one.

### What existing code was considered

- **The GitHub client.** `octocrab` is the established Rust client and is proposed by name
  per stele:ADR-0025. The alternatives weighed: hand-rolling over `reqwest`, rejected because
  pagination, conditional requests and rate-limit handling are exactly the tedium a
  maintained client has already got right; and `hubcaps`, which is unmaintained. Note that
  `octocrab`'s transitive dependencies already required rustc 1.88 when ADR-0002 was written,
  which is part of why the toolchain floor is 1.100.
- **`octocrab`'s own model types as domain types.** Considered and rejected. It is the
  shortest path and it would put `octocrab::models::pulls::PullRequest` in the domain, which
  is precisely the leak ADR-0001 exists to prevent and would make the GitLab adapter a
  rewrite rather than an addition.
- **`serde_json::Value` instead of typed payload structs.** Rejected: it moves every missing
  field from a deserialization error to an `Option` unwrapped somewhere later, which is the
  "fail loud" rule inverted.
- **Error types.** `thiserror` for the per-crate error enums. Deliberately not `anyhow` in
  library crates: a caller that must distinguish a missing field from a transport failure
  needs variants, not a boxed message. `anyhow` is acceptable in the binary's `main`.
- **Rendering.** Plain `std::fmt` and `clap` for argument parsing. No TUI crate: the roadmap
  defers any interface beyond a CLI, and a render assertable as a string is the property this
  slice needs.
- **Nothing in this workspace was available to reuse**, since `crates/domain` is empty by
  design. That is stated rather than left implied, because "nothing to reuse" and "did not
  look" produce the same diff.

### On measured numbers

No design question in this slice has a measurable answer that would change the approach, so
none is cited. Stating that explicitly is the point: the one measurement this repository owes
is the stack-cycle re-measurement, which is a ledger item and is not a precondition for this
work. No number from the invalidated 2026-11-23 probe run is relied on here.

## Verification

Unit tests, named per scenario. Each lives beside the code it exercises, and each derives
from the Definition of Done above rather than from the implementation.

| scenario | proof |
|---|---|
| Normalization carries a pull request and two threads through | `crates/github/tests/normalize.rs`, fixture `pull_request_with_threads.json` |
| The domain crate cannot see the provider | `crates/domain/tests/boundary.rs`, asserting over `cargo metadata` output that the resolved dependency set for `contributron-domain` contains no GitHub crate |
| A missing or out-of-domain field errors, naming the field | `crates/github/tests/normalize.rs`, fixtures `missing_number.json` and `unknown_thread_state.json` |
| Event identity is stable across processes and content-sensitive | `crates/domain/tests/identity.rs` |
| The render names number, title and unresolved count, and is deterministic | `crates/cli/tests/render.rs` |
| The audit record names all three identities, agent explicitly absent | `crates/domain/tests/actor.rs` |
| The live API still matches the recorded fixtures | `cargo run -p contributron-cli -- capture --repo <owner/name>`, run by hand against a real token |

The last row is the one a unit test cannot assert, since it depends on a live third party.
Stele's rule R11 wants such a check as a `scripts/*-verify.mjs` wired into `package.json`;
here it is a Cargo subcommand instead, because ADR-0002 keeps application code out of Node
and a Node script shelling into the Rust binary would add a layer that proves nothing extra.
R11 is satisfied trivially rather than circumvented: this slice adds no `scripts/*-verify.mjs`
for it to check. The ledger carries the audit item for confirming R11's behaviour either way.

**Not verified by this slice, and deliberately:** nothing is deduplicated, nothing is
persisted, nothing is polled, and no rule fires. A green suite here means the system can see
and describe one repository's pull requests from a recording, and nothing more.

## As built

<!-- Filled at merge, in past tense, confirming each Definition-of-Done scenario was met or
     recording the deviation. This is the freeze step. -->
