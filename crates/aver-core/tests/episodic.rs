use aver_core::{AgentKind, MockObserver, ObservationDraft, ObservationRelevance, Store};

#[test]
fn record_event_persists_episode_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "user_message",
            "Remember that Aver should batch memory extraction.",
            "conversation",
        )
        .unwrap();

    let event = store.get_event(event_id).unwrap();

    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.kind, "user_message");
    assert_eq!(
        event.payload,
        "Remember that Aver should batch memory extraction."
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
            "Aver should not write durable memory after every message.",
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
            "Aver should batch memory extraction.",
            "conversation",
        )
        .unwrap();

    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "concerns", "memory_extraction")
        .unwrap();

    let candidate = store.get_candidate_claim(candidate_id).unwrap();

    assert_eq!(candidate.event_id, event_id);
    assert_eq!(candidate.subject, "Aver");
    assert_eq!(candidate.predicate, "concerns");
    assert_eq!(candidate.object, "memory_extraction");
    assert_eq!(candidate.status, "PENDING");
    assert!(store.recall_text("memory_extraction").unwrap().is_empty());

    let err = store
        .propose_candidate_claim(9999, "ghost", "has", "no_event")
        .unwrap_err();
    assert!(matches!(
        err,
        aver_core::Error::MissingEventProvenance { .. }
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
            "Aver should batch memory extraction.",
            "conversation",
        )
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "concerns", "memory_extraction")
        .unwrap();

    let claim_id = store.promote_candidate_claim(candidate_id).unwrap();
    let claim = store.get_claim(claim_id).unwrap();
    let candidate = store.get_candidate_claim(candidate_id).unwrap();

    assert_eq!(claim.subject, "Aver");
    assert_eq!(claim.predicate, "concerns");
    assert_eq!(claim.object, "memory_extraction");
    assert_eq!(claim.provenance, aver_core::Provenance::Inferred);
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
            "Maybe Aver uses Redis for storage.",
            "conversation",
        )
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "Redis")
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

#[test]
fn promote_candidate_claim_twice_returns_existing_claim() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let event_id = store
        .record_event(
            "session-1",
            "explicit_remember",
            "Aver uses triggered episodic memory.",
            "conversation",
        )
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "Aver", "uses", "triggered_episodic_memory")
        .unwrap();

    let first_claim_id = store.promote_candidate_claim(candidate_id).unwrap();
    let second_claim_id = store.promote_candidate_claim(candidate_id).unwrap();

    assert_eq!(second_claim_id, first_claim_id);
    assert_eq!(
        store
            .recall_text("triggered_episodic_memory")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn should_extract_memories_triggers_on_explicit_remember_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    assert!(!store.should_extract_memories("session-1", 3).unwrap());

    store
        .record_event(
            "session-1",
            "explicit_remember",
            "Remember that Aver batches durable memory writes.",
            "conversation",
        )
        .unwrap();

    assert!(store.should_extract_memories("session-1", 3).unwrap());
}

#[test]
fn should_extract_memories_triggers_on_event_count_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .record_event(
            "session-1",
            "user_message",
            "first relevant fact",
            "conversation",
        )
        .unwrap();
    assert!(!store.should_extract_memories("session-1", 2).unwrap());

    store
        .record_event(
            "session-1",
            "tool_result",
            "second relevant fact",
            "conversation",
        )
        .unwrap();
    assert!(store.should_extract_memories("session-1", 2).unwrap());
}

#[test]
fn record_event_rejects_empty_session_id_before_log_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .record_event(" ", "message", "hello", "test")
        .expect_err("blank session ids should be rejected");

    assert!(err.to_string().contains("session_id"));
    let log_path = dir.path().join("events.jsonl");
    assert!(
        !log_path.exists(),
        "invalid events must not reach the event log"
    );
}

#[test]
fn propose_candidate_claim_rejects_empty_subject() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store
        .record_event("s1", "message", "remember something", "test")
        .unwrap();

    let err = store
        .propose_candidate_claim(event_id, " ", "depends_on", "StripeSDK")
        .expect_err("blank candidate subjects should be rejected");

    assert!(err.to_string().contains("subject"));
}

