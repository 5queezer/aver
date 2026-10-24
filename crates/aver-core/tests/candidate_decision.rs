//! ADR-0024 decision-model seams for ADR-0014: candidate triage and the
//! extraction trigger, all driven by `MockDecisionClient` (no Ollama).

use aver_core::{CandidateWrite, EventWrite};
use std::path::Path;

use aver_core::candidate_decision::CandidateTriage;
use aver_core::decision::{DecisionError, MockDecisionClient};
use aver_core::{Error, ExtractionTriggerReason, Provenance, Store, replay};

const PROMOTE: usize = 0;
const REJECT: usize = 1;
const HOLD: usize = 2;
const EXTRACT: usize = 0;
const SKIP: usize = 1;
const FLOOR: f64 = 0.5;

fn delete_db(dir: &Path) {
    std::fs::remove_file(dir.join("db.sqlite")).unwrap();
    let _ = std::fs::remove_file(dir.join("db.sqlite-wal"));
    let _ = std::fs::remove_file(dir.join("db.sqlite-shm"));
}

fn read_log(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn log_lines_of_kind(dir: &Path, kind: &str) -> Vec<serde_json::Value> {
    read_log(dir, "events.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|value| value["kind"] == kind)
        .collect()
}

fn staged(store: &Store) -> i64 {
    let event_id = store
        .record_event(EventWrite::new(
            "s1",
            "note",
            "the service depends on postgres",
            "src",
        ))
        .unwrap();
    store
        .propose_candidate_claim(CandidateWrite::new(
            event_id,
            "service",
            "depends_on",
            "postgres",
        ))
        .unwrap()
}

#[test]
fn triage_promote_sets_inferred_and_logs_decision() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let client = MockDecisionClient::with_indices(vec![PROMOTE]);

    assert_eq!(
        store
            .triage_candidate_claim(candidate_id, &client, FLOOR)
            .unwrap(),
        CandidateTriage::Promote
    );

    let candidate = store.get_candidate_claim(candidate_id).unwrap();
    assert_eq!(candidate.status, "PROMOTED");
    let claim = store
        .get_claim(candidate.promoted_claim_id.unwrap())
        .unwrap();
    assert_eq!(claim.provenance, Provenance::Inferred);
    assert!((claim.confidence - 0.45).abs() < 1e-9);

    let lines = log_lines_of_kind(dir.path(), "promote_candidate_claim");
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0]["decision"],
        serde_json::json!({"model": "mock", "choice": "promote"})
    );

    let received = client.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].options, vec!["promote", "reject", "hold"]);
    assert!(received[0].state.contains("depends_on"));
    assert!(
        received[0]
            .state
            .contains("the service depends on postgres")
    );
    let state = &received[0].state;
    let at = |key: &str| state.find(key).unwrap();
    assert!(at("\"subject\"") < at("\"predicate\"") && at("\"predicate\"") < at("\"object\""));
}

#[test]
fn triage_reject_records_decision_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let client = MockDecisionClient::with_indices(vec![REJECT]);

    assert_eq!(
        store
            .triage_candidate_claim(candidate_id, &client, FLOOR)
            .unwrap(),
        CandidateTriage::Reject
    );

    let candidate = store.get_candidate_claim(candidate_id).unwrap();
    assert_eq!(candidate.status, "REJECTED");
    assert_eq!(candidate.rejection_reason.as_deref(), Some("decision:mock"));
    let lines = log_lines_of_kind(dir.path(), "reject_candidate_claim");
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0]["decision"],
        serde_json::json!({"model": "mock", "choice": "reject"})
    );
    assert_eq!(
        client.received()[0].options,
        vec!["promote", "reject", "hold"]
    );
}

