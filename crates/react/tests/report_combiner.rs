//! Slice 0016: `combine_rule_results` folds a slice of per-rule results into a single
//! `ReactionReport`. These tests exercise the three cases the slice calls out explicitly:
//! both rules write (both agent ids survive), one writes and one errors (the write is
//! reported, the error rides along in `errors`), and every rule errors (the aggregated
//! report is `Failed`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::assert_is_empty
)]

use contributron_domain::actor::{AgentId, Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::pull_request::Label;
use contributron_poll::ReactionReport;
use contributron_react::{
    LABEL_RULE_AGENT_ID, MERGE_RULE_AGENT_ID, ProviderError, RULE_AGENT_ID, ReactError,
    ReactionKind, ReactionOutcome, ReactionReceipt, combine_rule_results,
};

fn attribution(agent_id: &str) -> Attribution {
    Attribution {
        user: UserId::new("alice").expect("static test id"),
        worker: WorkerId::new("poll").expect("static test id"),
        agent: ExecutingAgent::Agent {
            id: AgentId::new(agent_id).expect("static test id"),
        },
    }
}

fn commented(agent_id: &str, body: &str) -> ReactionOutcome {
    ReactionOutcome::Posted {
        receipt: ReactionReceipt {
            attribution: attribution(agent_id),
            kind: ReactionKind::Commented {
                body: body.to_owned(),
            },
        },
    }
}

fn label_removed(agent_id: &str) -> ReactionOutcome {
    let label = Label::new("needs-rebase").expect("label");
    ReactionOutcome::Posted {
        receipt: ReactionReceipt {
            attribution: attribution(agent_id),
            kind: ReactionKind::LabelRemoved { label },
        },
    }
}

/// DoD 5: a tick in which the merge-conflicts-cleared rule posts a comment *and* the
/// needs-rebase-label rule removes the label records both writes with distinct agent ids.
#[test]
fn two_rule_writes_in_one_tick_both_appear_in_receipts() {
    let results = vec![
        (RULE_AGENT_ID, Ok(ReactionOutcome::NotApplicable)),
        (
            MERGE_RULE_AGENT_ID,
            Ok(commented(MERGE_RULE_AGENT_ID, "merge-cleared body")),
        ),
        (LABEL_RULE_AGENT_ID, Ok(label_removed(LABEL_RULE_AGENT_ID))),
    ];

    let report = combine_rule_results(&results);

    match report {
        ReactionReport::Posted { agent_ids, errors } => {
            assert_eq!(
                agent_ids,
                vec![
                    MERGE_RULE_AGENT_ID.to_owned(),
                    LABEL_RULE_AGENT_ID.to_owned(),
                ],
                "both writes must appear, in order",
            );
            assert!(errors.is_empty(), "no peer errors, got {errors:?}");
        }
        other => panic!("expected Posted, got {other:?}"),
    }
}

/// A rule posts and a peer errors: the Posted side survives, the error rides along.
#[test]
fn one_rule_posts_and_one_errors_still_reports_the_post() {
    let results = vec![
        (
            RULE_AGENT_ID,
            Ok(commented(RULE_AGENT_ID, "threads-cleared body")),
        ),
        (
            MERGE_RULE_AGENT_ID,
            Err(ReactError::Provider(ProviderError::Client(
                "rate limited".to_owned(),
            ))),
        ),
    ];

    let report = combine_rule_results(&results);

    match report {
        ReactionReport::Posted { agent_ids, errors } => {
            assert_eq!(agent_ids, vec![RULE_AGENT_ID.to_owned()]);
            assert_eq!(errors.len(), 1, "one peer error survives");
            assert_eq!(errors[0].0, MERGE_RULE_AGENT_ID);
            assert!(
                errors[0].1.contains("rate limited"),
                "error message dropped: {}",
                errors[0].1,
            );
        }
        other => panic!("expected Posted, got {other:?}"),
    }
}

/// All rules error: the combiner aggregates into one `Failed` with every agent id named.
#[test]
fn all_rules_error_reports_failed_with_each_agent_id() {
    let results = vec![
        (
            RULE_AGENT_ID,
            Err(ReactError::Provider(ProviderError::Client(
                "boom-threads".to_owned(),
            ))),
        ),
        (
            MERGE_RULE_AGENT_ID,
            Err(ReactError::Provider(ProviderError::Client(
                "boom-merge".to_owned(),
            ))),
        ),
        (
            LABEL_RULE_AGENT_ID,
            Err(ReactError::History("boom-history".to_owned())),
        ),
    ];

    let report = combine_rule_results(&results);

    match report {
        ReactionReport::Failed { error } => {
            assert!(
                error.contains(RULE_AGENT_ID) && error.contains("boom-threads"),
                "threads error should name its rule and message, got: {error}",
            );
            assert!(
                error.contains(MERGE_RULE_AGENT_ID) && error.contains("boom-merge"),
                "merge error should name its rule and message, got: {error}",
            );
            assert!(
                error.contains(LABEL_RULE_AGENT_ID) && error.contains("boom-history"),
                "label error should name its rule and message, got: {error}",
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// Empty input is total: the combiner reports `NotApplicable` rather than panicking.
#[test]
fn an_empty_input_slice_reports_not_applicable() {
    let results: Vec<(&'static str, Result<ReactionOutcome, ReactError>)> = Vec::new();
    let report = combine_rule_results(&results);
    assert!(matches!(report, ReactionReport::NotApplicable));
}

/// At least one AlreadySettled with no writes gives an AlreadySettled report.
#[test]
fn at_least_one_already_settled_with_no_write_gives_already_settled() {
    let results = vec![
        (RULE_AGENT_ID, Ok(ReactionOutcome::NotApplicable)),
        (LABEL_RULE_AGENT_ID, Ok(ReactionOutcome::AlreadySettled)),
    ];
    let report = combine_rule_results(&results);
    match report {
        ReactionReport::AlreadySettled { agent_ids, errors } => {
            assert_eq!(agent_ids, vec![LABEL_RULE_AGENT_ID.to_owned()]);
            assert!(errors.is_empty());
        }
        other => panic!("expected AlreadySettled, got {other:?}"),
    }
}
