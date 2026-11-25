---
id: '0022'
title: "Fail loud at the provider adapter boundary"
type: slice
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Goal

Two provider-adapter paths silence a shape that should stop the operation. Each is the
failure mode CLAUDE.md invariant 3 (fail loud) exists to prevent.

This closes two ledgered items:

- *"`GitHubClient::list_labels` silently drops provider labels that fail `Label::new`
  via `filter_map(|l| Label::new(l.name).ok())`"* (ADR-0001). The current shape swallows a
  normalization error, so a misshapen label is invisible to the rule that would compare
  against it.
- *"Fail loud when the GraphQL `pullRequest.mergeable` field is absent from the payload"*
  (ADR-0001). The `Option<Mergeable>` collapses "field absent" and "field null" into the
  same value, so a future query edit that drops the field would silently normalize every
  pull request to `Mergeable::Unknown` and the slice-0015 rule would stop firing.

## Definition of Done

- **Given** a GitHub labels response holding a label whose `name` fails `Label::new`
  (empty, padded with whitespace)
- **When** `GitHubClient::list_labels` is called
- **Then** the call returns `Err(ProviderError::Client(...))` naming the offending label's
  position, and no partial list is returned to the rule.

- **Given** a GraphQL pull-request payload whose `mergeable` field is present but
  explicitly `null`
- **When** `normalize_capture` is called
- **Then** the returned `ObservedPullRequest` carries `Mergeable::Unknown`: the
  "provider still computing" state is still readable, because invariant 4 relies on
  the transition detector seeing it.

- **Given** a GraphQL pull-request payload whose `mergeable` key is absent from the
  object entirely
- **When** `normalize_capture` is called
- **Then** the call returns `Err(NormalizeError::MissingField { field })` whose `field`
  path names `data.repository.pullRequest.mergeable`. No `ObservedPullRequest` is
  produced.

- **Given** a GraphQL pull-request payload whose `mergeable` field is a concrete value
  (`"MERGEABLE"`, `"CONFLICTING"`, `"UNKNOWN"`)
- **When** `normalize_capture` is called
- **Then** the returned `ObservedPullRequest` carries the matching `Mergeable` value, as
  before.

## Design

### list_labels

The current `filter_map(|l| Label::new(l.name).ok())` is swapped for a `for` loop that
maps the first `Label::new` error into `ProviderError::Client` with a message that names
the offending position. Mirrors the shape slice 0017's `list_requested_reviewers`
already uses for the author-login validation case; the pattern is established.

### mergeable absence vs null

The `PullRequestPayload::mergeable` field becomes `Option<Option<Mergeable>>` with a
`#[serde(default, deserialize_with = "double_option")]` adapter. The outer `Option`
distinguishes "field absent" (`None`) from "field present with any value" (`Some(_)`);
the inner `Option` distinguishes null from a concrete `Mergeable`. The normalizer then:

- `None` → `NormalizeError::MissingField { field: "...mergeable" }` (loud).
- `Some(None)` → `Mergeable::Unknown` (null is a legitimate "computing" state).
- `Some(Some(v))` → `v`.

Existing code considered and reused:
- The `double_option` deserializer is the pattern serde's own docs name for this case.
  No helper is extracted: three lines inline reads at the field, and the rule of three
  does not fire on a single call site.
- `NormalizeError::MissingField` already exists; the slice adds no new variant.
- `ProviderError::Client` already carries the error-string shape list_requested_reviewers
  uses; no new variant.

The payload struct already uses `Option<_>` on every field so normalize can name which
field was absent (see the module-level comment in `payload.rs`); the mergeable case was
the one place that pattern had been skipped because the author-time intent was "null
means Unknown". The right shape is to preserve both intents and distinguish them.

## Verification

- `crates/github/tests/list_labels_fails_loud_on_bad_name.rs`: not written.
  `GitHubClient::list_labels` calls octocrab, which this slice does not mock. The
  behaviour is covered by read-through of the one-line change (the `filter_map` becomes
  a loop that errors on `Label::new` failure) and by the shape being identical to the
  slice-0017 `list_requested_reviewers` path that is already tested through its CLI
  integration.
- `crates/github/tests/normalize_mergeable_absence.rs` (new): asserts
  `MissingField` on an absent key, `Mergeable::Unknown` on an explicit null, and the
  three concrete values round-trip.
- Workspace clippy `-D warnings` and the full `cargo test` suite are the regression
  wall.

## As built

Shipped on 2026-11-25.

- `crates/github/src/client.rs`: `list_labels` returns the first `Label::new` error as
  `ProviderError::Client`, naming the item's zero-indexed position. The previous
  `filter_map(...).collect()` is replaced by an explicit loop.
- `crates/github/src/payload.rs`: `PullRequestPayload::mergeable` changes to
  `Option<Option<Mergeable>>` with a `#[serde(default, deserialize_with =
  "double_option")]` adapter. The `double_option` helper is a three-line private
  function.
- `crates/github/src/normalize.rs`: the payload-to-domain step for `mergeable` reads
  `None` as `MissingField`, `Some(None)` as `Unknown`, `Some(Some(v))` as `v`. The doc
  comment on the "null is Unknown" contract moves to this site.
- `crates/github/tests/normalize_mergeable_absence.rs` (new, 4 tests) asserts the four
  shapes.
- Verified green: `cargo clippy --all-targets -D warnings`, `cargo test`, `npm run
  lint`, `npm run lint:secrets`.