#[test]
fn triage_hold_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let events_before = read_log(dir.path(), "events.jsonl");
    let claims_before = read_log(dir.path(), "log.jsonl");
    let client = MockDecisionClient::with_indices(vec![HOLD]);

    assert_eq!(
        store
            .triage_candidate_claim(candidate_id, &client, FLOOR)
            .unwrap(),
        CandidateTriage::Hold
    );

    assert_eq!(
        store.get_candidate_claim(candidate_id).unwrap().status,
        "PENDING"
    );
    assert_eq!(read_log(dir.path(), "events.jsonl"), events_before);
    assert_eq!(read_log(dir.path(), "log.jsonl"), claims_before);
}

#[test]
fn triage_unavailable_client_leaves_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let events_before = read_log(dir.path(), "events.jsonl");
    let claims_before = read_log(dir.path(), "log.jsonl");
    let client = MockDecisionClient::new(vec![Err(DecisionError::Unavailable)]);

    let err = store
        .triage_candidate_claim(candidate_id, &client, FLOOR)
        .unwrap_err();
    assert!(matches!(err, Error::Decision(DecisionError::Unavailable)));

    assert_eq!(
        store.get_candidate_claim(candidate_id).unwrap().status,
        "PENDING"
    );
    assert_eq!(read_log(dir.path(), "events.jsonl"), events_before);
    assert_eq!(read_log(dir.path(), "log.jsonl"), claims_before);
}

#[test]
fn triage_refuses_non_pending_candidate_without_asking() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    store.promote_candidate_claim(candidate_id).unwrap();
    let client = MockDecisionClient::with_indices(vec![REJECT]);

    assert!(matches!(
        store.triage_candidate_claim(candidate_id, &client, FLOOR),
        Err(Error::InvalidCandidateStatus { .. })
    ));
    assert!(client.received().is_empty());
}

#[test]
fn replay_with_decision_field_reproduces_claim_without_client() {
    let dir = tempfile::tempdir().unwrap();
    let (promoted_id, rejected_id, claim_id) = {
        let store = Store::open(dir.path()).unwrap();
        let promoted_id = staged(&store);
        let event_id = store
            .record_event(EventWrite::new("s1", "note", "noise", "src"))
            .unwrap();
        let rejected_id = store
            .propose_candidate_claim(CandidateWrite::new(
                event_id,
                "noise",
                "depends_on",
                "chatter",
            ))
            .unwrap();
        let client = MockDecisionClient::with_indices(vec![PROMOTE, REJECT]);
        store
            .triage_candidate_claim(promoted_id, &client, FLOOR)
            .unwrap();
        store
            .triage_candidate_claim(rejected_id, &client, FLOOR)
            .unwrap();
        let claim_id = store
            .get_candidate_claim(promoted_id)
            .unwrap()
            .promoted_claim_id
            .unwrap();
        store.close().unwrap();
        (promoted_id, rejected_id, claim_id)
    };

    delete_db(dir.path());
    let report = replay(dir.path(), false).expect("replay should succeed");
    assert_eq!(report.lifecycle, 4, "two proposals + promotion + rejection");

    let store = Store::open(dir.path()).unwrap();
    let promoted = store.get_candidate_claim(promoted_id).unwrap();
    assert_eq!(promoted.status, "PROMOTED");
    assert_eq!(promoted.promoted_claim_id, Some(claim_id));
    let claim = store.get_claim(claim_id).unwrap();
    assert_eq!(
        (
            claim.subject.as_str(),
            claim.predicate.as_str(),
            claim.object.as_str()
        ),
        ("service", "depends_on", "postgres")
    );
    assert_eq!(claim.provenance, Provenance::Inferred);
    assert!((claim.confidence - 0.45).abs() < 1e-9);
    let rejected = store.get_candidate_claim(rejected_id).unwrap();
    assert_eq!(rejected.status, "REJECTED");
    assert_eq!(rejected.rejection_reason.as_deref(), Some("decision:mock"));
}

