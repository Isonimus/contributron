# Contributron

Event-driven automation for Git repositories. An operator declares rules that observe
repository activity (pull requests, review threads, pipelines, issues) and dispatch work on
event: a deterministic action, or an AI agent given a bounded task and an explicit set of
permitted capabilities.

The operator keeps authority. The system reduces their workload without taking away their
decisions.

## Status

**Nothing is built yet.** This repository currently contains its own governance: the
decisions the work will obey, the sequenced plan, and the executable checks that enforce
both. Milestone M0.

That ordering is deliberate. The design this project starts from is a multi-year platform,
and the decisions that narrow it to something reachable are more valuable written down and
arguable than implicit in code nobody can question later.

| What | Where |
|---|---|
| Why each choice was made | [`adr/`](adr/), indexed in [`adr/INDEX.md`](adr/INDEX.md) |
| The sequenced plan, M0 through M5 | [`docs/roadmap.md`](docs/roadmap.md) |
| Open work, deferrals, known defects | [`LEDGER.md`](LEDGER.md) |
| The standard code is held to | [`docs/quality-bar.md`](docs/quality-bar.md) |
| Repo-specific conventions and invariants | [`CLAUDE.md`](CLAUDE.md) |

GitHub issues and milestones mirror the ledger for navigation. The ledger is authoritative:
where they disagree, the issue is wrong (ADR-0006).

## The shape of the plan

Five founding principles outlive any milestone. The two that constrain the most code:

**The engine is the security boundary, never the agent.** Repository content, comments and
issues are attacker-controlled input. An agent that asks for an unauthorized action
receives a denial, not an opportunity to argue. No agent approves its own work, merges,
touches a protected branch, alters permissions or disables an audit record.

**The provider is the final authority on provider state.** Mergeability, approvals and
branch protection are re-fetched immediately before any sensitive operation. An agent's
claim that something is ready is evidence, not authorization.

Milestones, in order: observation (read-only), deterministic reaction (rules without AI),
agent execution (read-only investigation in an isolated worktree), supervised mutation
(agent-authored changes behind operator approval), then extraction (the rule language,
designed from three workflows that exist rather than from anticipation).

## Decisions worth knowing before reading the code

- One provider adapter, GitHub, and no abstraction until the second one exists (ADR-0001).
- Rust for the core; Node is present only to run the document toolchain (ADR-0002).
- Polling before webhooks, because a workstation behind NAT cannot receive one and
  reconciliation is required regardless (ADR-0003).
- No rule language until three workflows exist as ordinary Rust (ADR-0004).
- Single operator, but the user/worker/agent distinction is first-class from the first
  commit, because that is the part that is expensive to retrofit (ADR-0005).

## This repository governs itself

Conventions here are executable or they are declared unenforceable out loud. A pre-commit
hook runs the document invariants against the commit rather than the working tree, so a fix
left unstaged cannot green a red commit.

```
npm run lint       # document invariants
npm run index      # regenerate adr/INDEX.md
npm run immutable  # immutable bodies have only gained lines
```

That machinery is [`@isonimus/stele`](https://github.com/Isonimus/stele), vendored rather
than depended on.

## License

MIT.
