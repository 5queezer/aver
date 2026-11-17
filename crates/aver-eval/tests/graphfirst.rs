//! Tests for the graph-first ops evaluation: fixture well-formedness,
//! replay-equivalent seeding with linked events, the live hybrid baseline,
//! and the integration seam's honest `unimplemented` rows until
//! `Store::recall_graph_first` lands on this branch.

use aver_eval::graphfirst::{RecallArm, load_ops_fixture, run_ops_fixture};

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
        assert!(row.ranked_indices.iter().all(|&i| i < 37));
        assert!(row.payload_tokens > 0);
        assert!(row.latency_micros > 0);
    }

    // The easy control (strong token match on a distinctive object) must be
    // a perfect hit for every arm; if this breaks, the harness is wrong,
    // not the recall.
    for arm in &report.arms {
        let control = arm
            .questions
            .iter()
            .find(|row| row.query == "Where is checkout-service deployed?")
            .expect("control query present");
        assert_eq!(control.answer_quality, 1.0, "{:?} control", arm.arm);
        assert_eq!(control.evidence_coverage, 1.0, "{:?} control", arm.arm);
    }

    // Means are over the 20 rows and bounded.
    let mean_quality = hybrid.aggregates.mean_answer_quality.unwrap();
    assert!((0.0..=1.0).contains(&mean_quality));
    assert!(hybrid.aggregates.p95_latency_micros.is_some());
}

/// The graph-first arms run the real ADR-0033 entry point. The temporal
/// assertions pin *characterized* behavior — two are known review findings
/// and must change loudly when core fixes them; the test asserts harness
/// wiring, not a quality verdict. The report is the verdict.
#[test]
fn graph_first_arms_run_the_real_entry_point() {
    let fixture = fixture();
    let report = run_ops_fixture(&fixture).expect("comparison runs");

    for arm in &report.arms[1..] {
        assert_eq!(arm.aggregates.ok_questions, 20);
        // Every routed question names an anchor, so nothing falls back.
        assert_eq!(arm.aggregates.fallback_rate, Some(0.0));
        assert!(arm.questions.iter().all(|row| row.fell_back == Some(false)));
        // Graph-first returns evidence-bearing payloads everywhere.
        assert!(arm.questions.iter().all(|row| row.payload_tokens > 0));
    }

    let graph_first = &report.arms[1];
    let by_query = |query: &str| {
        graph_first
            .questions
            .iter()
            .find(|row| row.query == query)
            .expect("query present")
    };
    // FINDING (anchor coverage): stripe-api-v1/v3 tokenize with digit
    // splits, so a query naming "Stripe API" covers only 2/5 tokens and
    // they cannot anchor. The newest stripe-matching claim in the bounded
    // set — an incident claim — wins instead of the expected claim 18.
    let latest = by_query("What is the latest Stripe API version used by checkout-service?");
    assert_eq!(latest.ranked_indices.first(), Some(&30));
    assert_eq!(latest.answer_quality, 0.0);
    assert_eq!(latest.evidence_coverage, 0.0);
    // FINDING (no stemming): 'using' does not prefix-match 'uses', so
    // narrowing keeps only the two runs_version noise claims and the newer
    // noise claim wins instead of the expected claim 17.
    let before =
        by_query("Which Stripe API version was checkout-service using before the migration?");
    assert_eq!(before.ranked_indices.first(), Some(&34));
    assert_eq!(before.answer_quality, 0.0);

    // Count questions get a count verdict from the graph-first arms.
    for row in graph_first
        .questions
        .iter()
        .filter(|row| row.route == "count")
    {
        assert!(row.count_correct.is_some());
    }
}
