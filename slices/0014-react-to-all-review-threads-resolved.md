---
id: '0014'
title: "React to all review threads resolved by posting a comment"
type: slice
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# Slice 0014: React to all review threads resolved by posting a comment

## Goal

M2's first deterministic rule, and the project's first write to the provider. When the
durable store records an observation of a pull request whose `unresolved_thread_count` is
zero and the immediately prior observation of the same pull request (same owner, same
number) recorded a non-zero count, the reactor posts one comment on that pull request. The
comment carries a stable idempotency marker so a restart cannot cause a second post.

This exercises the full M1 → M2 handoff that existed in principle until now: the loop writes
to the store (M1), the reactor reads from the store and detects a transition, and the first
write to the provider leaves our boundary (M2). It also forces the credential plane
ADR-0013 settled into use: the write path now actually reads a token, which is the trigger
for promoting standing invariant 10 from `pending (LEDGER)` to `verified_by:`.

Two ledger items fold in. The first is the invariant-10 promotion itself, which ships here
because this is the first slice that reads a secret: `scripts/lint-secrets.mjs` lands with
this slice, is wired into `package.json`, and the standing-invariant table row flips in the
same commit. The second is nothing else; the SSH signing-key, UserId-validation,
second-SIGINT and thread-pagination ledger lines are orthogonal and stay open.

## Definition of Done

- **Given** a pull request with two unresolved review threads has been observed and the
  observation written to the store
- **When** a new observation of the same pull request records zero unresolved threads
- **Then** exactly one comment is posted to that pull request through the provider's
  comments endpoint, and the posted body contains the stable marker
  `<!-- contributron-rule:threads-cleared -->`

- **Given** the most recent stored observation of a pull request recorded zero unresolved
  threads
- **When** a further observation also records zero unresolved threads
- **Then** no comment is posted — the rule reacts to the transition, not to the current
  state

- **Given** no observation of a pull request exists in the store
- **When** the first observation arrives with zero unresolved threads
- **Then** no comment is posted — "all threads resolved" is a claim about a change, and
  there is no prior state to compare against

- **Given** the reactor has posted the marker comment for a prior transition and the process
  has since restarted
- **When** a further observation arrives, still with zero unresolved threads, and the
  provider reports the previously-posted marker comment still present
- **Then** no second comment is posted — idempotency holds across process restarts without
  requiring a new store column

- **Given** `CONTRIBUTRON_TOKEN` is unset and no `--token-path` is given
- **When** the reactor sees a qualifying transition
- **Then** the write-path reaction fails loud with an error naming the missing credential,
  the failure is surfaced in the tick outcome, and the read-paths (`watch`, `list`) continue
  to function

- **Given** the repository at `HEAD`
- **When** `node scripts/lint-secrets.mjs` runs
- **Then** it reports no token-shaped or key-shaped literal in any tracked file, and no
  code path writes a secret into a file inside the working tree; the script is wired into
  `package.json` and runs in CI

## Design

**Where the reactor plugs in.** The poll task, after it writes a new observation to the
store, hands the prior observation (if any) and the new one to the reactor. The reactor
decides whether this pair crosses the transition and, if so, performs the write. A separate
task tailing the store is the eventual shape when there are multiple rules firing on
different cadences; at the first rule, that is speculative generality. Rule of three: the
abstraction is extracted on the third rule, not guessed on the first (global §3,
stele:ADR-0001).

**Transition detection.** The reactor compares `prior.unresolved_thread_count()` with
`new.unresolved_thread_count()` and fires exactly when the first is non-zero and the second
is zero. No other state matters at this rule: title changes, draft flips and author changes
do not gate it. If `prior` is `None` (no previous observation for this `(owner, number)`),
the reactor never fires — a "transition" from nothing to zero is not a transition the
operator asked us to react to.

**Idempotency without a new column.** The reactor could persist "last reacted to observation
X for rule R on target T" in the store. It does not, for this slice. Instead, before posting,
the reactor lists the pull request's existing comments, scans for the stable marker
`<!-- contributron-rule:threads-cleared -->`, and skips the post if the marker is present.
This costs one extra provider API call per qualifying transition; it buys idempotency across
process restarts, store wipes, and operator-initiated manual reruns, without adding a schema
the next rule would immediately need to re-shape. Persisting reaction state in the store is a
later slice's problem and it arrives when the third rule asks a question the marker scan
cannot answer.

