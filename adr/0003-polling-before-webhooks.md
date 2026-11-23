---
id: '0003'
title: "Polling before webhooks"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0003 — Polling before webhooks

## Context

Webhooks are the natural way to learn that a repository changed, and the original design
assumed them. They have a precondition this product cannot meet at the start: the receiver
must be reachable from the provider. A daemon running on a developer workstation behind NAT
or a corporate firewall is not, and the alternatives all add infrastructure before anything
works. A hosted relay is a service to operate and secure. A self-hosted endpoint is a
deployment the single operator does not yet have.

Webhooks are also insufficient on their own. Deliveries are lost, duplicated and reordered,
so any correct system needs periodic reconciliation against provider state regardless.
Reconciliation is polling. The question is therefore not whether to poll but whether to
build webhooks first or second.

There is a third consideration specific to how this project is being built. Polling against
recorded fixtures is deterministic and runs offline, which makes it assertable in an
automated check. Webhook ingestion requires a live inbound delivery to exercise end to end,
which does not.

## Decision

Event intake is by polling, with reconciliation against provider state. Webhook ingestion
is deferred.

Polling is written as one of several possible event sources behind a single internal entry
point, so that adding webhooks later means adding a source rather than restructuring the
pipeline. That entry point is where deduplication lives, because deduplication is required
by polling on its own: consecutive polls observe the same unchanged objects.

Before any sensitive operation, provider state is re-fetched regardless of what the event
stream said. This is a standing invariant, not a property of the intake mechanism, and it
survives the arrival of webhooks unchanged.

## Consequences

Latency is bounded below by the poll interval rather than being near-immediate, and API
rate limits become a real constraint that conditional requests and backoff must respect.
For the automations in scope, where the human counterpart responds in minutes or hours, a
delay of tens of seconds is not a product defect.

Deduplication and reconciliation get built first, which is the correct order: they are the
parts that make webhook delivery safe when it arrives, and building them against a
deterministic source is easier than building them against a live one.

The first milestone becomes fully testable offline. That is the property this decision was
chosen for as much as any other.