#[test]
fn replay_tolerates_lines_without_decision_field() {
    let dir = tempfile::tempdir().unwrap();
    let candidate_id = {
        let store = Store::open(dir.path()).unwrap();
        let candidate_id = staged(&store);
        let client = MockDecisionClient::with_indices(vec![PROMOTE]);
        store
            .triage_candidate_claim(candidate_id, &client, FLOOR)
            .unwrap();
        store.close().unwrap();
        candidate_id
    };

    // Rewrite the log as a pre-ADR-0024 writer would have produced it.
    let events_path = dir.path().join("events.jsonl");
    let rewritten: String = read_log(dir.path(), "events.jsonl")
        .lines()
        .map(|line| {
            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
            value.as_object_mut().unwrap().remove("decision");
            format!("{value}\n")
        })
        .collect();
    assert!(!rewritten.contains("\"decision\""));
    std::fs::write(&events_path, rewritten).unwrap();

    delete_db(dir.path());
    replay(dir.path(), false).expect("old-format lines must replay");
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.get_candidate_claim(candidate_id).unwrap().status,
        "PROMOTED"
    );
}

#[test]
fn replay_rejects_decision_field_contradicting_line_kind() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let candidate_id = staged(&store);
        let client = MockDecisionClient::with_indices(vec![PROMOTE]);
        store
            .triage_candidate_claim(candidate_id, &client, FLOOR)
            .unwrap();
        store.close().unwrap();
    }
    let events_path = dir.path().join("events.jsonl");
    let tampered = read_log(dir.path(), "events.jsonl")
        .replace("\"choice\":\"promote\"", "\"choice\":\"reject\"");
    std::fs::write(&events_path, tampered).unwrap();

    delete_db(dir.path());
    assert!(matches!(
        replay(dir.path(), false),
        Err(Error::ReplayMalformed { .. })
    ));
}

fn undigested_session(store: &Store) {
    store
        .record_event(EventWrite::new(
            "s1",
            "note",
            "we chose sqlite for storage",
            "src",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new("s1", "note", "tests run offline", "src"))
        .unwrap();
}

#[test]
fn trigger_extract_adds_decision_model_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    undigested_session(&store);
    let events_before = read_log(dir.path(), "events.jsonl");
    let client = MockDecisionClient::with_indices(vec![EXTRACT]);

    let decision = store
        .extraction_decision_with_client("s1", 100, None, &client, FLOOR)
        .unwrap();
    assert!(decision.should_extract);
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::DecisionModel)
    );

    let received = client.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].options, vec!["extract", "skip"]);
    assert!(received[0].state.starts_with("undigested_events: 2\n"));
    assert!(received[0].state.contains("we chose sqlite for storage"));
    assert_eq!(
        read_log(dir.path(), "events.jsonl"),
        events_before,
        "triggers are not writes"
    );
}

#[test]
fn trigger_skip_or_error_returns_deterministic_decision() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    undigested_session(&store);
    let deterministic = store.extraction_decision("s1", 100, None).unwrap();

    let skip = MockDecisionClient::with_indices(vec![SKIP]);
    assert_eq!(
        store
            .extraction_decision_with_client("s1", 100, None, &skip, FLOOR)
            .unwrap(),
        deterministic
    );
    assert_eq!(skip.received()[0].options, vec!["extract", "skip"]);

    let unavailable = MockDecisionClient::new(vec![Err(DecisionError::Unavailable)]);
    assert_eq!(
        store
            .extraction_decision_with_client("s1", 100, None, &unavailable, FLOOR)
            .unwrap(),
        deterministic
    );
}

#[test]
fn trigger_skips_model_when_deterministic_reason_fired() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    undigested_session(&store);
    let client = MockDecisionClient::with_indices(vec![EXTRACT]);

    let decision = store
        .extraction_decision_with_client("s1", 2, None, &client, FLOOR)
        .unwrap();
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::EventCountThreshold)
    );
    assert!(
        !decision
            .reasons
            .contains(&ExtractionTriggerReason::DecisionModel)
    );
    assert!(client.received().is_empty());
}

