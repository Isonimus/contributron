//! The domain model: what Contributron observes, and who is accountable for it.
//!
//! Nothing here names a provider concept. There is no `GitHub` in any type, field or error
//! in this crate, and no dependency through which one could arrive (standing invariant 1,
//! ADR-0001). A provider payload becomes a value in this crate only by passing through an
//! adapter's normalization, and the adapter depends on this crate rather than the reverse.

pub mod actor;
pub mod event;
pub mod pull_request;
