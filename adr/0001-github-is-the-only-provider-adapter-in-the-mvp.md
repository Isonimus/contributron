---
id: '0001'
title: "GitHub is the only provider adapter in the MVP"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0001 — GitHub is the only provider adapter in the MVP

## Context

The product is intended to track both GitLab and GitHub projects, and the original design
proposed a `GitProvider` interface with adapters for each, written before either adapter
existed. The initial pain that motivated the product is on GitLab, at work.

Two facts pull the other way. This repository is hosted on GitHub, so a GitHub adapter can
be exercised against real pull requests and real review threads from the first week, with
no fixture invention and no second account to maintain. And an interface designed before
its first implementation is a guess: it encodes what the author imagined two providers have
in common, which is reliably not what they actually have in common.

## Decision

The MVP ships exactly one provider adapter, for GitHub, and no provider abstraction.

Provider-specific types are confined to the adapter module by convention and by module
boundary. The domain model never names a GitHub field: normalization happens at the edge,
and `object_attributes`, `pull_request_review` and their equivalents do not cross it. This
is the part of the original design that survives, because it is what makes a second adapter
possible later without a rewrite.

The abstraction is extracted when the GitLab adapter is written, and its shape is derived
from the two real implementations rather than from anticipation.

## Consequences

The GitLab adapter is deferred and carries a ledger line. The work it represents is not
reduced, only resequenced: writing the interface later costs roughly what writing it now
costs, and it is far more likely to be right.

This repository becomes its own primary test fixture. Every pull request opened during
development is usable input, which shortens the feedback loop and means the first real
integration test exercises a live provider rather than a recording.

The risk accepted is that a GitHub-shaped assumption leaks into the domain model and is
discovered only when GitLab arrives. The mitigation is the normalization boundary above,
which is checkable at review: a domain type that mentions a provider concept is a defect.

Should the work pain shift decisively to GitLab before the MVP completes, this decision is
superseded rather than amended, and the superseding record states what the GitHub-first
bet cost.
