# Ledger

The single mutable file in this repo (stele:ADR-0001). Everything else is
immutable or generated. Open work, deferrals, and known defects all live here — there is
no second tracking file, because two files require manual sync and manual sync does not
happen.

**Closing an item means deleting its line here.** Do not annotate the source ADR; the
ADR's claim ("at the time of this decision we deferred X") stays true forever and needs
no update. Single writer, one direction.

Format: `- [type] description (ADR-NNNN)` — type is `bug` | `feature` | `deferred` |
`audit`. Cite the source ADR where one exists; rule 8 checks that the citation resolves.
A decision that lives in **another** repo is cited `<repo>:ADR-NNNN` — the linter cannot
open that corpus, so it skips qualified references (stele:ADR-0009).

## Open

<!-- One line per open item. Delete the line to close it; the git log is the done-record. -->

- [deferred] Workflow DSL: parser, typed IR, interpreter. Not designed until three real
  workflows exist to generalize from; a language fitted to zero users is a guess.
- [deferred] Visual workflow editor as a projection of the DSL syntax tree, and the workflow
  simulator that replays historical events against a draft without executing actions.
- [deferred] GitLab provider adapter. The provider interface is extracted when this lands,
  not before: it is the test of whether the abstraction was right.
- [deferred] Webhook ingestion, signature validation and a hosted relay. Polling first,
  because a workstation behind NAT cannot receive webhooks and polling is deterministic
  enough to test against recorded fixtures.
- [deferred] Team model: organizations, teams, roles, approval policies, separated
  credentials. Single-operator until an automation actually works.
- [deferred] PostgreSQL and the transactional outbox. Needed when state is shared across
  machines, not before.
- [deferred] Desktop shell (Tauri/Vue). A CLI renders the same state and is assertable on
  stdout.
- [deferred] Distributed worker fleet over WebSockets, and the opt-in team-wide agent pool.
- [deferred] Container-based execution isolation. Git worktrees first; containers when an
  agent is permitted to run untrusted code.
- [deferred] Non-Git event sources: CI pipelines, Sentry, Grafana, filesystem, schedules.
- [audit] Re-run scripts/stack-cycle-probe.mjs on a quiet machine, taking medians over
  repeated runs. The 2026-11-23 attempt is not citable: load average 19.34 produced a 2.6x
  swing on identical work, and the rs+deps stage failed dependency resolution on rustc
  1.86 rather than compiling. The toolchain half is resolved (now rustc 1.100.0); the
  contention half is not. No decision may cite compile-time numbers until this is redone
  (ADR-0002).
