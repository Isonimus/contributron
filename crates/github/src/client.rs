//! A `ProviderClient` that talks to GitHub's REST issue-comment endpoints (slice 0014).
//!
//! The reactor crate holds the pure transition logic and depends on a
//! [`ProviderClient`][contributron_react::ProviderClient] trait; this module is where that
//! trait meets `octocrab`. The adapter boundary (invariant 1) keeps GitHub vocabulary from
//! leaking: the only types crossing out of this crate are
//! [`contributron_react::ExistingComment`] and [`contributron_react::ProviderError`].
//!
//! Pull-request comments ride the issues endpoints on GitHub: a pull request is an issue
//! with extra fields, and `GET /repos/{o}/{r}/issues/{n}/comments` returns the general
//! conversation comments that `POST` on the same path appends to. Review-thread comments
//! are a different endpoint and not what this rule posts.

use contributron_domain::pull_request::{AuthorLogin, Label};
use contributron_poll::PullRequestLocator;
use contributron_react::{ClientFuture, ExistingComment, ProviderClient, ProviderError};

/// GitHub's maximum page size on the issues and labels endpoints.
///
/// A hundred is the per-request page size; the reader walks pages via
/// [`octocrab::Octocrab::all_pages`] until the Link header has no `next`, so the total
/// read is unbounded by this constant. Picking the largest page keeps the walk to one
/// round trip on the common case where the PR has fewer than a hundred items.
const COMMENT_PAGE_SIZE: u8 = 100;

/// A credentialed GitHub client this crate hands the reactor.
///
/// Holds an `octocrab::Octocrab` built with a personal token. The capture client in
/// [`crate::capture`] is deliberately a different constructor: it serves the read path and
/// must stay anonymous-friendly for public repositories, and merging the two would push the
/// read path into a credentialed shape it does not need (slice 0014 design).
#[derive(Debug, Clone)]
pub struct GitHubClient {
    inner: octocrab::Octocrab,
}

/// Why building a [`GitHubClient`] failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientBuildError {
    /// `octocrab` could not assemble the HTTP client, usually because the token could not be
    /// turned into a valid `Authorization` header value.
    #[error("building the GitHub client: {0}")]
    Build(#[from] octocrab::Error),
}

impl GitHubClient {
    /// Builds a client that carries `token` on every request.
    ///
    /// # Errors
    ///
    /// Returns [`ClientBuildError`] if `octocrab` cannot assemble the HTTP client.
    pub fn new(token: String) -> Result<Self, ClientBuildError> {
        let inner = octocrab::Octocrab::builder()
            .personal_token(token)
            .build()?;
        Ok(Self { inner })
    }
}

impl ProviderClient for GitHubClient {
    fn list_comments<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<ExistingComment>> {
        Box::pin(async move {
            let page = self
                .inner
                .issues(&locator.owner, &locator.repo)
                .list_comments(locator.number)
                .per_page(COMMENT_PAGE_SIZE)
                .send()
                .await
                .map_err(|e| ProviderError::Client(format!("listing comments: {e}")))?;
            let all = self
                .inner
                .all_pages(page)
                .await
                .map_err(|e| ProviderError::Client(format!("listing comments: {e}")))?;
            Ok(all
                .into_iter()
                .map(|c| ExistingComment {
                    body: c.body.unwrap_or_default(),
                })
                .collect())
        })
    }