#[test]
fn trigger_state_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let long = "word ".repeat(400);
    for _ in 0..40 {
        store
            .record_event(EventWrite::new("s1", "note", &long, "src"))
            .unwrap();
    }
    let client = MockDecisionClient::with_indices(vec![SKIP]);

    store
        .extraction_decision_with_client("s1", 1_000, None, &client, FLOOR)
        .unwrap();
    let state = &client.received()[0].state;
    assert!(state.starts_with("undigested_events: 40\n"));
    assert!(state.len() <= 4096, "state was {} bytes", state.len());
}

#[test]
fn triage_sends_criteria_descriptions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let client = MockDecisionClient::with_indices(vec![HOLD]);
    store
        .triage_candidate_claim(candidate_id, &client, FLOOR)
        .unwrap();
    assert_eq!(
        client.received()[0].descriptions.as_deref(),
        Some(
            &[
                "the claim is a specific, durable fact or preference that the source text clearly supports".to_string(),
                "the claim is wrong, trivial, conversational filler, or not supported by the source text".to_string(),
                "the claim may be useful but the source text is too ambiguous to decide".to_string(),
            ][..]
        )
    );
}

#[test]
fn triage_below_floor_holds_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let candidate_id = staged(&store);
    let events_before = read_log(dir.path(), "events.jsonl");
    let claims_before = read_log(dir.path(), "log.jsonl");
    let client =
        MockDecisionClient::with_choices(vec![(PROMOTE, Some(0.32)), (REJECT, Some(0.49))]);

    for _ in 0..2 {
        assert_eq!(
            store
                .triage_candidate_claim(candidate_id, &client, FLOOR)
                .unwrap(),
            CandidateTriage::Hold
        );
    }
    assert_eq!(
        store.get_candidate_claim(candidate_id).unwrap().status,
        "PENDING"
    );
    assert_eq!(read_log(dir.path(), "events.jsonl"), events_before);
    assert_eq!(read_log(dir.path(), "log.jsonl"), claims_before);
}

#[test]
fn triage_at_or_above_floor_acts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let promoted = staged(&store);
    let rejected = staged(&store);
    let client = MockDecisionClient::with_choices(vec![(PROMOTE, Some(0.78)), (REJECT, Some(0.5))]);

    assert_eq!(
        store
            .triage_candidate_claim(promoted, &client, FLOOR)
            .unwrap(),
        CandidateTriage::Promote
    );
    assert_eq!(
        store
            .triage_candidate_claim(rejected, &client, FLOOR)
            .unwrap(),
        CandidateTriage::Reject
    );
    assert_eq!(
        store.get_candidate_claim(promoted).unwrap().status,
        "PROMOTED"
    );
    assert_eq!(
        store.get_candidate_claim(rejected).unwrap().status,
        "REJECTED"
    );
}

#[test]
fn trigger_applies_floor_and_sends_descriptions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    undigested_session(&store);
    let deterministic = store.extraction_decision("s1", 100, None).unwrap();

    let unsure = MockDecisionClient::with_choices(vec![(EXTRACT, Some(0.4))]);
    assert_eq!(
        store
            .extraction_decision_with_client("s1", 100, None, &unsure, FLOOR)
            .unwrap(),
        deterministic
    );
    let descriptions = unsure.received()[0].descriptions.clone().unwrap();
    assert_eq!(descriptions.len(), 2);
    assert!(descriptions[0].contains("durable facts"));
    assert!(descriptions[1].contains("chatter"));

    let sure = MockDecisionClient::with_choices(vec![(EXTRACT, Some(0.9))]);
    let decision = store
        .extraction_decision_with_client("s1", 100, None, &sure, FLOOR)
        .unwrap();
    assert!(decision.should_extract);
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::DecisionModel)
    );
}