**Invariant 4 does not apply.** The rule reacts to an observed transition in review-thread
state — information the provider already shows in its UI without a re-fetch — and writes a
comment, which is not a sensitive operation. Mergeability, approval and branch protection
are the three things invariant 4 names specifically; thread resolution is not among them,
because a thread-resolution comment is informational, not an act that would be wrong if the
observed state had drifted. The reactor does not re-fetch before posting.

**Invariant 6 is honoured.** The write records the three identities: `owner_user` is the
account on whose behalf this installation of `contributron` is acting (single operator
today, ADR-0005); `worker` is the host identifier the poll loop already carries; `agent_id`
is a stable literal `rule:threads-cleared` for this deterministic rule. The three-identity
tuple is written to the store alongside the reaction record and included in any
`TickOutcome` surface that mentions the reaction.

**Credential handling, per ADR-0013.** The CLI accepts `--token-path <file>` or the
`CONTRIBUTRON_TOKEN` env var, never `--token <value>` (shells log argv). CLI flag precedes
env var. The file is read once at process start; the string lives in a `SecretString`-style
wrapper that redacts `Debug`/`Display`, and is held in memory for the lifetime of the
process. It is never echoed into a `TickOutcome`, never written to the store, never logged.
A missing or unreadable token aborts the write path only; the read path works on public
repositories without one. This matches ADR-0013's token-handling consequences exactly.

