---
id: '0013'
title: "Operator personal access token for M2; GitHub App deferred to the second adapter"
type: architecture
status: accepted
date: 2026-11-24
supersedes: []
superseded_by: []
---

# ADR-0013: Operator personal access token for M2; GitHub App deferred to the second adapter

## Context

M2 performs the first write to the provider (the deterministic-reaction milestone). That
write needs a credential, and until this ADR the question of *which* credential, acquired
*how*, has been a ledger line deferring the decision. The decision cannot wait past the
first write because it decides what the write is attributable to in the provider's own UI
and how invariant 6 (every operation names all three identities) gets its visible
counterpart.

Three planes need separate answers. Only one is the "login" plane.

1. **REST/GraphQL API.** How `contributron` authenticates to the GitHub API, both for the
   polling loop that already ships and for the writer M2 introduces. SSH keys do not
   authenticate the API at all; it is a token or nothing.
2. **Git transport inside a worktree.** How `contributron` pushes and fetches during agent
   execution, which is M3 work. Either a token over HTTPS or an SSH key.
3. **Commit identity.** Which identity a commit is signed with, so that "which identity
   produced this commit" is verifiable by the provider rather than asserted by our own
   audit record. This is orthogonal to planes 1 and 2 and is tracked as its own ledger
   line.

Plane 1 has three candidate shapes:

- **A1: Operator personal access token (fine-grained), supplied at invocation time.** The
  operator mints a token in GitHub's UI, scopes it to specific repositories and
  permissions, and `contributron` reads it from a path or environment variable. No OAuth
  flow, no App registration, no JWT signing. The token is long-lived and tied to the
  operator's whole account; actions are attributable only through invariant 6's
  self-reported `agent_id`.

- **A2: OAuth device flow for operator identity.** A CLI has no browser callback to listen
  on, so device flow is the correct OAuth shape. The operator runs a `login` command,
  visits a short URL and pastes a code. Token refresh is handled by the client. Still the
  operator's own account acting, so the attribution weakness of A1 remains while the
  machinery is significantly larger.

- **A3: GitHub App installation token.** An App that Claude's operator registers once and
  installs on each repository `contributron` is permitted to touch. `contributron` holds
  the App's private key, mints a short-lived JWT, exchanges it for a one-hour installation
  token scoped to those repositories, and refreshes before expiry. Actions appear in
  GitHub's UI as the App, not as the operator's account, which is what gives invariant 6 a
  provider-side counterpart.

A3 is where this eventually goes. Short-lived, repo-scoped, revocable at the provider
without touching the operator's account, and attributable at the provider's UI rather than
by our own audit record alone. The question this ADR answers is whether to pay that cost
*now* or later.

## Decision

**M2 uses A1: an operator-minted fine-grained personal access token, supplied to the CLI
by a path or environment variable, never committed to the repository and never written to
any config file.**

A3 is the eventual shape and is deliberately not built for M2. The reason is the rule
ADR-0001 and ADR-0005 both state: a provider abstraction is extracted on the **second**
adapter, not guessed on the first. A GitHub App is a GitHub-specific object. GitLab has no
installation concept and the shape that works for GitHub installation tokens is not the
shape that works for GitLab project access tokens. Building the credential model around
one provider before the second one has forced the question would repeat the mistake
ADR-0001 was written to prevent.

The attribution weakness of A1 (an action is attributable to the operator's account rather
than to `contributron` as an identifiable actor in the provider's UI) is real. Invariant 6
exists to carry that weight at review time while a provider-side counterpart is absent.
A3 is where part of that invariant moves from review-only to provider-verifiable; today
it stays review-only.

The token is handled as follows:

- The operator supplies it by `--token-path <file>` or `CONTRIBUTRON_TOKEN` env var, never
  as a `--token <value>` flag (shells log argv). Precedence: CLI flag over env var.
- The file is read at process start and the value is held in memory only. It is never
  logged, never written to the store, never echoed in a `TickOutcome`.
- A missing or unreadable token aborts the write path with a clear error. The read path
  (`contributron watch`, `list`) does not require a token for public repositories, so it
  stays usable without one.

## Consequences

1. **M2 can start.** The credential line in `LEDGER.md` is closed by this ADR (deleted
   from the ledger per the single-writer rule). The SSH-signing ledger line (plane 3)
   stays open: it is orthogonal and this ADR does not decide it.
2. **Attribution in the provider UI is the operator.** A row we write is attributed to
   the operator's account at the provider, and the `agent_id` field (invariant 6) is the
   only place "what actor produced this" is recorded. This is one of the review-only
   halves of invariant 6 until A3 ships.
3. **A future provider adapter forces the credential abstraction.** When the second
   adapter lands (GitLab is the line in the ledger), the shape of "operator supplies a
   token out-of-band" is almost certainly common to both and the shape of "App
   installation vs. project access token" is not. The common shape is extracted *then*,
   from two instances, per the rule above.
4. **The App registration cost is deferred, not avoided.** A3 is where invariant 6's
   attribution gains a provider-side witness. A later ADR supersedes this one when either
   (a) the second adapter arrives and the credential abstraction is extracted, or
   (b) a feature that reasonably demands provider-visible agent attribution ships before
   that. The replacement is a new ADR, not an edit to this one.
5. **No secret ever lands in a config file in this repo.** Token handling is standing
   invariant 10 below. The reason is categorical (secrets in git history are hard to
   recall even after a rotation) and the rule is cheap enough that it costs nothing to
   state up front.

## Alternatives considered

**A2: OAuth device flow now.** Rejected for M2. It inherits A1's attribution weakness
(still the operator's account acting) while paying the OAuth client-registration and
refresh plumbing cost, which is close to the plumbing cost of A3. If we are paying that
cost, A3 is the shape that also pays off at invariant 6; A2 pays it and does not. A later
decision may still land on device flow as the way the operator obtains the token that
plane 1 reads from, as a user-experience improvement on A1's "paste a file path" — that is
an operator-convenience question, not the credential-model question this ADR settles, and
is a separate ADR when it arrives.

**A3: GitHub App now.** Rejected on the same rule that rejects every other speculative
abstraction here: a GitHub-specific object built before a second provider has pushed back
on its shape. The attribution win is real and is deferred, not denied. The feature line
in the ledger that was folded into this decision already acknowledged this tension; this
ADR is where it resolves.

**A fourth option — no write path in M2 at all, defer the credential decision further.**
Rejected because the deferral has already cost a milestone. M2's defining property is "the
first write"; deferring the credential past M2 means deferring M2, which means the loop
and the store ship without ever being exercised against the thing they exist to feed, and
the feedback that would correct the credential model never arrives.

## Standing invariant added by this ADR

Added to `CLAUDE.md` §4 in the same commit:

| 10 | No secret (token, private key, signing key) is committed to the repository or written to any file inside the working tree. Secrets are supplied by env var or by a path outside the working tree. | ADR-0013 | `pending (LEDGER)` — the first M2 slice that reads a token ships `scripts/*-verify.mjs` coverage and promotes the row |

The row ships as `pending (LEDGER)` so it is not a silent aspiration: a ledger line
carries the debt of writing the check. The row goes in now because M2's first commit
introduces secret-reading code and the invariant needs to exist before that commit, not
after. The slice that first reads a token is where the row is promoted from pending to
`verified_by:`, because that is where there is something concrete to check.
