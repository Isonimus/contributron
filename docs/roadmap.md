# Contributron: Roadmap

**Live doc.** It states what the plan *is* right now. It is updated in the same change that
alters the plan: completing a slice, accepting an ADR that re-sequences work, or cutting a
milestone.

## What this file is not

It is **not a tracker**. It carries no open defects, no deferrals, no follow-ups, no
checkboxes. Those are ledger lines in [`LEDGER.md`](../LEDGER.md), which is the only
hand-maintained tracker in this repo (stele:ADR-0001). Two trackers need manual sync, and
manual sync does not happen.

It is **not a decision record**. The reasoning behind every choice below lives in
[`adr/`](../adr/). Where this file and an ADR disagree, the ADR wins and this file is the
bug.

What it *is*: the single place a reader learns the shape of the work and the order it
happens in, so that a decision taken today is still legible in six months.

---

## The product, in one paragraph

Contributron is an event-driven automation layer for Git repositories. An operator declares
rules that observe repository activity (pull requests, review threads, pipelines, issues)
and dispatch work on event: a deterministic action, or an AI agent given a bounded task and
an explicit set of permitted capabilities. The operator keeps authority; the system reduces
the operator's workload without taking away their decisions. Agents are interchangeable
workers, never the product.

## The founding principles

These outlive any milestone below. A change that breaks one of them needs an ADR, not a
commit message.

1. **The engine is the security boundary, never the agent.** Repository content, comments
   and issues are untrusted input. An agent that asks for an unauthorized action receives a
   denial, not an opportunity to argue. No agent approves its own work, merges, touches a
   protected branch, alters permissions or disables an audit record.
2. **The provider is the final authority on provider state.** Re-fetch mergeability,
   approvals and branch protection immediately before any sensitive operation. An agent's
   claim that something is ready is evidence, not authorization.
3. **The operator remains in control.** A system that correctly recognises it cannot safely
   proceed is worth more than one that confidently does the wrong thing.
4. **Agents are interchangeable.** No domain type names a vendor. Claude Code is the first
   adapter, not the foundation.
5. **Every action is attributable.** Three distinct identities are recorded on every
   operation: the **user** who owns or approved it, the **worker** that ran it, the **agent**
   that executed it. "AI performed an action" is not an audit record.

---

## Locked decisions

Each becomes an ADR before the code it governs is written. Until an ADR exists, this row is
a statement of intent, not a decision.

| Area | Decision | Rationale in brief |
|---|---|---|
| Provider | GitHub first; one adapter only | This repo is its own test fixture from day one. GitLab is a later adapter. |
| Provider abstraction | Extracted on the **second** adapter, not the first | An interface fitted to one real implementation beats one fitted to two guesses. |
| Stack | Rust for the core | Long-running event daemon with concurrent polling and process supervision is squarely Rust's domain, and a stricter compiler is a better loop oracle. The compile-speed argument for TypeScript was probed and did not survive. |
| Event intake | Polling before webhooks | A workstation behind NAT cannot receive webhooks. Polling is also deterministic, so it is testable against recorded fixtures. |
| Rule authoring | No DSL until three workflows exist | A language designed before its first user is a guess. Extract it from three real workflows. |
| Deployment | Single operator | Multi-tenancy before a working automation is plumbing that protects nothing. |
| Domain model | Actor is first-class from commit one | Keeps principle 5 true, and makes the team model a later slice rather than a rewrite. |
| Doc toolchain | Node alongside Cargo, for stele only | Stele's linter and hooks are `.mjs`. A `package.json` exists for the document toolchain; it never carries application code. |
| Interface | CLI before any GUI | A rendered state view is assertable on stdout. A desktop shell is not. |

## Deliberately deferred

Each of these is a ledger line carrying the reason, not a gap. See [`LEDGER.md`](../LEDGER.md).

Workflow DSL and its parser, typed IR and interpreter. Visual workflow editor. Workflow
simulator. GitLab adapter. Webhook ingestion. Team and organization model, roles, approval
policies. PostgreSQL. Desktop shell. Distributed worker fleet and the shared agent pool.
Container-based execution isolation. Non-Git integrations (CI, Sentry, Grafana).

---

## Milestones

Each milestone is a set of slices. Each slice is written before its code, carries a
Definition of Done as Given/When/Then, and names its proof in `## Verification`
(stele:ADR-0011). A slice is the unit of review.

### M0: Foundation

The repo governs itself before it governs anything else.

