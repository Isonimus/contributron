---
id: '0004'
title: "No workflow DSL until three workflows exist"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0004 — No workflow DSL until three workflows exist

## Context

The product's intended differentiator is a declarative rule language: the operator writes
`ON`, `IF`, `WATCH`, `FOR EACH`, `LET`, `DO`, `ASK`, `REQUIRE`, `NOTIFY`, and a visual
editor projects the same syntax tree. The original design specified nine primitives, a
typed intermediate representation, an interpreter, a visual editor and a simulator that
replays historical events against a draft.

That is a compiler and an IDE. Either is a project in its own right.

The deeper problem is ordering. The semantics of the proposed constructs are not yet known
even informally. `WHILE` was offered in the first sketch without an answer to whether it
evaluates once per event, evaluates continuously, subscribes to future events, or what it
does when the merge request closes mid-execution. Those are not syntax questions, they are
questions about an execution model that does not exist. A language designed before its
first program encodes its author's guess about which abstractions will matter.

The global quality bar states the rule directly: reuse is reached for on the third instance
of the same logic, not the first, and a shared abstraction built before that is speculative
generality.

## Decision

No DSL, no parser, no intermediate representation and no visual editor until three
workflows exist in the system as ordinary Rust.

The three workflows are written plainly, with whatever duplication that produces. The
duplication is the evidence: it shows which constructs are genuinely shared and what their
semantics turned out to be in practice. The language is extracted from that, and the
extraction is itself a decision recorded at the time.

The engine built before the language is still a real engine. Persistent workflow instances,
recorded state transitions, event subscription and the policy check are all required by
hardcoded workflows and are all reusable by a language later. What is deferred is the
surface syntax and its compiler, not the execution model.

## Consequences

The product's headline differentiator does not exist until the fifth milestone. An operator
cannot author a rule without writing Rust until then, which means the MVP is useful to its
author before it is useful to anyone else. This is accepted: a language shipped on guessed
semantics would have to be broken later, and a breaking change to a language users have
written programs in is far more expensive than a late first release.

Three hardcoded workflows will contain duplication that would be flagged as a defect under
the normal reading of the quality bar. It is deliberate here and is recorded as such, so
that nobody removes it prematurely and destroys the evidence it exists to provide.

The risk is that three workflows chosen by one author generalize poorly, producing a
language fitted to a narrow sample. The mitigation is that the three are chosen to differ
deliberately: a read-only observation, a deterministic mutation, and an agent-driven change
requiring approval.
