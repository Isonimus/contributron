# Security policy

## What counts

Contributron holds provider credentials, runs AI agents against repository content, and can
be authorized to change branches and merge code. Its threat surface is therefore not
incidental to the product: it is most of the product. A security issue here is anything that
lets an action happen which the operator did not authorize, or that widens what an agent can
reach.

Concretely, these are security issues:

- **An agent performing an action outside its granted capabilities.** The policy engine is
  the boundary, never the agent. If an agent can obtain an effect its workflow was not
  authorized for, that is a defect in the boundary.
- **Prompt injection reaching an effect.** Repository content, comments, issue bodies, review
  threads, branch names and commit messages are attacker-controlled. Text that persuades an
  agent to do something is expected; text that *succeeds* in causing an unauthorized action
  is a vulnerability in the engine that permitted it.
- **Credential exposure beyond its intended scope.** A provider token reaching an agent
  process, a worker, a log line, an error message, an artifact or a worktree it had no
  reason to be in.
- **Escaping execution isolation.** An agent task reading or writing outside its worktree,
  reaching the host filesystem, or affecting another task.
- **Bypassing an operator decision.** Merging, approving, replying or pushing without the
  approval the workflow required, or on provider state that was re-fetched and found stale.
- **A protected operation becoming reachable.** Agents must never approve their own changes,
  modify protected branches, alter repository permissions or disable an audit record.
- **An audit record that can be made to lie.** An operation whose recorded user, worker or
  agent identity is wrong, absent, or forgeable.

A rule that does not fire, an agent that proposes a bad fix, a poll that misses an event or a
report that is simply wrong is a bug, not a security issue. Open a normal issue for it.

## Supported versions

Nothing has been released yet. Until the first release, `main` is the only supported
revision.

| Version | Supported |
|---|---|
| `main` | Yes |
| Released versions | None yet |

## Reporting one

Report it privately, not in a public issue:

1. **Preferred:** GitHub's private vulnerability reporting, on this repository's **Security**
   tab, **Report a vulnerability**.
2. Otherwise, contact the maintainer, [@Isonimus](https://github.com/Isonimus).

Include the version or commit, the configuration involved (the workflow, its declared
capabilities, and the autonomy level it ran at), what the agent or engine was able to do, and
what should have stopped it. **Redact tokens and repository content** before sending logs.

If the report involves prompt injection, the payload itself is the useful part: include it
verbatim, and say which surface carried it (a review comment, an issue body, a file in the
diff) and what effect it achieved.

There is no bug bounty. Reports are credited if you would like.

## What this project assumes about agents

Stated here because a reader evaluating the security of the design should not have to infer
it from the code:

**An AI agent is never a security boundary.** It is an untrusted worker given a bounded task.
Every permission it has is enumerated before it starts, enforced outside it, and re-checked
against the provider immediately before any sensitive operation. An agent that asks for
something it was not granted receives a denial, and no amount of argument from the agent
changes that, because nothing in the enforcement path reads what the agent says.
