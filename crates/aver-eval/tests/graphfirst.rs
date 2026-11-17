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
/// assertions pin *characterized* behavior — Q6 was a review finding fixed
/// in a2ce61f, Q10 (before the migration) now falls back to hybrid with
/// `temporal_constraint` — and must change loudly if core regresses. The
/// report is the verdict.
#[test]
fn graph_first_arms_run_the_real_entry_point() {
    let fixture = fixture();
    let report = run_ops_fixture(&fixture).expect("comparison runs");

    for arm in &report.arms[1..] {
        assert_eq!(arm.aggregates.ok_questions, 20);
        // Every question names an anchor; only the one temporal question
        // relative to an event (Q10, "before the migration") falls back.
        assert_eq!(arm.aggregates.fallback_rate, Some(0.05));
        let fell_back: Vec<(&str, Option<&str>)> = arm
            .questions
            .iter()
            .filter(|row| row.fell_back == Some(true))
            .map(|row| (row.query.as_str(), row.fallback_reason.as_deref()))
            .collect();
        assert_eq!(
            fell_back,
            vec![(
                "Which Stripe API version was checkout-service using before the migration?",
                Some("temporal_constraint")
            )]
        );
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
    // Q6 (latest Stripe API): fixed in a2ce61f — versioned entities anchor
    // on their words, so the newest stripe-api claim wins.
    let latest = by_query("What is the latest Stripe API version used by checkout-service?");
    assert_eq!(latest.ranked_indices.first(), Some(&18));
    assert_eq!(latest.answer_quality, 1.0);
    assert_eq!(latest.evidence_coverage, 1.0);
    // Q10 (before the migration): word cues cannot place "before X", so
    // graph-first declines and the hybrid fallback answers, as well as the
    // hybrid arm does (it ranked the CURRENT version first before this).
    let q10 = "Which Stripe API version was checkout-service using before the migration?";
    let before = by_query(q10);
    let hybrid_before = report.arms[0]
        .questions
        .iter()
        .find(|row| row.query == q10)
        .expect("query present");
    assert_eq!(before.ranked_indices, hybrid_before.ranked_indices);
    assert_eq!(before.answer_quality, hybrid_before.answer_quality);

    // Count questions get an exact count from the graph-first arms (the
    // generic-noun anchor dilution was fixed in a2ce61f).
    for row in graph_first
        .questions
        .iter()
        .filter(|row| row.route == "count")
    {
        assert_eq!(row.count_correct, Some(true), "{}", row.query);
    }
}
