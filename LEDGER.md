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
- [audit] CI green currently proves the toolchain is wired and nothing more:
  `crates/domain` holds no types and `cargo test` runs zero tests. The badges must not be
  read as evidence of working behaviour until the first slice lands tests behind them.
- [audit] Confirm stele rule R11 (every scripts/*-verify.mjs wired into package.json)
  behaves correctly once package.json exists; the install predates it.

- [feature] Decide how provider credentials are acquired and held, before M2 performs its
  first write. Three planes need separate answers and only one of them is a "login": the
  REST/GraphQL plane needs a token (SSH keys do not authenticate the GitHub or GitLab API
  at all); the git transport plane inside a worktree can use either a token over HTTPS or
  an SSH key; commit identity is a third plane again. The candidate shape is an OAuth
  device flow for the operator's own identity (a CLI has no browser callback to listen on)
  plus a GitHub App installation token for execution, because an installation token is
  short-lived and repo-scoped and makes an agent's action attributable to the app rather
  than to the operator's account, which is what gives invariant 6 a visible counterpart in
  the provider's own UI instead of a self-reported audit field. The cost is app
  registration, a private key to hold, and JWT signing. Not decided yet because GitLab has
  no installation concept to match an App, so the credential abstraction is subject to the
  same rule as the provider abstraction: extracted on the second adapter, not guessed on
  the first (ADR-0001, ADR-0005).
- [feature] Sign agent-authored commits with an SSH signing key distinct from the
  operator's (`gpg.format = ssh`), so that "which identity produced this commit" is
  verifiable by the provider rather than asserted by our own audit record. Cheap, and it
  converts part of invariant 6 from review-only into something a third party can check
  (ADR-0005).

## Resolved

Entries move out of "Open" by deletion. A narrative of what merely *happened* belongs in
the git log; what belongs on an open item is evidence it cannot be re-derived later — a
measured result, a reproduction, a count the eventual decision turns on. Keep that with
the item while it is open, and move it into the ADR that closes it. What this file is not
is a record of completed work.
