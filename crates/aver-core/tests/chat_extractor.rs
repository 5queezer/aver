use aver_core::{AgentKind, ClaimExtractor, EpisodicEvent, Store, extractor::ChatEventExtractor};

fn make_event(id: i64, payload: &str) -> EpisodicEvent {
    EpisodicEvent {
        id,
        session_id: "s1".to_string(),
        kind: "message".to_string(),
        payload: payload.to_string(),
        source: "test".to_string(),
        agent_id: "human".to_string(),
        agent_kind: AgentKind::Human,
        ts: 0,
        scope: "global".to_string(),
    }
}

#[test]
fn i_prefer_tabs_over_spaces_produces_user_prefers_tabs() {
    let extractor = ChatEventExtractor;
    let event = make_event(1, "I prefer tabs over spaces");
    let drafts = extractor.extract(&[event]).unwrap();
    assert!(!drafts.is_empty(), "expected at least one draft");
    let draft = drafts.iter().find(|d| d.predicate == "prefers").unwrap();
    assert_eq!(draft.subject, "User");
    assert!(
        draft.object.to_lowercase().contains("tabs"),
        "expected 'tabs' in object, got '{}'",
        draft.object
    );
}

#[test]
fn i_decided_to_use_rust_produces_user_decides() {
    let extractor = ChatEventExtractor;
    let event = make_event(1, "I decided to use Rust");
    let drafts = extractor.extract(&[event]).unwrap();
    assert!(!drafts.is_empty());
    let draft = drafts.iter().find(|d| d.predicate == "decides").unwrap();
    assert_eq!(draft.subject, "User");
}

#[test]
fn the_project_uses_sqlite_produces_project_candidate() {
    let extractor = ChatEventExtractor;
    let event = make_event(1, "The project uses SQLite");
    let drafts = extractor.extract(&[event]).unwrap();
    assert!(!drafts.is_empty());
    let draft = drafts.iter().find(|d| d.subject == "Project").unwrap();
    assert_eq!(draft.predicate, "uses");
}

#[test]
fn we_are_using_axum_for_http_produces_at_least_one_candidate() {
    let extractor = ChatEventExtractor;
    let event = make_event(1, "we are using axum for HTTP");
    let drafts = extractor.extract(&[event]).unwrap();
    assert!(!drafts.is_empty(), "expected at least one candidate");
}

#[test]
fn generic_message_produces_no_candidates() {
    let extractor = ChatEventExtractor;
    let event = make_event(1, "Hello world");
    let drafts = extractor.extract(&[event]).unwrap();
    assert!(
        drafts.is_empty(),
        "expected no candidates for generic message, got: {drafts:?}"
    );
}

#[test]
fn full_store_integration_record_propose_promote_recall() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    // Record an event with a preference payload.
    let event_id = store
        .record_event("session-1", "message", "I prefer tabs over spaces", "cli")
        .unwrap();

    // Extract via ChatEventExtractor.
    let extractor = ChatEventExtractor;
    let candidate_ids = store
        .propose_claims_from_extractor("session-1", &extractor)
        .unwrap();
    assert!(
        !candidate_ids.is_empty(),
        "extractor should produce candidates"
    );

    // Promote the first candidate.
    let claim_id = store.promote_candidate_claim(candidate_ids[0]).unwrap();

    // Recall it.
    let claim = store.get_claim(claim_id).unwrap();
    assert_eq!(claim.subject, "User");
    assert_eq!(claim.predicate, "prefers");
    assert!(
        claim.object.to_lowercase().contains("tabs"),
        "expected 'tabs' in object, got '{}'",
        claim.object
    );

    // Verify it shows up in recall_text.
    let recalled = store.recall_text("tabs").unwrap();
    assert!(recalled.iter().any(|c| c.id == claim_id));

    // Also verify event_id is tracked.
    let candidate = store.get_candidate_claim(candidate_ids[0]).unwrap();
    assert_eq!(candidate.event_id, event_id);
}