**What existing code was considered.** Searched the workspace for existing write-path code
and prior reactor-shaped modules (`crates/github/src/capture.rs`, `crates/github/src/`
generally, `crates/poll/src/lib.rs`, `crates/store/src/lib.rs`). The capture layer is
read-only — it fetches and normalizes, nothing writes. The poll loop drives read cadence.
The store is read/write for observations but has no reaction concept. None of these is
reused; each is a boundary the new `crates/react/` crate sits on. Deliberately not reused:
the `octocrab` client used in `crates/github/`'s capture is **not** carried into the
reactor as a shared client, because the capture client is anonymous-friendly (public repos
without a token) and the reactor's client must carry a token: they are the same library
but different constructors, and merging them would push the capture layer into a
credentialed shape it does not need (invariant 4's spirit: do not centralize authority
where it isn't required).

**Secret linting.** `scripts/lint-secrets.mjs` walks the repo at `HEAD` and asserts two
things: no tracked file contains a GitHub token-shaped literal
(`ghp_[A-Za-z0-9]{36,}`, `github_pat_[A-Za-z0-9_]{80,}`) or a PEM-key opener
(`-----BEGIN (RSA |OPENSSH |EC )?PRIVATE KEY-----`); and no Rust source in `crates/` writes
through `std::fs::write`, `std::fs::File::create`, or `tokio::fs::write` with a buffer
that was derived from a `SecretString` type (the check is syntactic — a pass means the
patterns are absent, not that the behaviour is proven impossible; the type wrapper is the
real defence). Wired as `npm run lint:secrets` and included in the `lint` composite.

## Verification

Unit and integration tests:

- `crates/react/tests/transition_fires_once.rs` — Scenario 1. A fake store seeded with a
  prior observation of two unresolved threads; a fake provider client recording writes;
  the reactor is called with (`Some(prior)`, `new_zero`) and the provider client's write
  log holds exactly one entry whose body contains the marker.
- `crates/react/tests/steady_state_does_nothing.rs` — Scenario 2. Prior zero, new zero,
  write log empty.
- `crates/react/tests/first_observation_does_nothing.rs` — Scenario 3. `None` prior, new
  zero, write log empty.
- `crates/react/tests/marker_scan_prevents_duplicate.rs` — Scenario 4. Fake client reports
  an existing marker comment; reactor is called with a qualifying transition; write log
  empty.
- `crates/react/tests/missing_token_fails_loud.rs` — Scenario 5. Reactor constructed with
  no credential source and called on a qualifying transition returns an error that names
  the missing credential; no partial write occurred.
- `crates/cli/tests/read_path_without_token.rs` — Scenario 5 (read-path half). `watch` and
  `list` invoked without `CONTRIBUTRON_TOKEN` or `--token-path` succeed against a
  public-repo fixture.
- `scripts/lint-secrets.mjs` — Scenario 6. Named here so the proof-to-criterion link holds;
  wired into `package.json`.

## As built

To be filled in at merge.

Frozen 2026-11-24 on branch `feat/m2-first-write` (commits 517c3b1..2fcdaca).

- Shipped `crates/react/` with `react()`, `ReactionOutcome`, `ReactError`, `ProviderClient`,
  `ProviderError`, `ExistingComment`, `ClientFuture`, and the constants `RULE_MARKER`
  (`<!-- contributron-rule:threads-cleared -->`) and `RULE_AGENT_ID` (`rule:threads-cleared`).
  The reactor checks the transition first, then scans existing comments for the marker, then
  posts. Scenarios 1 through 4 each ship their named test
  (`transition_fires_once.rs`, `steady_state_does_nothing.rs`,
  `first_observation_does_nothing.rs`, `marker_scan_prevents_duplicate.rs`) and pass.
- Threaded a `Reactor = Arc<dyn Fn(...) -> ReactorFuture + Send + Sync>` hook through the
  poll loop as `run_loop_with_reactor`, called from `watch` after each observation write.
  Each tick carries a `reaction: Option<ReactionReport>` with `NotApplicable`, `AlreadyPosted`,
  `Posted { agent_id }` or `Failed { error }`; `report_tick` prints one line per real outcome.
- Shipped `crates/github/src/client.rs` carrying `GitHubClient` over `octocrab::Octocrab`,
  which implements `ProviderClient` with `list_comments` (single page, up to 100 comments)
  and `post_comment` (via `.issues(owner, repo).create_comment(number, body)`). The dep arrow
  is `github → react` (no cycle: react depends on poll, not on github).
- CLI wiring in `crates/cli/src/main.rs` builds a `GitHubClient` when a token resolves, and
  a `MissingTokenClient` otherwise. The missing-token client errors on every call with an
  ADR-0013 message naming `--token-path`, `CONTRIBUTRON_TOKEN` and the git-credential
  fallback. Because `react()` checks the transition first, non-qualifying ticks never touch
  the client and never surface the credential error.
- Scenario 5 (write-path half) shipped as `crates/react/tests/missing_token_fails_loud.rs`,
  with a `NoCredentialClient` that records calls: a qualifying transition returns
  `ReactError::Provider(ProviderError::Client(_))` with a message containing both "token"
  (or `CONTRIBUTRON_TOKEN`) and "ADR-0013", and the recorded calls list is exactly
  `["list_comments"]` — the marker scan is the first and only call, no partial post leaked.
  A second test covers the non-qualifying-tick case: no transition, no client touch,
  `NotApplicable`.
- Scenario 5 (read-path half) shipped as `crates/cli/tests/read_path_without_token.rs`:
  `token::resolve_optional` with no `--token-path` and no `CONTRIBUTRON_TOKEN` env var does
  not error (either `None` or the ADR-0013 git-credential fallback is a pass), and
  `EventStore::open` → `list` → `render_state` renders a stored observation without
  touching any credential source. Deviation: the anonymous-GraphQL path against a public
  repository is not yet exercised end-to-end because the capture client has no anonymous
  constructor; ledgered.
- Scenario 6 shipped as `scripts/lint-secrets.mjs`, wired as `npm run lint:secrets` and
  into the composite `npm run lint`, and promoted standing invariant 10 from
  `pending (LEDGER)` to `verified_by:` in the same commit (f30410c). The script walks
  tracked files for GitHub token-shaped literals and PEM openers.
- ADR-0013 R-U1 token resolution unified in `crates/cli/src/token.rs` (bf3b86c): CLI flag
  precedes env var precedes the git-credential fallback, with the resolved string held in
  a `SecretString` wrapper that redacts `Debug` and `Display`.
- Deviations from the design that are now ledger items, each cited below:
  - The three-identity tuple for a reaction is carried in the `TickOutcome` surface but
    **is not persisted to the store** at this slice. The design text claimed it would be;
    it did not, because the store has no reaction surface yet. Invariant 6 is honoured at
    the in-process surface — every `Posted { agent_id }` names all three identities — but
    not yet in persistence. Ledgered.
  - Comment pagination is deferred: `list_comments` reads the first 100 comments, matching
    the review-thread single-page pull in the capture client. A pull request with more than
    100 comments can still be marker-scanned correctly only if the marker is within the
    first page. Ledgered.
  - Anonymous GraphQL for public-repo read paths (`watch` end-to-end without a token) is
    deferred for the same reason as the read-path test's deviation above. Ledgered.
- Checks at merge: `cargo test --workspace` passes, `cargo clippy --workspace --all-targets
  -- -D warnings` clean, `npm run lint` clean (lint-docs 14 docs ok, lint-secrets ok,
  build-index --check ok, check-immutable ok), `npm run index` clean.
