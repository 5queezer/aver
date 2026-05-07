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
