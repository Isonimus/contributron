//! The three identities every recorded operation names (ADR-0005).
//!
//! A single-operator deployment has one user, one worker and usually no agent at all. The
//! model still carries all three, because retrofitting accountability onto a system that
//! recorded only "it happened" means re-deriving facts nobody kept.

use serde::{Deserialize, Serialize};

/// The human account that owns or approved an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserId(String);

/// The process that executed an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerId(String);

/// The AI agent that performed the work, where one did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentId(String);

/// An identity newtype constructor received an empty or whitespace-only value.
///
/// Carries the kind of identifier it was so that the diagnostic names *which* field
/// was blank. One shared error type keeps three parallel zero-variant structs from
/// accumulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{kind} must not be empty")]
pub struct EmptyId {
    /// A short name for the identifier kind (`"user id"`, `"worker id"`, `"agent id"`).
    pub kind: &'static str,
}

macro_rules! string_id {
    ($name:ident, $what:literal, $kind:literal) => {
        impl $name {
            #[doc = concat!("Wraps a ", $what, ".")]
            ///
            /// # Errors
            ///
            /// Returns [`EmptyId`] for an empty or whitespace-only value. The stored
            /// value is the trimmed input, so a padded identifier compares equal to its
            /// unpadded form and the provider never receives surrounding whitespace.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
                let value = value.into();
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    Err(EmptyId { kind: $kind })
                } else {
                    Ok(Self(trimmed.to_owned()))
                }
            }

            #[doc = concat!("The ", $what, " as a string.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(UserId, "user identifier", "user id");
string_id!(WorkerId, "worker identifier", "worker id");
string_id!(AgentId, "agent identifier", "agent id");

/// Whether an agent performed the work, stated rather than left to inference.
///
/// This is not an [`Option`] on purpose. An omitted field and an operation that genuinely
/// had no agent are different facts, and only one of them is a gap in the record. Serializing
/// a variant rather than skipping a key keeps that difference legible to a reader who was
/// not there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExecutingAgent {
    /// No agent was involved: the operation was performed by the system on its own behalf.
    NoAgent,
    /// The named agent performed the work.
    Agent {
        /// Which agent.
        id: AgentId,
    },
}

/// Who is accountable for one recorded operation: all three identities, always present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// The account that owns or approved the operation.
    pub user: UserId,
    /// The process that carried it out.
    pub worker: WorkerId,
    /// The agent that performed the work, or an explicit statement that there was none.
    pub agent: ExecutingAgent,
}

impl Attribution {
    /// An operation the local operator performed with no agent involved.
    #[must_use]
    pub fn local_operator(user: UserId, worker: WorkerId) -> Self {
        Self {
            user,
            worker,
            agent: ExecutingAgent::NoAgent,
        }
    }
}
