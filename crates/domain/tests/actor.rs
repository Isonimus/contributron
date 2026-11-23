//! Scenario six of slice 0007: the audit record names all three identities, and an absent
//! agent is stated rather than omitted (ADR-0005, standing invariant 6).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use serde_json::Value;

fn record(agent: ExecutingAgent) -> Value {
    let attribution = Attribution {
        user: UserId::new("local-operator"),
        worker: WorkerId::new("cli"),
        agent,
    };
    serde_json::to_value(&attribution).expect("an attribution serializes")
}

#[test]
fn an_operation_with_no_agent_says_so_rather_than_omitting_the_field() {
    let value = record(ExecutingAgent::NoAgent);
    let object = value.as_object().expect("an attribution is an object");

    // The distinction this asserts: a reader of the record can tell "no agent was involved"
    // from "nobody recorded whether an agent was involved". With an omitted key those two
    // facts are the same bytes, and only one of them is a gap in the record.
    assert!(object.contains_key("agent"), "the agent field is present");
    assert_eq!(value["agent"]["kind"], "no-agent");

    assert_eq!(value["user"], "local-operator");
    assert_eq!(value["worker"], "cli");
    assert_eq!(
        object.len(),
        3,
        "exactly the three identities, no fourth field"
    );
}

#[test]
fn an_operation_an_agent_performed_names_that_agent() {
    let value = record(ExecutingAgent::Agent {
        id: AgentId::new("reviewer-1"),
    });

    assert_eq!(value["agent"]["kind"], "agent");
    assert_eq!(value["agent"]["id"], "reviewer-1");
    assert_eq!(value["user"], "local-operator");
    assert_eq!(value["worker"], "cli");
}

#[test]
fn the_two_records_are_distinguishable() {
    // Without this, both tests above would still pass if every agent serialized identically.
    assert_ne!(
        record(ExecutingAgent::NoAgent),
        record(ExecutingAgent::Agent {
            id: AgentId::new("reviewer-1")
        })
    );
}