#[test]
fn reject_candidate_claim_requires_existing_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .reject_candidate_claim(42, "not supported")
        .expect_err("rejecting a missing candidate should fail");

    assert!(err.to_string().contains("candidate"));
}

#[test]
fn reject_candidate_claim_rejects_empty_reason() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store
        .record_event("s1", "message", "remember something", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "PaymentGateway", "depends_on", "StripeSDK")
        .unwrap();

    let err = store
        .reject_candidate_claim(candidate_id, " ")
        .expect_err("blank rejection reasons should be rejected");

    assert!(err.to_string().contains("rejection reason"));
}

#[test]
fn promote_candidate_claim_rejects_rejected_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store
        .record_event("s1", "message", "remember something", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "PaymentGateway", "depends_on", "StripeSDK")
        .unwrap();
    store
        .reject_candidate_claim(candidate_id, "not supported by source")
        .unwrap();

    let err = store
        .promote_candidate_claim(candidate_id)
        .expect_err("rejected candidates should not be promoted");

    assert!(err.to_string().contains("candidate"));
}

#[test]
fn reject_candidate_claim_rejects_promoted_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event_id = store
        .record_event("s1", "message", "remember something", "test")
        .unwrap();
    let candidate_id = store
        .propose_candidate_claim(event_id, "PaymentGateway", "depends_on", "StripeSDK")
        .unwrap();
    store.promote_candidate_claim(candidate_id).unwrap();

    let err = store
        .reject_candidate_claim(candidate_id, "not supported by source")
        .expect_err("promoted candidates should not be rejected later");

    assert!(err.to_string().contains("candidate"));
}

#[test]
fn should_extract_memories_rejects_zero_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .should_extract_memories("s1", 0)
        .expect_err("zero extraction thresholds should be rejected");

    assert!(err.to_string().contains("event threshold"));
}

#[test]
fn list_candidate_claims_rejects_unknown_status_filter() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .list_candidate_claims(None, Some("UNKNOWN"))
        .expect_err("unknown candidate status filters should be rejected");

    assert!(err.to_string().contains("candidate status"));
}

#[test]
fn list_events_for_session_rejects_empty_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .list_events_for_session(" ")
        .expect_err("blank session filters should be rejected");

    assert!(err.to_string().contains("session_id"));
}

#[test]
fn should_extract_memories_rejects_empty_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .should_extract_memories(" ", 1)
        .expect_err("blank extraction session ids should be rejected");

    assert!(err.to_string().contains("session_id"));
}

#[test]
fn list_candidate_claims_rejects_empty_session_filter() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .list_candidate_claims(Some(" "), None)
        .expect_err("blank candidate session filters should be rejected");

    assert!(err.to_string().contains("session_id"));
}

#[test]
fn get_event_requires_existing_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .get_event(42)
        .expect_err("fetching a missing event should return a domain error");

    assert!(err.to_string().contains("event"));
}

#[test]
fn get_candidate_claim_requires_existing_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .get_candidate_claim(42)
        .expect_err("fetching a missing candidate should return a domain error");

    assert!(err.to_string().contains("candidate"));
}

#[test]
fn record_event_rejects_empty_source_before_log_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .record_event("session-1", "message", "hello", " ")
        .expect_err("blank event sources should be rejected before audit logging");

    assert!(err.to_string().contains("source"));
    assert!(!dir.path().join("events.jsonl").exists());
}

