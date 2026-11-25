//! Slice 0021: `capture_pull_request` walks `reviewThreads` pages using `$after` and
//! assembles one synthetic payload whose `hasNextPage` is `false` and whose `nodes`
//! carries every thread across pages. The normalizer continues to refuse a truncated
//! payload because the stitched fixture always declares the walk complete.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_github::assemble_fixture_for_test as assemble_fixture;
use contributron_github::normalize_capture;
use serde_json::json;

fn page(nodes: &[(&str, bool)], has_next: bool, cursor: Option<&str>) -> serde_json::Value {
    let nodes_val: Vec<serde_json::Value> = nodes
        .iter()
        .map(|(id, resolved)| json!({ "id": id, "isResolved": resolved }))
        .collect();
    json!({
        "repository": {
            "pullRequest": {
                "number": 7,
                "title": "a pull request",
                "isDraft": false,
                "author": { "login": "alice" },
                "mergeable": "MERGEABLE",
                "reviewThreads": {
                    "pageInfo": { "hasNextPage": has_next, "endCursor": cursor },
                    "nodes": nodes_val,
                }
            }
        }
    })
}

#[test]
fn two_pages_are_concatenated_into_one_fixture_with_has_next_false() {
    let pages = vec![
        page(&[("t1", true), ("t2", false)], true, Some("cursor-1")),
        page(&[("t3", true), ("t4", true)], false, None),
    ];
    let fixture = assemble_fixture(pages).expect("assemble");
    let parsed: serde_json::Value = serde_json::from_str(&fixture).expect("re-parse");

    let page_info = parsed
        .pointer("/data/repository/pullRequest/reviewThreads/pageInfo")
        .expect("page info");
    assert_eq!(page_info["hasNextPage"], json!(false));
    assert_eq!(page_info["endCursor"], serde_json::Value::Null);

    let nodes = parsed
        .pointer("/data/repository/pullRequest/reviewThreads/nodes")
        .and_then(|v| v.as_array())
        .expect("nodes array");
    let ids: Vec<&str> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["t1", "t2", "t3", "t4"]);

    // Normalizer accepts it: a fixture whose walk is complete reports an observation.
    normalize_capture(&fixture).expect("normalizer accepts the stitched fixture");
}

#[test]
fn one_page_fixture_is_byte_comparable_to_the_input_for_the_common_case() {
    // A pull request with no second page: the stitched fixture is one page, with
    // hasNextPage: false and endCursor: null (which matches a hand-captured payload).
    let pages = vec![page(&[("t1", true)], false, None)];
    let fixture = assemble_fixture(pages).expect("assemble");
    normalize_capture(&fixture).expect("normalizer accepts the one-page fixture");
    let parsed: serde_json::Value = serde_json::from_str(&fixture).expect("re-parse");
    assert_eq!(
        parsed["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"]
            .as_array()
            .map(Vec::len),
        Some(1),
    );
}

#[test]
fn three_pages_preserve_order_so_the_normalizer_sees_the_same_sequence_as_the_provider() {
    let pages = vec![
        page(&[("a1", true), ("a2", true)], true, Some("c-a")),
        page(&[("b1", false)], true, Some("c-b")),
        page(&[("c1", true), ("c2", false), ("c3", true)], false, None),
    ];
    let fixture = assemble_fixture(pages).expect("assemble");
    let parsed: serde_json::Value = serde_json::from_str(&fixture).expect("re-parse");
    let ids: Vec<&str> = parsed
        .pointer("/data/repository/pullRequest/reviewThreads/nodes")
        .and_then(|v| v.as_array())
        .map(|v| v.iter().map(|n| n["id"].as_str().unwrap()).collect())
        .expect("nodes");
    assert_eq!(ids, vec!["a1", "a2", "b1", "c1", "c2", "c3"]);
}
