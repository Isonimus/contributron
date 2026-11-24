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

use contributron_poll::PullRequestLocator;
use contributron_react::{ClientFuture, ExistingComment, ProviderClient, ProviderError};

/// How many existing comments the marker scan reads.
///
/// A hundred is GitHub's maximum page size. For the first slice that reads existing
/// comments, a single page matches the capture crate's single-page pull of review threads
/// (the same ledger item covers both deferrals). A pull request with more than a hundred
/// general comments would let a duplicate slip past the scan; the ledger carries the
/// pagination item that closes this gap.
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
        let inner = octocrab::Octocrab::builder().personal_token(token).build()?;
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
            Ok(page
                .items
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
}
