use aver_core::{Provenance, Store};

#[test]
fn synonym_merge_threshold_requires_at_least_point_92_similarity() {
    assert!(!Store::should_merge_synonym(0.919_999));
    assert!(Store::should_merge_synonym(0.92));
}

#[test]
fn inferred_temporal_decay_uses_exponential_delta_over_tau() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store.record_event("s", "tool", "payload", "test").unwrap();
    let candidate = store
        .propose_candidate_claim(event, "auth", "uses", "jwt")
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
fn consolidation_promotes_inferred_claim_after_two_corroborating_sources() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first_event = store
        .record_event("s", "tool", "first", "source:a")
        .unwrap();
    let second_event = store
        .record_event("s", "tool", "second", "source:b")
        .unwrap();
    let first = store
        .propose_candidate_claim(first_event, "auth", "uses", "jwt")
        .unwrap();
    let second = store
        .propose_candidate_claim(second_event, "auth", "uses", "jwt")
        .unwrap();
    let first_claim = store.promote_candidate_claim(first).unwrap();
    store.promote_candidate_claim(second).unwrap();

    store.consolidate_report().unwrap();

    let survivor = store.get_claim(first_claim).unwrap();
    assert_eq!(survivor.provenance, Provenance::Extracted);
    assert_eq!(survivor.source_refs.len(), 2);
    assert!(survivor.confidence >= 0.75);
}