- Stele kit installed, linter green, pre-commit hook live.
- The locked decisions above recorded as ADRs.
- Cargo workspace, `clippy` at deny level, `cargo test`, `cargo fmt`, all wired into CI.
- `package.json` carrying only the stele document checks, so rule R11 has a surface to
  check against.

**Done when:** a commit that violates a recorded convention is refused by the hook, and the
full check suite runs from one command.

### M1: Observation

Read-only. No mutation of any remote, by anything.

- GitHub adapter: authenticate, list open pull requests, read review threads and their
  resolution state.
- Normalize provider payloads into domain events. No GitHub field name reaches the domain.
- Durable event store with deduplication. A redelivered event changes nothing.
- Polling loop with reconciliation against provider state.
- CLI renders current state: tracked repositories, open PRs, unresolved threads.

**Done when:** against recorded API fixtures, normalization is exact, redelivery is a no-op,
and the rendered state matches the expected output. Fully offline and deterministic.

### M2: Deterministic reaction

Rules, without AI. The engine proves itself on actions whose correctness is checkable.

- Persistent workflow instances with recorded state transitions, surviving restart.
- Hardcoded rules in ordinary TypeScript. Still no DSL.
- Non-AI actions: notify the operator, label, comment.
- The policy engine, with capabilities declared per workflow and checked before dispatch.
- Audit record carrying all three identities.

**Done when:** a rule fires on a real event against this repo, the action is recorded with
full attribution, and a workflow instance survives a process kill mid-execution.

### M3: Agent execution

The architectural assumption under test: bounded, isolated, locally executed AI work.

- Agent task contract: description, repository context, allowed capabilities, limits.
- Claude Code CLI adapter returning a normalized result.
- Execution in an isolated Git worktree, one branch per task.
- Full task lifecycle, distinguishing a lost worker from a terminated process.
- Read-only investigation only. The agent reports; it changes nothing.

**Done when:** a review thread opens on a real pull request, an agent investigates it in an
isolated worktree, and the report reaches the operator with no mutation of the remote.

### M4: Supervised mutation

The first slice where an agent changes something, and the first where approval exists.

- Approval requests as first-class persisted entities, surviving operator absence.
- Agent-prepared commits on a task branch, never the operator's working branch.
- Diff and test results presented for operator decision.
- Replies to review threads after approval.
- Conditional merge, with provider state re-fetched at the moment of merge.

**Done when:** the original scenario works end to end: a reviewer opens a thread, an agent
proposes a fix, the operator approves, the fix is pushed, and the merge happens only once
every configured condition is re-verified against the provider.

### M5: Extraction

Only now does the language get designed, from three workflows that exist.

- Extract the DSL from the common shape of the hardcoded workflows.
- Typed intermediate representation; immutable workflow versions.
- Visual editor as a projection of the DSL, sharing one execution model.
- GitLab adapter, which is also the test of whether the provider interface was right.
- Team model, roles, approval policies.

**Done when:** the three M2-M4 workflows are expressed in the DSL and behave identically.

---

## The longer horizon

Recorded so the shape is not lost, not as commitment. The engine is not specific to Git: the
same trigger, condition, action and escalation primitives cover pipeline failures, container
health, exception trackers, alerting and scheduled maintenance. Git is the first integration
because it is the one with a concrete present pain. If the engine is built correctly, adding
a source of events is an adapter, not an architecture.

The team-wide agent pool, where developers opt their machines in as execution resources and
the scheduler matches tasks to capability, stays explicitly opt-in and never shares personal
credentials or unrestricted filesystem access.

## Known risks

| Risk | Assessment | Mitigation in the plan |
|---|---|---|
| Scope creep | High. The original brief is a multi-year platform. | Milestones are vertical slices, each independently useful. The DSL is deferred to M5. |
| DSL complexity | High. A language is a project. | Not designed until three real workflows exist to generalize from. |
| Prompt injection via repository content | Very high. Comments are attacker-controlled. | Principle 1: the policy engine enforces, the agent never does. |
| Agent execution reliability | High. Tasks run for minutes and machines die. | Persistent instances from M2, lifecycle states that distinguish disconnection from failure. |
| Credential blast radius | High. Local execution means local secrets. | Integration, worker and agent credentials kept distinct. Merge authority never reaches a worker. |
| Competition | Medium-high. The space moves fast. | Differentiation is cross-project orchestration, operator authority and agent independence, not review quality. |
