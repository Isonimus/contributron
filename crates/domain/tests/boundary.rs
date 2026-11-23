//! Scenario two of slice 0007: the domain crate cannot see the provider.
//!
//! Standing invariant 1 (CLAUDE.md) says no domain type names a provider concept. As a
//! convention that is a review question, and reviews miss things. This test makes it a
//! property of the resolved dependency graph: a type cannot be named through a dependency
//! that does not exist, so the invariant holds by construction and this test is what notices
//! if someone adds the dependency that would end that.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

const DOMAIN_CRATE: &str = "contributron-domain";

/// A dependency is provider-aware if its name says so. Deliberately a substring match: the
/// point is to fail on `contributron-github`, `octocrab`, `github-rs` and anything else
/// arriving later, rather than to maintain a list of known clients.
fn is_provider_aware(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("github") || name.contains("gitlab") || name.contains("octocrab")
}

fn workspace_metadata() -> Value {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--locked"])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("cargo metadata runs");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON")
}

/// Every package reachable from the domain crate, by any kind of dependency edge.
///
/// Development dependencies are included on purpose. A provider client reaching the domain
/// crate through a dev-dependency would let a test in this crate name a GitHub type, and the
/// invariant is about what this crate can express, not about what ships in a release build.
fn resolved_closure(metadata: &Value) -> BTreeSet<String> {
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("the resolve graph holds nodes");

    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut name_of: HashMap<&str, &str> = HashMap::new();
    for package in metadata["packages"].as_array().expect("packages is a list") {
        let id = package["id"].as_str().expect("a package has an id");
        let name = package["name"].as_str().expect("a package has a name");
        name_of.insert(id, name);
    }
    for node in nodes {
        let id = node["id"].as_str().expect("a node has an id");
        let deps = node["dependencies"]
            .as_array()
            .expect("a node lists its dependencies")
            .iter()
            .map(|dep| dep.as_str().expect("a dependency is an id"))
            .collect();
        edges.insert(id, deps);
    }

    let root = *name_of
        .iter()
        .find(|(_, name)| **name == DOMAIN_CRATE)
        .map(|(id, _)| id)
        .expect("the domain crate is in the workspace");

    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        for dependency in edges.get(id).into_iter().flatten() {
            let name = name_of.get(dependency).copied().unwrap_or(dependency);
            if seen.insert(name.to_owned()) {
                pending.push(dependency);
            }
        }
    }
    seen
}

#[test]
fn the_domain_crate_depends_on_nothing_provider_aware() {
    let metadata = workspace_metadata();
    let closure = resolved_closure(&metadata);

    assert!(
        !closure.is_empty(),
        "the closure is empty, so this test would pass for any graph and proves nothing"
    );

    let offenders: Vec<&String> = closure
        .iter()
        .filter(|name| is_provider_aware(name))
        .collect();
    assert!(
        offenders.is_empty(),
        "the domain crate can reach provider-aware crates {offenders:?}, which breaks the \
         normalization boundary ADR-0001 requires"
    );
}

#[test]
fn the_guard_itself_detects_a_provider_aware_name() {
    // Without this, a typo in `is_provider_aware` would make the test above pass for every
    // possible graph, which is the failure mode of a guard nobody tested.
    assert!(is_provider_aware("contributron-github"));
    assert!(is_provider_aware("octocrab"));
    assert!(!is_provider_aware("contributron-domain"));
    assert!(!is_provider_aware("serde"));
}
