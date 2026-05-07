use memory_core::{CandidateClaimDraft, ClaimExtractor, MockClaimExtractor, Store};

#[test]
fn mock_claim_extractor_proposes_candidate_claims_from_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store
        .record_event(
            "session-1",
            "explicit_remember",
            "Remember that AML should batch memory extraction.",
            "conversation",
        )
        .unwrap();
    let event = store.get_event(event_id).unwrap();
    let extractor = MockClaimExtractor::new(vec![CandidateClaimDraft {
        event_id,
        subject: "AML".to_string(),
        predicate: "should_batch".to_string(),
        object: "memory_extraction".to_string(),
    }]);

    let drafts = extractor.extract(&[event]).unwrap();

    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].event_id, event_id);
    assert_eq!(drafts[0].subject, "AML");
    assert_eq!(drafts[0].predicate, "should_batch");
    assert_eq!(drafts[0].object, "memory_extraction");
}
