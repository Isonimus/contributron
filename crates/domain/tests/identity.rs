//! Scenario four of slice 0007: event identity is stable across processes and sensitive to
//! content.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::event::PullRequestObservation;
use contributron_domain::pull_request::{
    AuthorLogin, Mergeable, ObservedPullRequest, PullRequestAuthor, PullRequestNumber,
    ReviewThread, ThreadId, ThreadResolution,
};

/// Written down before this test was first run, and derived independently of the
/// implementation: the SHA-256 of the canonical encoding of the observation `fixed_subject`
/// builds, obtained by piping that JSON through `sha256sum`:
///
/// ```text
/// {"number":7,"title":"slice 0007","draft":false,
///  "author":{"kind":"account","login":"Isonimus"},
///  "mergeable":"UNKNOWN",
///  "threads":[{"id":"T1","resolution":"unresolved"}]}
/// ```
///
/// A literal is used rather than a second call to `event_id`, because comparing the function
/// to itself would pass for any value including a per-process random one. This constant is
/// what makes the cross-process claim testable inside one process: a hasher seeded per
/// process could not reproduce it.
///
/// Updated in slice 0015 when `ObservedPullRequest` gained the `mergeable` field; the prior
/// value is preserved in git history to prove the hash really did move with the shape.
const EXPECTED_IDENTITY: &str = "5f2f36cbb50c57ca23aecdff7914b79bf60b2dd3d2b941ed0e896e8a309c4d8a";

fn author() -> PullRequestAuthor {
    PullRequestAuthor::Account {
        login: AuthorLogin::new("Isonimus").expect("a non-empty login"),
    }
}

fn thread(id: &str, resolution: ThreadResolution) -> ReviewThread {
    ReviewThread {
        id: ThreadId::new(id).expect("a non-empty identifier"),
        resolution,
    }
}

fn subject(threads: Vec<ReviewThread>) -> ObservedPullRequest {
    ObservedPullRequest::new(
        PullRequestNumber::new(7).expect("seven is a valid pull request number"),
        "slice 0007",
        false,
        author(),
        Mergeable::Unknown,
        threads,
    )
}

fn observation(pull_request: ObservedPullRequest, agent: ExecutingAgent) -> PullRequestObservation {
    let attribution = Attribution {
        user: UserId::new("local-operator").expect("static test id"),
        worker: WorkerId::new("cli").expect("static test id"),
        agent,
    };
    PullRequestObservation::new(attribution, pull_request)
}

fn identity_of(observation: &PullRequestObservation) -> String {
    observation
        .event_id()
        .expect("the observation encodes canonically")
        .as_str()
        .to_owned()
}

#[test]
fn identity_is_a_content_digest_an_independent_tool_can_reproduce() {
    let observed = observation(
        subject(vec![thread("T1", ThreadResolution::Unresolved)]),
        ExecutingAgent::NoAgent,
    );

    assert_eq!(identity_of(&observed), EXPECTED_IDENTITY);
}

#[test]
fn the_same_content_observed_by_a_different_actor_has_the_same_identity() {
    // Two workers polling the same repository have seen one fact, not two. An identity that
    // varied by observer would defeat the deduplication it exists for.
    let by_operator = observation(
        subject(vec![thread("T1", ThreadResolution::Unresolved)]),
        ExecutingAgent::NoAgent,
    );
    let by_agent = observation(
        subject(vec![thread("T1", ThreadResolution::Unresolved)]),
        ExecutingAgent::Agent {
            id: AgentId::new("reviewer-1").expect("static test id"),
        },
    );

    assert_eq!(identity_of(&by_operator), identity_of(&by_agent));
}

#[test]
fn provider_thread_ordering_does_not_change_identity() {
    // GitHub does not promise an order for a connection's nodes, so an identity sensitive to
    // it would report a change on every poll.
    let one_order = observation(
        subject(vec![
            thread("T1", ThreadResolution::Unresolved),
            thread("T2", ThreadResolution::Resolved),
        ]),
        ExecutingAgent::NoAgent,
    );
    let other_order = observation(
        subject(vec![
            thread("T2", ThreadResolution::Resolved),
            thread("T1", ThreadResolution::Unresolved),
        ]),
        ExecutingAgent::NoAgent,
    );

    assert_eq!(identity_of(&one_order), identity_of(&other_order));
}

#[test]
fn a_change_in_any_observed_field_changes_the_identity() {
    let baseline = identity_of(&observation(
        subject(vec![thread("T1", ThreadResolution::Unresolved)]),
        ExecutingAgent::NoAgent,
    ));

    let resolved_instead = identity_of(&observation(
        subject(vec![thread("T1", ThreadResolution::Resolved)]),
        ExecutingAgent::NoAgent,
    ));
    assert_ne!(baseline, resolved_instead, "resolving a thread is a change");

    let retitled = identity_of(&observation(
        ObservedPullRequest::new(
            PullRequestNumber::new(7).expect("seven is a valid pull request number"),
            "slice 0008",
            false,
            author(),
            Mergeable::Unknown,
            vec![thread("T1", ThreadResolution::Unresolved)],
        ),
        ExecutingAgent::NoAgent,
    ));
    assert_ne!(baseline, retitled, "retitling is a change");

    let now_draft = identity_of(&observation(
        ObservedPullRequest::new(
            PullRequestNumber::new(7).expect("seven is a valid pull request number"),
            "slice 0007",
            true,
            author(),
            Mergeable::Unknown,
            vec![thread("T1", ThreadResolution::Unresolved)],
        ),
        ExecutingAgent::NoAgent,
    ));
    assert_ne!(baseline, now_draft, "becoming a draft is a change");

    let extra_thread = identity_of(&observation(
        subject(vec![
            thread("T1", ThreadResolution::Unresolved),
            thread("T2", ThreadResolution::Unresolved),
        ]),
        ExecutingAgent::NoAgent,
    ));
    assert_ne!(baseline, extra_thread, "a new thread is a change");

    // Slice 0015: the mergeable field is observed content and must move the identity.
    let mergeable_flipped = identity_of(&observation(
        ObservedPullRequest::new(
            PullRequestNumber::new(7).expect("seven is a valid pull request number"),
            "slice 0007",
            false,
            author(),
            Mergeable::Mergeable,
            vec![thread("T1", ThreadResolution::Unresolved)],
        ),
        ExecutingAgent::NoAgent,
    ));
    assert_ne!(
        baseline, mergeable_flipped,
        "mergeable is observed content; a change in it is a change in the identity",
    );
}
