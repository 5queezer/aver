use aver_core::{CandidateWrite, EventWrite};
use aver_core::{Provenance, Store};

#[test]
fn inferred_temporal_decay_uses_exponential_delta_over_tau() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(EventWrite::new("s", "tool", "payload", "test"))
        .unwrap();
    let candidate = store
        .propose_candidate_claim(CandidateWrite::new(event, "auth", "uses", "jwt"))
        .unwrap();
    let claim_id = store.promote_candidate_claim(candidate).unwrap();
    let original = store.get_claim(claim_id).unwrap();

    let decayed = store
        .decay_inferred_confidence_at(original.write_ts + 10, 10.0)
        .unwrap();

    assert_eq!(decayed, 1);
    let claim = store.get_claim(claim_id).unwrap();
    let expected = original.confidence * std::f64::consts::E.powf(-1.0);
    assert!((claim.confidence - expected).abs() < 1e-9);
}

#[test]
fn consolidation_keeps_inferred_claim_after_two_unverified_sources() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first_event = store
        .record_event(EventWrite::new("s", "tool", "first", "source:a"))
        .unwrap();
    let second_event = store
        .record_event(EventWrite::new("s", "tool", "second", "source:b"))
        .unwrap();
    let first = store
        .propose_candidate_claim(CandidateWrite::new(first_event, "auth", "uses", "jwt"))
        .unwrap();
    let second = store
        .propose_candidate_claim(CandidateWrite::new(second_event, "auth", "uses", "jwt"))
        .unwrap();
    let first_claim = store.promote_candidate_claim(first).unwrap();
    store.promote_candidate_claim(second).unwrap();

    store.consolidate_report().unwrap();

    let survivor = store.get_claim(first_claim).unwrap();
    assert_eq!(survivor.provenance, Provenance::Inferred);
    assert_eq!(survivor.source_refs.len(), 2);
    assert_eq!(survivor.confidence, 0.45);
}
