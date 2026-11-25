//! Slice 0020: the three identity newtypes reject empty-after-trim input at the
//! constructor, and store the trimmed value. Mirrors the shape `AuthorLogin` and
//! `ThreadId` already enforce.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::actor::{AgentId, EmptyId, UserId, WorkerId};

#[test]
fn user_id_rejects_an_empty_string() {
    let err = UserId::new("").expect_err("empty should fail");
    assert_eq!(err.kind, "user id");
}

#[test]
fn worker_id_rejects_whitespace_only_input() {
    let err = WorkerId::new("   ").expect_err("whitespace should fail");
    assert_eq!(err.kind, "worker id");
}

#[test]
fn agent_id_rejects_a_tab_and_newline_only_input() {
    let err = AgentId::new("\t\n").expect_err("whitespace should fail");
    assert_eq!(err.kind, "agent id");
}

#[test]
fn a_padded_user_id_stores_the_trimmed_value() {
    let id = UserId::new("  alice  ").expect("padded is accepted");
    assert_eq!(id.as_str(), "alice");
}

#[test]
fn a_padded_worker_id_stores_the_trimmed_value() {
    let id = WorkerId::new(" cli\n").expect("padded is accepted");
    assert_eq!(id.as_str(), "cli");
}

#[test]
fn a_padded_agent_id_stores_the_trimmed_value() {
    let id = AgentId::new("\trule:x ").expect("padded is accepted");
    assert_eq!(id.as_str(), "rule:x");
}

#[test]
fn the_empty_id_error_names_which_identifier_was_blank() {
    // Three parallel constructors share one error type. The diagnostic distinguishes them
    // by `kind`, so a reader of the error can tell which field was empty.
    let errors: [EmptyId; 3] = [
        UserId::new("").unwrap_err(),
        WorkerId::new("").unwrap_err(),
        AgentId::new("").unwrap_err(),
    ];
    assert_eq!(errors[0].kind, "user id");
    assert_eq!(errors[1].kind, "worker id");
    assert_eq!(errors[2].kind, "agent id");
}
