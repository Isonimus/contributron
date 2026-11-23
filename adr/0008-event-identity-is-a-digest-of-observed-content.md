---
id: '0008'
title: "Event identity is a digest of observed content"
type: architecture
status: accepted
date: 2026-11-23
supersedes: []
superseded_by: []
---

# ADR-0008: Event identity is a digest of observed content

## Context

Slice 0007 produces observations of pull requests from provider payloads. Nothing stores
them yet, and the durable event store with deduplication is a later slice. But the identity
of an event is a data format: once observations are persisted, every stored identity was
produced by whatever rule was in force when it was written, and changing the rule afterwards
means every stored row refers to an event nobody can recompute.

ADR-0003 chose polling before webhooks. Polling guarantees redelivery: the same pull request
in the same state is read again every interval, by design. Something must decide whether two
readings are the same fact, and that decision cannot be deferred to the store, because a
store can only deduplicate on an identity it is given.

Three candidate rules were available.

An **assigned identifier**, a counter or a UUID minted at observation. Cheap, and it makes
every poll a new event, which pushes the entire deduplication problem into the consumer and
guarantees that two workers reading the same repository disagree about how many events
occurred.

A **provider identifier**, GitHub's node id for the pull request. It is stable and it is not
an event identity: it does not change when the pull request does, so two different states of
one pull request would share an identity.

A **digest of the observed content**. Two processes that read the same state agree without
coordinating, and any change in what was observed is a different event.

The remaining question was what counts as observed content. An observation also records who
made it: the user, the worker and the agent that ADR-0005 requires. Including attribution in
the identity would mean that two workers polling the same repository produce two events for
one fact, which is the behaviour the assigned identifier was rejected for.

## Decision

An event's identity is the SHA-256 of a canonical encoding of the **observed content**, and
of nothing else. Attribution is excluded. Observing is not an event; what was observed is.

The canonical encoding is JSON over the observed value. JSON escapes its strings, so no field
value can impersonate a field boundary, which a concatenation of raw bytes would permit: a
title ending in a delimiter could otherwise forge the next field. Collections that the
provider does not promise an order for are sorted before encoding, so provider ordering
cannot change an identity.

Any further provider adapter must produce the same identity for the same observed state. An
identity that varies by adapter would make the GitLab adapter's observations undeduplicable
against GitHub's, which is a cost paid at the point the second adapter lands and the provider
interface is extracted.

## Consequences

Deduplication becomes possible without a store having been designed, and the later slice that
adds one inherits the property rather than retrofitting it.

The identity is defined by the encoding, so a change to the shape of an observed value changes
every identity derived from it. This is correct, because such a change means something
different was observed, and it is also a migration cost once anything is persisted. Nothing is
persisted today, and that is the cheapest moment this decision will ever be made at. It was
already exercised once: adding a variant for a deleted author changed the encoding, and the
expected digest in `crates/domain/tests/identity.rs` was recomputed independently with
`sha256sum`.

Deriving the identity can fail, in principle, if a value cannot be encoded. The function
returns a `Result` rather than collapsing that case, although it is unreachable for the types
involved, because an identity that silently fell back to something else would surface as a
duplicate action taken against a repository rather than as an error.

A content digest says nothing about *when* something was observed, and a consumer that needs
ordering cannot get it from the identity. That is accepted here: ordering is the event store's
problem and it has not been designed.
