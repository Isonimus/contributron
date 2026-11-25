---
id: '0020'
title: "Boundary validation for identity newtypes"
type: slice
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Goal

Tighten the domain's identity newtypes so a blank or whitespace-only string is rejected at
the constructor, the way `AuthorLogin`, `ThreadId` and `Label` already are. Today
`UserId::new`, `WorkerId::new` and `AgentId::new` accept the empty string and
`AuthorLogin::new` accepts a padded login and stores it untrimmed, so a persisted row can
carry `owner_user=''`, `worker=''` or `agent_id=''`, and a reviewer list can be sent to
the provider as `" alice"`. Invariant 6 says every operation names all three identities;
an empty or padded name is a boundary-validation gap, not a corruption bug.

This closes two ledgered `[bug]` items:

- *"`UserId::new`, `WorkerId::new` and `AgentId::new` accept the empty string. The three
  newtypes are declared by the `string_id!` macro in `crates/domain/src/actor.rs`, which
  does no validation."*
- *"`AuthorLogin::new` rejects whitespace-only input but stores the untrimmed value, so
  `' alice'` passes validation and is sent to the provider with its leading space."*

The CLI-side trim workaround in `resolve_reviewers_with_env` becomes redundant for the
padding case but is retained for empty-entry filtering (a trailing comma on
`--reviewer=a,,b` still needs to be dropped at the CLI layer so an empty login never
reaches the newtype's error path at all).

## Definition of Done

- **Given** the string `""` or `"   "`
- **When** `UserId::new`, `WorkerId::new` or `AgentId::new` is called with it
- **Then** the call returns `Err(EmptyId)` carrying the newtype's kind; no partial or
  trimmed value is constructed.

- **Given** the string `" alice "`
- **When** `AuthorLogin::new` is called with it
- **Then** the call returns `Ok(AuthorLogin)` whose `as_str()` is `"alice"`. The trimmed
  value is what is stored, compared and sent to the provider.

- **Given** the string `""` or `"   "`
- **When** `AuthorLogin::new` is called with it
- **Then** the call returns `Err(EmptyAuthorLogin)`. This case was already handled; the
  regression test guards against a future refactor regressing it.

- **Given** a valid identifier (one or more non-whitespace characters, optionally with
  surrounding whitespace in the newtypes that trim)
- **When** any of the constructors above is called with it
- **Then** the call returns `Ok(_)` and `as_str()` returns the trimmed content, matching
  what the store writes and reads back.

## Design

The `string_id!` macro in `crates/domain/src/actor.rs` already centralises the three
identifier types. Follow the pattern established by `AuthorLogin` and `ThreadId`: the
constructor returns `Result<Self, EmptyId>`, trims the input, and rejects anything whose
trimmed form is empty. A single shared `EmptyId { kind: &'static str }` error keeps the
callsite diagnostic clean (`"user id must not be empty"` etc.) without introducing three
sibling zero-variant structs.

The blast radius is ~110 call sites (most in tests), so the mechanical fan-out dominates
the change. The CLI and store layers that already have fallible paths propagate with `?`;
tests opt into `clippy::unwrap_used` in their file allow block and use `.expect("test
literal")` the way other test modules do. The store's row-read path converts a row whose
identity column was somehow empty into a `StoreError::IdentityEmpty` so a corrupted row is
refused rather than panicking.

Existing code considered and reused:
- The `AuthorLogin::new` shape (trim, reject empty, store trimmed) is lifted unchanged.
- `Label::new` was already correct: it rejects empty and padded explicitly rather than
  trimming, because a GitHub label carrying intentional whitespace is a normalization
  failure.
- The CLI's `resolve_reviewers_with_env` filters empty strings before calling
  `AuthorLogin::new`, so a trailing-comma case does not surface through the newtype.

The one subtle choice: `UserId`/`WorkerId`/`AgentId` *trim* rather than requiring already
trimmed input. An operator is more likely to type `--worker " cli"` than to intentionally
embed whitespace in a worker identifier, so the forgiving shape matches the one
`AuthorLogin` already uses. `Label` keeps its strict shape because that constraint comes
from the provider, not from operator ergonomics.

## Verification

- `crates/domain/tests/actor_validation.rs` (new): asserts empty, whitespace-only, and
  padded input for each of the three identity newtypes.
- `crates/domain/tests/author_login_trims.rs` (new): asserts `" alice "` is accepted and
  stored as `"alice"`, and that a padded login compares equal to its unpadded form.
- `crates/store/tests/owner_scope.rs`, `crates/store/tests/reactions.rs`: existing
  coverage updated to construct identities fallibly; the owner-mismatch case continues to
  hold because the store compares trimmed strings.
- Workspace clippy with `-D warnings` and the full `cargo test` suite are the regression
  wall.

## As built

Shipped on 2026-11-25.

- `crates/domain/src/actor.rs`: the `string_id!` macro now returns `Result<Self,
  EmptyId>`. `EmptyId::kind` carries `"user id"`, `"worker id"` or `"agent id"`; one
  shared error type, three constructors, consistent diagnostic.
- `crates/domain/src/pull_request.rs`: `AuthorLogin::new` trims before storing; the
  stored value is the trimmed one. `EmptyAuthorLogin` is unchanged.
- `crates/store/src/lib.rs`: the row-read helpers return
  `StoreError::IdentityEmpty { column }` on an empty identity column rather than
  constructing a tombstone newtype. Three call sites touched; the on-disk shape is
  unchanged.
- `crates/cli/src/main.rs`: the CLI continues to filter empty reviewer entries (trailing
  comma case) before calling `AuthorLogin::new`; the trim workaround comment is updated
  to note that padding is now caught at the newtype.
- All ~110 call sites across production and tests updated: production propagates with
  `?` or `.map_err`, tests opt into `clippy::unwrap_used` and use `.expect("static test
  value")`.
- Regression tests: `crates/domain/tests/actor_validation.rs` covers the three new
  constructors. `crates/domain/tests/author_login_trims.rs` covers `AuthorLogin` trim.
- Verified green: `cargo clippy --all-targets -D warnings`, `cargo test`, `npm run lint`,
  `npm run lint:secrets`.
