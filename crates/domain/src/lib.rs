//! Provider-independent domain types.
//!
//! This crate holds the model the rest of the system is written against: normalized
//! events, the three-way actor identity every operation records, and workflow state. No
//! type here names a provider concept; GitHub's field names stop at the adapter boundary
//! (ADR-0001).
//!
//! # Status
//!
//! Intentionally empty. It exists so that the workspace, the lint configuration and CI
//! have something real to run against before the first slice is written, and so that
//! "the checks are wired" is a verified claim rather than an intention.
//!
//! A green CI badge on this commit therefore proves that the toolchain works, and nothing
//! about the correctness of a system that does not yet exist. The first slice replaces
//! this paragraph with types and the tests that hold them to their specification.
