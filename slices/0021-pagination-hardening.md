---
id: '0021'
title: "Paginate provider reads so rules never miss state past page one"
type: slice
status: accepted
date: 2026-11-25
supersedes: []
superseded_by: []
---

## Goal

Four provider read paths currently read the first page only. Each is the same bug class:
the rule compares the observed state against a slice of the ground truth and makes a
decision on incomplete evidence. Close the four together because they share one pattern
and the ledger has carried them as parallel entries since slice 0014.

This closes three ledgered `[feature]` items and one `[feature]` carrying the same shape
for the GraphQL path:

- *"Paginate PR issue comments in `GitHubClient::list_comments`"* (rule: `threads-cleared`).
- *"Paginate PR labels in `GitHubClient::list_labels`"* (rule: `needs-rebase-label`).
- *"Paginate PR requested reviewers in `GitHubClient::list_requested_reviewers`"* (rule:
  `request-reviewers`).
- *"Paginate `reviewThreads`"* (the capture that every rule reads from).

The three REST endpoints use octocrab's `all_pages` helper, which walks the Link header
until the stream ends. The GraphQL `reviewThreads` connection uses cursor pagination with
`$after` and `pageInfo.endCursor`; the normalizer's current refusal of a payload whose
`hasNextPage` is true stands, because the capture now accumulates nodes into one payload
whose final `hasNextPage` is false. A pull request that genuinely has more threads than
one fetch returns is now observable rather than invisible.

## Definition of Done

- **Given** a pull request with 150 issue comments
- **When** `GitHubClient::list_comments` is called
- **Then** the returned vector holds 150 `ExistingComment` values; the marker scan sees a
  comment on page two the same as one on page one.

- **Given** a pull request with 120 labels
- **When** `GitHubClient::list_labels` is called
- **Then** the returned vector holds every label whose name passes `Label::new`; the
  slice-0016 membership check sees `needs-rebase` even when it sits past page one.

- **Given** a pull request whose requested-reviewer user list has 35 entries
- **When** `GitHubClient::list_requested_reviewers` is called
- **Then** every user login is returned. The GitHub endpoint is not itself paginated
  (`/pulls/{n}/requested_reviewers` returns one `{users, teams}` object), so the slice
  documents that no walk is needed here rather than adding code the endpoint would not
  use. The ledger entry closes by that evidence.

- **Given** a pull request with 250 review threads
- **When** `capture_pull_request` captures it
- **Then** the resulting fixture carries all 250 nodes under
  `reviewThreads.nodes` and its `pageInfo.hasNextPage` is `false`; the normalizer accepts
  the fixture and reports an `ObservedPullRequest` whose thread count matches the live
  state.

- **Given** a pull request with between one and `per_page` state (comments, labels,
  threads, reviewers)
- **When** any of the four reads above is called
- **Then** no second request is made (the Link header has no `next`; the GraphQL
  `hasNextPage` is `false` on the first page). Pagination costs nothing on the common
  case.

## Design

The three REST endpoints use `octocrab::Octocrab::all_pages(page)`, which consumes the
first `Page<T>` plus the client reference and walks the `next` link until exhausted. The
helper is straightforward and the one the capture crate already depends on transitively,
so the dependency surface is unchanged.

`list_requested_reviewers` is a different shape: the GitHub endpoint returns
`{users: [], teams: []}` as a single object rather than a `Page<T>`. GitHub's own
documentation does not list this endpoint under the paginated ones. Rather than invent
pagination for an endpoint that does not expose it, the slice documents the finding (the
ledger item's concern was derived from the parallel REST readers; the specific endpoint
does not share their behaviour) and closes the item.

For GraphQL `reviewThreads`, the query gains `$after: String` and selects
`pageInfo.endCursor` alongside `hasNextPage`. The capture loops: fetch, accumulate nodes,
check `hasNextPage`, pass `endCursor` as `after` on the next call. Once the final page is
in hand, the capture assembles one synthetic payload with
`pageInfo.hasNextPage: false`, `pageInfo.endCursor: null`, and all nodes concatenated.
That shape matches what the normalizer already accepts, so the normalizer code stays
untouched and the fixture remains byte-comparable with a single-page hand capture.

Existing code considered and reused:
- `octocrab::all_pages` already walks the Link header; adopting it means the implementation
  is one line of glue, not a new helper.
- The normalizer's `hasNextPage` refusal stays: an older build that never saw a
  multi-page fixture would still not have the code to walk it, and the capture is the one
  place that now guarantees `hasNextPage == false` by construction.
- The capture's fixture shape and the `data` wrapper are unchanged, so recorded fixtures
  made before this slice still deserialize.

The one subtle choice: the capture no longer respects `THREAD_PAGE_SIZE` as a cap. It
remains as the per-request page size (still 100), but the total is unbounded. A pull
request with pathological thread counts could inflate memory, but the realistic bound is
GitHub's own per-PR practical limit, which is below anything the store or the normalizer
cannot hold.

## Verification

- `crates/github/tests/capture_paginates_review_threads.rs` (new): the pagination loop's
  assembly step is extracted as `assemble_fixture`, and the test drives it with
  two-page, three-page and one-page fixtures, asserting node order is preserved,
  `hasNextPage` is always `false` in the stitched output, and the normalizer accepts the
  result.
- REST pagination (`list_comments`, `list_labels`): no new test. The change delegates to
  `octocrab::Octocrab::all_pages`, which is tested upstream; adding a wiremock dependency
  for a one-line delegation is scope creep this slice declines. The ledger items close
  on evidence: the call now walks the Link header, which is what pagination means at the
  REST layer.
- `list_requested_reviewers`: no new test. GitHub's endpoint is not paginated; the
  documentation block cites the evidence and the ledger item closes on that evidence.
- Workspace `cargo clippy -D warnings` and the full `cargo test` suite are the
  regression wall.

## As built

Shipped on 2026-11-25.

- `crates/github/src/client.rs`: `list_comments` and `list_labels` call
  `self.inner.all_pages(page)` after the first `send()`, walking the Link header until
  exhausted. The `COMMENT_PAGE_SIZE` doc updates to note it is only the per-request
  size, not a cap on the total read. `list_requested_reviewers` keeps its one-call
  shape; the doc records that the endpoint is not paginated.
- `crates/github/src/capture.rs`: `capture_pull_request` delegates to a
  `capture_pull_request_with` helper that loops: fetch a page with `$after`, check
  `hasNextPage`, advance the cursor until done. The accumulated pages are passed to a
  pure `assemble_fixture` helper that stitches one payload whose `hasNextPage` is
  `false` and whose nodes are concatenated in walk order. A `hasNextPage: true` with
  `endCursor: null` is caught and surfaced as `NormalizeError::Truncated`.
  The `QUERY` now takes `$after: String` and selects `pageInfo.endCursor`.
- `crates/github/src/lib.rs`: `assemble_fixture_for_test` is exposed under `#[doc(hidden)]`
  so the regression test can drive the pure assembler without a live GraphQL endpoint.
- `crates/github/tests/capture_paginates_review_threads.rs` (new, 3 tests): asserts the
  concatenated order, the one-page byte-shape and the three-page walk-order.
- Verified green: `cargo clippy --all-targets -D warnings`, `cargo test`, `npm run
  lint`, `npm run lint:secrets`.
- Deviation: no HTTP-mocked tests for the REST pagination paths. The ledger's concern
  (first-page-only reads) is closed by swapping `page.items` for `all_pages(page)`,
  which octocrab's tests cover; the slice's audit trail here is the source change itself
  plus the per-slice doc. If a future failure pattern argues for an integration test,
  `wiremock` is the shape it will take.
