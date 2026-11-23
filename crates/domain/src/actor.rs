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

macro_rules! string_id {
    ($name:ident, $what:literal) => {
        impl $name {
            #[doc = concat!("Wraps a ", $what, ".")]
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[doc = concat!("The ", $what, " as a string.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(UserId, "user identifier");
string_id!(WorkerId, "worker identifier");
string_id!(AgentId, "agent identifier");

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
