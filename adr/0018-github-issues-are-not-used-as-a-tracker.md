---
id: '0018'
title: "GitHub issues are not used as a tracker"
type: architecture
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Context

At project start on 2026-11-23, thirteen GitHub issues were opened against this repository
to mirror the initial deferrals from `LEDGER.md` and the first slice. One more (#14, the
`chrono` RUSTSEC-2026-0543 advisory) was opened on 2026-11-24. The intent was a public,
provider-native view of what the project had decided to defer.

Two days later the two trackers have already desynchronised in both directions, which is
the failure ADR-0001 named at the time it designated `LEDGER.md` the single mutable file:

- Of the 14 ledger items open on 2026-11-25, **12 have no GitHub issue**: every pagination
  item (`list_comments`, `list_labels`, `list_requested_reviewers`, `reviewThreads`), the
  `UserId`/`WorkerId`/`AgentId` empty-string bug, the second-SIGINT and in-flight-fetch
  item, the `threads-cleared` marker-scan hardening, the reaction persistence feature, the
  anonymous capture client, the GraphQL `mergeable` fail-loud, the `list_labels`
  `filter_map` swallow, the SSH-signed agent commits feature, and the `scripts/build-index.mjs`
  backslash bug.
- Of the 14 open issues, **one (#14) has no ledger line**: the `chrono` advisory lives on
  GitHub and nowhere else.
- Three slices (0014 for threads-cleared, 0015–0017 for the next three deterministic
  rules) shipped between project start and this decision; none closed a GitHub issue,
  because none had one to close.

The drift is not a discipline problem to fix with a project invariant — it is the exact
structural failure ADR-0001 was written to prevent. A convention that reads *"close the
GitHub issue when the ledger line is deleted"* is a manual sync between two writers, and
the data above shows that the sync does not happen. Making it a review-only invariant
would entrench the drift as policy.

A machine-checked sync (a `scripts/lint-issues.mjs` wired into CI) is available in
principle and was considered. It is rejected for this milestone on three grounds:

1. **Cost to carry.** A lint rule that calls the GitHub API from CI couples the repository's
   green-build state to a network service, to a token's rate limit, and to GitHub's own
   availability. Every CI run becomes dependent on an external system to confirm an
   internal fact (what is in `LEDGER.md`).
2. **Value delivered.** The audience for a public issue tracker is a hypothetical external
   contributor. The project is a single-operator deployment (ADR-0005), there are no
   external contributors, and the ledger is already a committed, diffable file that any
   reader of the repository can open.
3. **Rule of three.** This is the first repository of mine in which the question has been
   asked. A general tracker-mirror tool is designed from three real users of it, not from
   zero (`CLAUDE.md` §3). The script can be written the day a second consumer appears.

## Decision

GitHub issues are not used as a tracker for this repository.

- `LEDGER.md` is the single writer for open work: deferrals, known defects, follow-ups,
  and audits. This is the existing invariant from ADR-0001, restated here only because the
  present ADR is its enforcement moment for the GitHub-issues question.
- The existing issues #2 through #13 are closed with a one-line comment citing this ADR.
  They are kept as historical records (not deleted) so the chain from the 2026-11-23 seed
  commit to this decision stays visible.
- Issue #14 (the `chrono` RUSTSEC advisory) is a real open item with no ledger line. It is
  **added to `LEDGER.md` as a `[bug]`** in the same change that closes this ADR's
  implementation, and the GitHub issue is then closed citing that ledger line.
- No new issue is opened against this repository by project process. An issue opened by an
  external party is triaged into the ledger (or rejected) the same day and then closed
  with a pointer; the ledger stays the source of truth.
- No lint rule is wired to verify this, by the reasoning in `## Context`. The invariant is
  carried by this ADR and by `CLAUDE.md`, not by CI.

## Consequences

- The public visibility that GitHub's issue tracker would have offered to external readers
  is given up for the duration of the single-operator milestones (M1 through at least M3).
  A reader who wants to know what is open opens `LEDGER.md`; this file is referenced from
  `README.md` and from `CLAUDE.md`, so the pointer is discoverable from the two entry
  points a repository reader lands on.
- One decision, one place. Closing a ledger line is complete when the line is deleted; no
  second surface needs updating, and no "did you remember to close the issue?" review
  question exists. The failure mode of a half-closed item (ledger line deleted, issue
  still open or vice versa) is removed by construction, which is the benefit ADR-0001's
  single-writer principle was invoked for.
- A future decision to reopen GitHub issues (for example, when the project transitions to
  a team model under ADR-0005's deferred team scope) supersedes this ADR with a new one
  that states *why the single-operator reasoning no longer applies*. A machine-checked
  mirror is designed at that point, with the then-current ledger as the sync source and a
  CI lint as the enforcement.
- No row is added to `CLAUDE.md` §4. The §4 table lists invariants that bind implementation
  work; "do not open a GitHub issue" is a process convention that binds the operator, and
  process conventions live in the governing ADR, not in the invariant table.
