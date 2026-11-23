//! The GitHub adapter: the one place where GitHub's vocabulary is spoken.
//!
//! Everything leaving this crate is a `contributron_domain` type. A caller cannot obtain a
//! GitHub payload struct and a domain value from the same call and get them confused,
//! because normalization consumes the former and returns the latter.

pub mod capture;
pub mod normalize;
pub mod payload;

pub use capture::{CaptureError, PullRequestLocator, capture_pull_request};
pub use normalize::{NormalizeError, normalize_capture, normalize_pull_request};
