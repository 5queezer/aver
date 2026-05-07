use memory_core::{AgentKind, Store};

#[test]
fn record_event_persists_episode_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "user_message",
            "Remember that AML should batch memory extraction.",
            "conversation",
        )
        .unwrap();

    let event = store.get_event(event_id).unwrap();

    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.kind, "user_message");
    assert_eq!(
        event.payload,
        "Remember that AML should batch memory extraction."
    );
    assert_eq!(event.source, "conversation");
    assert_eq!(event.agent_id, "local");
    assert_eq!(event.agent_kind, AgentKind::Human);
    assert!(event.ts > 0);
}

#[test]
fn record_event_appends_to_episodic_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "explicit_remember",
            "AML should not write durable memory after every message.",
            "conversation",
        )
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();

    assert_eq!(entry["kind"], "record_event");
    assert_eq!(entry["event_id"], serde_json::json!(event_id));
    assert_eq!(entry["session_id"], "session-1");
    assert_eq!(entry["event_kind"], "explicit_remember");
    assert_eq!(entry["agent_id"], "local");
}

#[test]
fn propose_candidate_claim_requires_event_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "user_message",
            "AML should batch memory extraction.",
            "conversation",
        )
        .unwrap();

    let candidate_id = store
        .propose_candidate_claim(event_id, "AML", "should_batch", "memory_extraction")
        .unwrap();

    let candidate = store.get_candidate_claim(candidate_id).unwrap();

    assert_eq!(candidate.event_id, event_id);
    assert_eq!(candidate.subject, "AML");
    assert_eq!(candidate.predicate, "should_batch");
    assert_eq!(candidate.object, "memory_extraction");
    assert_eq!(candidate.status, "PENDING");
    assert!(store.recall_text("memory_extraction").unwrap().is_empty());

    let err = store
        .propose_candidate_claim(9999, "ghost", "has", "no_event")
        .unwrap_err();
    assert!(matches!(
        err,
        memory_core::Error::MissingEventProvenance { .. }
    ));
}

#[test]
fn promote_candidate_claim_creates_durable_claim_with_event_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "user_message",
            "AML should batch memory extraction.",
            "conversation",
        )
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "AML", "should_batch", "memory_extraction")
        .unwrap();

    let claim_id = store.promote_candidate_claim(candidate_id).unwrap();
    let claim = store.get_claim(claim_id).unwrap();
    let candidate = store.get_candidate_claim(candidate_id).unwrap();

    assert_eq!(claim.subject, "AML");
    assert_eq!(claim.predicate, "should_batch");
    assert_eq!(claim.object, "memory_extraction");
    assert_eq!(claim.provenance, memory_core::Provenance::Inferred);
    assert_eq!(claim.source_refs, vec![format!("event:{event_id}")]);
    assert_eq!(candidate.status, "PROMOTED");
    assert_eq!(candidate.promoted_claim_id, Some(claim_id));
}

#[test]
fn reject_candidate_claim_marks_rejected_with_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "assistant_observation",
            "Maybe AML uses Redis for storage.",
            "conversation",
        )
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "AML", "uses", "Redis")
        .unwrap();

    store
        .reject_candidate_claim(candidate_id, "unsupported by source")
        .unwrap();

    let candidate = store.get_candidate_claim(candidate_id).unwrap();

    assert_eq!(candidate.status, "REJECTED");
    assert_eq!(
        candidate.rejection_reason,
        Some("unsupported by source".to_string())
    );
    assert!(store.recall_text("Redis").unwrap().is_empty());
}
