---
id: '0005'
title: "Single operator deployment, but actor is first-class from the start"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0005 — Single operator deployment, but actor is first-class from the start

## Context

The product is intended for a development team, and the original design followed that to
its conclusion: organizations containing teams, four roles, permissions separated between
authoring a workflow and authorizing its execution, approval policies assignable to
individuals or groups, credentials partitioned between server, worker and agent, and
PostgreSQL to hold shared state across a deployed server.

All of that is necessary eventually and none of it is necessary to discover whether the
core idea works. Built first, it is multi-tenancy plumbing protecting a capability that
does not exist, and it must be maintained through every change to the capability it
anticipates.

The opposite mistake is equally real. Multi-tenancy retrofitted onto a model that assumed a
single user is one of the more expensive refactors in software, because the assumption does
not live in one place: it is distributed across every table, every query and every
permission check that was never written.

The distinction that makes the difference is narrower than the full team model. The
original design identified it precisely: three distinct identities on every operation, the
**user** who owns or approved it, the **worker** that ran it, and the **agent** that
executed it. That distinction costs nothing to honour on day one and is the part that is
expensive to add later.

## Decision

The deployment model is a single operator on one machine. No organizations, no teams, no
roles, no approval policies, no PostgreSQL, no deployed server.

The domain model and the audit record carry actor identity as a first-class field from the
first commit, in the three-way form above. Every recorded operation names all three, even
where two of them currently resolve to the same local operator.

Nothing in the persistence layer assumes a single user. Rows are scoped by owner even when
there is exactly one owner, so that introducing a second is a change of configuration
rather than a change of schema.

The team model is a later milestone and carries a ledger line.

## Consequences

Scoping columns exist with a single distinct value for some time, which looks redundant and
will attract suggestions to remove it. It is deliberate and recorded here so that the
suggestion can be answered rather than relitigated.

"AI performed an action" never appears in the audit record, which is the property the
original design was right to insist on, and it is available from the first milestone rather
than the fifth.

Authorization logic is deferred, not stubbed. There is no permission check that currently
returns true and is intended to be filled in later: the policy engine checks workflow
capabilities, which exist from the second milestone, and user roles simply do not
participate in it yet. A check that always passes is a check nobody will remember is
missing.
