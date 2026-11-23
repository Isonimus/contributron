---
id: '0006'
title: "The roadmap is a live doc, not a second tracker"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0006 — The roadmap is a live doc, not a second tracker

## Context

The plan for this product is large, was arrived at through a long discussion, and would be
expensive to reconstruct from decisions alone. It needs to be written down somewhere a
reader can find the shape of the work and the order it happens in.

Stele's document taxonomy admits four kinds and states that there is no fifth.
`LEDGER.md` states the reason it is the only tracker: two files require manual sync, and
manual sync does not happen. A `ROADMAP.md` at the repository root with milestones and
their status would be exactly the second tracker that warning describes, and it would
diverge from the ledger within weeks.

A roadmap is also not obviously one of the four kinds. It is not immutable, because plans
change. It is not generated. It is not the ledger. The remaining kind is the live doc,
defined as describing how something behaves now and updated in the same change as the thing
it describes.

A roadmap fits that definition if it is read correctly. It does not describe the future; it
describes **the current state of the plan**, which is a present-tense fact that changes when
the plan changes.

GitHub milestones and issues raise the same question again, from outside the repository.

## Decision

The roadmap is `docs/roadmap.md`, a live doc, cited from `CLAUDE.md` so it is not orphaned,
and updated in the same change that alters the plan.

It carries the sequenced plan: founding principles, locked decisions, milestones, the longer
horizon and the risk assessment. It carries **no open items**: no defects, no deferrals, no
follow-ups, no checkboxes and no status column. There is therefore nothing in it that could
fall out of sync with the ledger, because the two files hold disjoint kinds of fact.

`LEDGER.md` remains the only tracker. GitHub issues and milestones are a projection of the
ledger for collaborators and for discussion, never a source of truth: an item is closed by
deleting its ledger line, and the issue follows.

## Consequences

A reader gains one place to learn the shape of the work without reading twelve decision
records in sequence.

The boundary is a convention, not a rule the linter can enforce, because no linter can
distinguish a milestone description from a tracked item. It is stated in the roadmap's own
opening section so that a reviewer can apply it, and a status column appearing in that file
is a defect to be removed rather than maintained.

GitHub issues can drift from the ledger, since nothing mechanically ties them together. The
direction of authority is fixed here so the drift has a correct resolution: where they
disagree, the ledger wins and the issue is wrong.