    fn post_comment<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        body: &'a str,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async move {
            self.inner
                .issues(&locator.owner, &locator.repo)
                .create_comment(locator.number, body)
                .await
                .map_err(|e| ProviderError::Client(format!("posting a comment: {e}")))?;
            Ok(())
        })
    }

    /// Slice 0015's invariant-4 live read. Hits `GET /repos/{o}/{r}/pulls/{n}` and reads
    /// the REST `mergeable` boolean together with the `mergeable_state` enum.
    ///
    /// The scope of this rule is "conflicts cleared", not "the PR is ready to merge": the
    /// non-dirty settled states — `clean` / `unstable` / `has_hooks` / `blocked` /
    /// `behind` — all have the merge computed and free of conflicts, so they return
    /// `Ok(false)` (no conflict). `dirty` is the only state that still has a conflict
    /// and returns `Ok(true)`. The transient non-answers — `mergeable == None` (null
    /// while the server computes), `mergeable_state == Unknown` or `Draft` — do NOT
    /// silently decide:
    /// they are routed to [`ProviderError::Client`] so the reactor's park-and-retry path
    /// drives another attempt once the provider settles (finding 2 of the 2026-11-25
    /// review).
    fn live_merge_has_conflict<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, bool> {
        Box::pin(async move {
            use octocrab::models::pulls::MergeableState;
            let pr = self
                .inner
                .pulls(&locator.owner, &locator.repo)
                .get(locator.number)
                .await
                .map_err(|e| ProviderError::Client(format!("fetching pull request: {e}")))?;
            match (pr.mergeable, &pr.mergeable_state) {
                (_, Some(MergeableState::Dirty)) => Ok(true),
                (
                    _,
                    Some(
                        MergeableState::Clean
                        | MergeableState::Unstable
                        | MergeableState::HasHooks
                        | MergeableState::Blocked
                        | MergeableState::Behind,
                    ),
                ) => Ok(false),
                // Unknown / Draft / unmapped variants, or mergeable:null: the provider has
                // not settled on an answer. Treating that as "no conflict" would risk a
                // post on a still-dirty PR; treating it as "conflict present" would block
                // the rule forever. Err signals "retry".
                _ => Err(ProviderError::Client(
                    "GitHub's mergeability is still computing".to_owned(),
                )),
            }
        })
    }

    /// Slice 0016: list the labels currently attached to a pull request. A pull request is
    /// an issue on the labels endpoint; `list_labels_for_issue` is the correct call.
    ///
    /// An unparseable label name (empty, padded) is skipped rather than failing the whole
    /// list: a third-party label defined with a shape this code does not accept is not a
    /// reason to refuse the rule's read path. A warning would belong here in a later
    /// slice; for now the skip is silent because `needs-rebase` is the only label this rule
    /// cares about, and that one is constructed from a known-good literal.
    fn list_labels<'a>(&'a self, locator: &'a PullRequestLocator) -> ClientFuture<'a, Vec<Label>> {
        Box::pin(async move {
            let page = self
                .inner
                .issues(&locator.owner, &locator.repo)
                .list_labels_for_issue(locator.number)
                .per_page(COMMENT_PAGE_SIZE)
                .send()
                .await
                .map_err(|e| ProviderError::Client(format!("listing labels: {e}")))?;
            let all = self
                .inner
                .all_pages(page)
                .await
                .map_err(|e| ProviderError::Client(format!("listing labels: {e}")))?;
            Ok(all
                .into_iter()
                .filter_map(|l| Label::new(l.name).ok())
                .collect())
        })
    }

    /// Slice 0016: apply one label to a pull request. The endpoint is additive; applying a
    /// label that is already present is a no-op on the server, but the caller is still
    /// expected to short-circuit on `AlreadySettled` so the audit record is distinct.
    fn add_label<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async move {
            self.inner
                .issues(&locator.owner, &locator.repo)
                .add_labels(locator.number, &[label.as_str().to_owned()])
                .await
                .map_err(|e| ProviderError::Client(format!("adding a label: {e}")))?;
            Ok(())
        })
    }

    /// Slice 0016: remove one label from a pull request. Removing an absent label returns
    /// 404 from the GitHub endpoint; the caller is expected to check with `list_labels`
    /// first so this method is only called when the label is present.
    fn remove_label<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        label: &'a Label,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async move {
            self.inner
                .issues(&locator.owner, &locator.repo)
                .remove_label(locator.number, label.as_str())
                .await
                .map_err(|e| ProviderError::Client(format!("removing a label: {e}")))?;
            Ok(())
        })
    }

    /// Slice 0017: list the reviewers currently requested on a pull request. The GitHub
    /// endpoint returns users and teams; this rule only consults the user axis, so team
    /// review-requests are dropped at the boundary. A reviewer login that cannot be
    /// validated as an [`AuthorLogin`] is surfaced as a [`ProviderError::Client`] rather
    /// than silently dropped: finding 3 of the 2026-11-25 review (different site, same
    /// rule).
    ///
    /// The GitHub endpoint `GET /pulls/{n}/requested_reviewers` is not paginated: it
    /// returns `{users, teams}` as a single object and the Link header carries no
    /// `next`. The ledger item that treated this as a parallel to the issues-endpoint
    /// pagination was derived from the shape of the sibling methods, not from this
    /// endpoint's behaviour; it is closed by evidence rather than by a walk that would
    /// not do anything (slice 0021).
    fn list_requested_reviewers<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
    ) -> ClientFuture<'a, Vec<AuthorLogin>> {
        Box::pin(async move {
            let page = self
                .inner
                .pulls(&locator.owner, &locator.repo)
                .list_requested_reviewers(locator.number)
                .await
                .map_err(|e| ProviderError::Client(format!("listing requested reviewers: {e}")))?;
            let mut out: Vec<AuthorLogin> = Vec::with_capacity(page.users.len());
            for u in page.users {
                let login = AuthorLogin::new(u.login).map_err(|_| {
                    ProviderError::Client(
                        "GitHub returned a requested-reviewer login that did not pass \
                         AuthorLogin::new (empty or whitespace-only)"
                            .to_owned(),
                    )
                })?;
                out.push(login);
            }
            Ok(out)
        })
    }

    /// Slice 0017: request a review from one or more users on a pull request. Team review
    /// requests are not emitted by the review-requested rule; the GitHub call takes both
    /// axes and the empty team list carries the intent.
    fn request_reviewers<'a>(
        &'a self,
        locator: &'a PullRequestLocator,
        reviewers: &'a [AuthorLogin],
    ) -> ClientFuture<'a, ()> {
        Box::pin(async move {
            let users: Vec<String> = reviewers.iter().map(|r| r.as_str().to_owned()).collect();
            let teams: Vec<String> = Vec::new();
            self.inner
                .pulls(&locator.owner, &locator.repo)
                .request_reviews(locator.number, users, teams)
                .await
                .map_err(|e| ProviderError::Client(format!("requesting reviewers: {e}")))?;
            Ok(())
        })
    }
}
