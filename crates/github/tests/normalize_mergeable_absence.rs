//! Slice 0022: the normalizer distinguishes the `mergeable` field being absent from the
//! field being present but null. Absent is a payload the adapter cannot read (loud); null
//! is a legitimate "provider still computing" state that normalizes to
//! `Mergeable::Unknown`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::pull_request::Mergeable;
use contributron_github::{NormalizeError, normalize_capture};

fn payload_with_mergeable(mergeable_fragment: &str) -> String {
    format!(
        r#"{{
  "data": {{
    "repository": {{
      "pullRequest": {{
        "number": 7,
        "title": "a pull request",
        "isDraft": false,
        "author": {{ "login": "alice" }},
        {mergeable_fragment}
        "reviewThreads": {{
          "pageInfo": {{ "hasNextPage": false }},
          "nodes": []
        }}
      }}
    }}
  }}
}}"#
    )
}

#[test]
fn an_absent_mergeable_key_errors_naming_the_field() {
    // No "mergeable" key at all. Future query edits that silently drop the field would
    // land here rather than at Unknown.
    let raw = payload_with_mergeable("");
    let error = normalize_capture(&raw).expect_err("absent mergeable must be loud");
    match error {
        NormalizeError::MissingField { field } => {
            assert_eq!(field, "data.repository.pullRequest.mergeable");
        }
        other => panic!("expected MissingField, got {other:?}"),
    }
}

#[test]
fn a_null_mergeable_value_normalizes_to_unknown() {
    // Legitimate "still computing" state. The slice-0015 rule depends on seeing it.
    let raw = payload_with_mergeable(r#""mergeable": null,"#);
    let observed = normalize_capture(&raw).expect("null is accepted");
    assert_eq!(observed.mergeable, Mergeable::Unknown);
}

#[test]
fn a_mergeable_concrete_value_round_trips() {
    for (fragment, expected) in [
        (r#""mergeable": "MERGEABLE","#, Mergeable::Mergeable),
        (r#""mergeable": "CONFLICTING","#, Mergeable::Conflicting),
        (r#""mergeable": "UNKNOWN","#, Mergeable::Unknown),
    ] {
        let raw = payload_with_mergeable(fragment);
        let observed = normalize_capture(&raw).expect("concrete value accepted");
        assert_eq!(observed.mergeable, expected, "fragment: {fragment}");
    }
}

#[test]
fn a_mergeable_variant_this_adapter_does_not_know_errors_during_deserialization() {
    // Catches the other half of the fail-loud contract: a new provider variant must not
    // silently collapse into Unknown. The outer deserialize_with carries unknown enum
    // values up as a Serde error, not as a null.
    let raw = payload_with_mergeable(r#""mergeable": "NEW_VARIANT","#);
    let error = normalize_capture(&raw).expect_err("unknown variant must fail");
    match error {
        NormalizeError::Unreadable(_) => {}
        other => panic!("expected Unreadable, got {other:?}"),
    }
}