- [bug] `scripts/build-index.mjs` escapes `|` but not `\` when rendering an ADR title into
  the `INDEX.md` table, so a title holding a backslash immediately before a pipe emits a
  live cell separator and breaks the table. Reproduced 2026-11-23: `cell('a\|b')` returns
  `a\\|b`, where the pair renders as one literal backslash and the pipe goes unescaped;
  1 of 5 test cases. Escaping backslashes before pipes fixes it. Upstream in
  `@isonimus/stele`, surfaced here by CodeQL `js/incomplete-sanitization` on the first run.
  Patching the vendored copy would diverge from the toolkit, so the fix belongs upstream:
  stele PR #1, open 2026-11-23. Close this line by re-vendoring once it is released.
- [audit] Confirm stele rule R11 (every scripts/*-verify.mjs wired into package.json)
  behaves correctly once package.json exists; the install predates it.

- [feature] Sign agent-authored commits with an SSH signing key distinct from the
  operator's (`gpg.format = ssh`), so that "which identity produced this commit" is
  verifiable by the provider rather than asserted by our own audit record. Cheap, and it
  converts part of invariant 6 from review-only into something a third party can check
  (ADR-0005).
- [bug] `UserId::new`, `WorkerId::new` and `AgentId::new` accept the empty string. The three
  newtypes are declared by the `string_id!` macro in `crates/domain/src/actor.rs`, which does
  no validation, so a persisted row can carry `owner_user=''`, `worker=''` or `agent_id=''`.
  The store's NOT NULL columns accept `''`, and the write path's owner check compares two
  possibly-empty strings. Invariant 6 says every operation names all three identities; an
  empty name is a boundary-validation gap, not a corruption bug. Found by the 2026-11-24
  adversarial review of slice 0009. Fix: validate at the constructor the way `AuthorLogin`
  and `ThreadId` already do (ADR-0005).
- [feature] Second SIGINT and in-flight fetch cancellation for `contributron watch`. The
  shutdown task awaits `tokio::signal::ctrl_c()` once and exits, so a second Ctrl-C is a
  no-op; and the per-target task does not cancel its in-flight fetch, so a stalled HTTP
  call (octocrab default timeouts bound this, but the window exists) leaves the process
  waiting until the fetch times out. Found by the 2026-11-24 adversarial review of slice
  0011; the slice shipped with the cleaner first-SIGINT-drains contract and routes the
  second-signal escalation here. Fix: replace the one-shot `ctrl_c` await with a loop
  (first signal flips cancel; second signal `std::process::exit(130)`) and wire a per-
  fetch timeout at the `Fetch` boundary (ADR-0012).
- [feature] Harden the `threads-cleared` rule's marker scan against two cases the slice
  0014 shipping semantics accept as-is: a thread reopened and re-cleared is not announced
  a second time, because the first marker comment is permanent (the rule is "at most once
  per PR, ever"), and any commenter who pastes the marker string suppresses the bot,
  because the scan does not filter by author. Fix: filter the scan to the authenticated
  user's own comments, and either accept "at most once per PR, ever" as the rule's
  definition or track a per-transition marker (slice 0014 Design notes marker-only
  idempotency; this is the extension that follows). Found by the 2026-11-24 adversarial
  review (ADR-0001).
- [feature] Persist a reaction record in the store with the three-identity tuple
  (`owner_user`, `worker`, `agent_id="rule:threads-cleared"`) alongside the pull-request
  locator and the posted comment id. Slice 0014 carries the tuple through `TickOutcome` but
  the store has no reaction surface, so invariant 6 is honoured in-process and not in
  persistence. A third rule is likely to shape the schema; until then this is the gap
  (ADR-0005).
- [feature] Anonymous capture client for public-repo read paths. The current
  `crates/github/src/capture.rs` client requires a token at construction, so `watch`
  against a public repository without `CONTRIBUTRON_TOKEN` or `--token-path` cannot run
  end-to-end even though slice 0014 scenario 5 asserts the read path keeps working. The
  CLI half (`list`, token resolution) does keep working; the network half does not. Add an
  anonymous `octocrab` constructor and plumb it through (ADR-0013).
- [feature] Paginate PR issue comments in `GitHubClient::list_comments`. The marker scan
  reads the first 100 comments only (`COMMENT_PAGE_SIZE` in `crates/github/src/client.rs`),
  matching capture's single-page review-thread pull. A pull request with more than 100
  comments whose marker sits past page 1 will be re-posted on restart, breaking the
  idempotency guarantee of slice 0014 scenario 4 for that corner. Paginate until the marker
  is found or the stream ends (ADR-0001).
- [feature] Fail loud when the GraphQL `pullRequest.mergeable` field is absent from the
  payload (not when it is present and null; null is legitimate "computing" state). The
  normalizer currently maps missing → [`Mergeable::Unknown`] via serde's `Option<Mergeable>`,
  which would silently kill the slice-0015 rule if a future query edit removed the
  `mergeable` selection. Fix: use a double-Option (`Option<Option<Mergeable>>`) or a custom
  deserializer that distinguishes "field absent" from "field null", and route the former to
  a field-named `NormalizeError::MissingField`. Found by the 2026-11-25 adversarial review
  of slice 0015 (finding 6) (ADR-0001).
- [feature] Paginate `reviewThreads`. A capture reads the first 100 threads
  (`THREAD_PAGE_SIZE` in `crates/github/src/capture.rs`), and a pull request with more cannot
  be observed at all: the normalizer refuses a payload whose `pageInfo.hasNextPage` is true
  rather than record an observation that omits the rest. Refusing is correct and it is not
  sufficient, because such a pull request is now invisible instead of half-visible. The
  silent-truncation half was found by adversarial review on 2026-11-23 and fixed; this line
  is the remaining work.

## Resolved

Entries move out of "Open" by deletion. A narrative of what merely *happened* belongs in
the git log; what belongs on an open item is evidence it cannot be re-derived later — a
measured result, a reproduction, a count the eventual decision turns on. Keep that with
the item while it is open, and move it into the ADR that closes it. What this file is not
is a record of completed work.
