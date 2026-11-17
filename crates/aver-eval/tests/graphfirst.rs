//! Tests for the graph-first ops evaluation: fixture well-formedness,
//! replay-equivalent seeding with linked events, the live hybrid baseline,
//! and the integration seam's honest `unimplemented` rows until
//! `Store::recall_graph_first` lands on this branch.

use aver_eval::graphfirst::{
    ArmQueryOutcome, RecallArm, RowStatus, load_ops_fixture, run_ops_fixture,
};

const FIXTURE_JSON: &str = include_str!("../../../eval/fixtures/graph_first_ops.json");

fn fixture() -> aver_eval::graphfirst::OpsFixture {
    load_ops_fixture(FIXTURE_JSON).expect("fixture parses and validates")
}

#[test]
fn ops_fixture_is_well_formed() {
    let fixture = fixture();
    assert_eq!(fixture.claims.len(), 37);
    assert_eq!(fixture.queries.len(), 20);
    assert_eq!(fixture.now, 1_794_873_600, "2026-11-17T00:00:00Z");

    let route_count = |route: &str| fixture.queries.iter().filter(|q| q.route == route).count();
    assert_eq!(route_count("single"), 5);
    assert_eq!(route_count("temporal"), 5);
    assert_eq!(route_count("intersection"), 4);
    assert_eq!(route_count("list"), 4);
    assert_eq!(route_count("count"), 2);

    // Every temporal trap names its stale competitor; the inverse control
    // (stale claim is the correct answer) must exist.
    let temporals: Vec<_> = fixture
        .queries
        .iter()
        .filter(|q| q.route == "temporal")
        .collect();
    assert_eq!(
        temporals.iter().filter(|q| q.stale_index.is_some()).count(),
        4
    );
    let inverse = temporals
        .iter()
        .find(|q| q.stale_index.is_none())
        .expect("one temporal query without a stale trap");
    assert_eq!(inverse.expected_answers, ["stripe-api-v1"]);

    // Expected answers must match the relevant claims' text, otherwise the
    // answer-quality metric scores the fixture, not the recall.
    for q in &fixture.queries {
        let relevant_text: String = q
            .relevant_indices
            .iter()
            .map(|&i| {
                let c = &fixture.claims[i];
                format!("{} {} {}", c.subject, c.predicate, c.object)
            })
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        for answer in &q.expected_answers {
            assert!(
                relevant_text.contains(&answer.to_ascii_lowercase()),
                "query {:?}: expected answer {answer:?} not in relevant claims",
                q.query
            );
        }
    }
}

#[test]
fn invalid_fixtures_are_rejected() {
    let bad_route = FIXTURE_JSON.replace(
        "\"query\": \"What database does billing-service use?\",\n      \"route\": \"single\"",
        "\"query\": \"What database does billing-service use?\",\n      \"route\": \"sideways\"",
    );
    assert!(load_ops_fixture(&bad_route).is_err());

    let missing_count = FIXTURE_JSON.replace("\"expected_count\": 3", "\"expected_count\": null");
    assert!(load_ops_fixture(&missing_count).is_err());
}

#[test]
fn comparison_report_shape_and_hybrid_baseline() {
    let fixture = fixture();
    let report = run_ops_fixture(&fixture).expect("comparison runs");

    assert_eq!(report.claim_count, 37);
    assert_eq!(report.event_count, 37, "one source event per claim");
    assert_eq!(report.question_count, 20);
    assert_eq!(report.arms.len(), 3);
    assert_eq!(
        report.arms.iter().map(|a| a.arm).collect::<Vec<_>>(),
        vec![
            RecallArm::Hybrid,
            RecallArm::GraphFirst,
            RecallArm::GraphFirstRecency
        ]
    );

    let hybrid = &report.arms[0];
    assert_eq!(hybrid.aggregates.ok_questions, 20);
    assert!(hybrid.aggregates.fallback_rate.is_none());
    for row in &hybrid.questions {
        assert_eq!(row.status, RowStatus::Ok);
        assert!(row.ranked_indices.iter().all(|&i| i < 37));
        assert!(row.payload_tokens > 0);
        assert!(row.latency_micros > 0);
    }

    // The easy control (strong token match on a distinctive object) must be
    // a perfect hit for the current default; if this breaks, the harness is
    // wrong, not the recall.
    let control: &ArmQueryOutcome = hybrid
        .questions
        .iter()
        .find(|row| row.query == "Where is checkout-service deployed?")
        .expect("control query present");
    assert_eq!(control.answer_quality, 1.0);
    assert_eq!(control.evidence_coverage, 1.0);

    // Means are over the 20 ok rows and bounded.
    let mean_quality = hybrid.aggregates.mean_answer_quality.unwrap();
    assert!((0.0..=1.0).contains(&mean_quality));
    assert!(hybrid.aggregates.p95_latency_micros.is_some());
}

/// Until the integration seam is wired to `Store::recall_graph_first`, both
/// graph-first arms must report every question as `unimplemented` with empty
/// (not fabricated) aggregates. INTEGRATION: invert this test once
/// graph_first_recall calls the real entry point.
#[test]
fn graph_first_arms_report_unimplemented_until_integration() {
    let fixture = fixture();
    let report = run_ops_fixture(&fixture).expect("comparison runs");

    for arm in &report.arms[1..] {
        assert_eq!(arm.aggregates.ok_questions, 0);
        assert!(arm.aggregates.mean_answer_quality.is_none());
        assert!(arm.aggregates.mean_payload_tokens.is_none());
        assert!(arm.aggregates.fallback_rate.is_none());
        assert!(
            arm.questions
                .iter()
                .all(|row| row.status == RowStatus::Unimplemented)
        );
        assert!(
            arm.questions
                .iter()
                .all(|row| row.ranked_indices.is_empty())
        );
    }
}