#[test]
fn observer_projects_source_backed_observations() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store
        .record_event_from_agent(
            "claude",
            AgentKind::Llm,
            "session-1",
            "user_message",
            "Use observation projections for compaction continuity.",
            "conversation",
        )
        .unwrap();
    let second = store
        .record_event_from_agent(
            "claude",
            AgentKind::Llm,
            "session-1",
            "tool_result",
            "ADR-0016 was accepted.",
            "tool",
        )
        .unwrap();

    let observer = MockObserver::new(vec![ObservationDraft {
        content: "User accepted ADR-0016 for episodic observation projections.".to_string(),
        relevance: ObservationRelevance::High,
        source_event_ids: vec![first, second],
        derivation: "mock-observer".to_string(),
    }]);

    let ids = store
        .propose_observations_from_observer("session-1", &observer)
        .unwrap();
    let observation = store.get_observation(&ids[0]).unwrap();

    assert_eq!(observation.session_id, "session-1");
    assert_eq!(observation.agent_id, "claude");
    assert_eq!(observation.agent_kind, AgentKind::Llm);
    assert_eq!(observation.relevance, ObservationRelevance::High);
    assert_eq!(observation.source_event_ids, vec![first, second]);
    assert_eq!(observation.derivation, "mock-observer");
}

#[test]
fn record_observation_rejects_missing_event_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .record_observation(
            "session-1",
            "unsupported observation",
            ObservationRelevance::Medium,
            &[999],
            "mock-observer",
        )
        .expect_err("observations must cite existing events");

    assert!(err.to_string().contains("event"));
}

#[test]
fn record_observation_applies_privacy_filter_before_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event("session-1", "user_message", "safe payload", "conversation")
        .unwrap();

    let err = store
        .record_observation(
            "session-1",
            "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz1234567890",
            ObservationRelevance::Critical,
            &[event],
            "mock-observer",
        )
        .expect_err("secret-bearing observations must be rejected");

    assert!(err.to_string().contains("privacy"));
    assert!(
        store
            .list_observations_for_session("session-1")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn recall_observation_returns_exact_source_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(
            "session-1",
            "tool_result",
            "Build failed: E0425 cannot find function assemble_compaction_summary.",
            "cargo test",
        )
        .unwrap();
    let observation_id = store
        .record_observation(
            "session-1",
            "Build failed: E0425 cannot find function assemble_compaction_summary.",
            ObservationRelevance::High,
            &[event],
            "mock-observer",
        )
        .unwrap();

    let recalled = store.recall_observation(&observation_id).unwrap();

    assert_eq!(recalled.observation.id, observation_id);
    assert_eq!(recalled.events.len(), 1);
    assert_eq!(recalled.events[0].id, event);
    assert_eq!(
        recalled.events[0].payload,
        "Build failed: E0425 cannot find function assemble_compaction_summary."
    );
}

#[test]
fn assemble_compaction_summary_mechanically_renders_observations() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(
            "session-1",
            "user_message",
            "Use observations.",
            "conversation",
        )
        .unwrap();
    store
        .record_observation(
            "session-1",
            "User asked Aver to use observations for session continuity.",
            ObservationRelevance::Medium,
            &[event],
            "mock-observer",
        )
        .unwrap();

    let summary = store.assemble_compaction_summary("session-1").unwrap();

    assert!(summary.contains("# Aver session continuity summary"));
    assert!(summary.contains("[medium] User asked Aver to use observations"));
    assert!(summary.contains(&format!("source_events=[{event}]")));
}

#[test]
fn prune_observations_removes_lowest_relevance_projection_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(
            "session-1",
            "user_message",
            "Use observations.",
            "conversation",
        )
        .unwrap();
    let low = store
        .record_observation(
            "session-1",
            "Routine status update.",
            ObservationRelevance::Low,
            &[event],
            "mock-observer",
        )
        .unwrap();
    let high = store
        .record_observation(
            "session-1",
            "User decided observations must preserve source provenance.",
            ObservationRelevance::High,
            &[event],
            "mock-observer",
        )
        .unwrap();

    let pruned = store.prune_observations("session-1", 1).unwrap();
    let remaining = store.list_observations_for_session("session-1").unwrap();

    assert_eq!(pruned, 1);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, high);
    assert!(store.get_observation(&low).is_err());
    assert_eq!(store.get_event(event).unwrap().payload, "Use observations.");
}
