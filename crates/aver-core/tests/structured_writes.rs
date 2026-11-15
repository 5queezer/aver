use aver_core::{AgentKind, ClaimWrite, EventWrite, Provenance, Store, replay};

#[test]
fn linked_claims_validate_scope_and_replay_after_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(
            EventWrite::new("invented", "decision", "Synthetic reasoning.", "synthetic")
                .scope("proj/sample"),
        )
        .unwrap();
    let write = ClaimWrite::from_agent(
        "model",
        AgentKind::Llm,
        "decision:sample",
        "concerns",
        "topic:sample",
        "synthetic",
    )
    .scope("proj/sample")
    .event_id(event);
    let id = store.add_claim(write.clone()).unwrap();
    let before = std::fs::read(dir.path().join("log.jsonl")).unwrap();
    assert!(store.add_claim(write.clone().event_id(event + 1)).is_err());
    assert!(store.add_claim(write.scope("proj/other")).is_err());
    assert_eq!(std::fs::read(dir.path().join("log.jsonl")).unwrap(), before);
    assert_eq!(store.get_claim(id).unwrap().event_id, Some(event));
    assert!(
        store
            .get_claim(id)
            .unwrap()
            .source_refs
            .contains(&format!("event:{event}"))
    );
    store.close().unwrap();
    for _ in 0..2 {
        replay(dir.path(), true).unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.get_claim(id).unwrap().event_id, Some(event));
        assert_eq!(store.get_event(event).unwrap().scope, "proj/sample");
        store.close().unwrap();
    }
}

#[test]
fn strict_prose_rejection_has_no_claim_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let err = store
        .add_claim(
            ClaimWrite::new("sample", "concerns", "An invented sentence.", "synthetic")
                .strict_atomic(true),
        )
        .unwrap_err();
    assert!(err.to_string().contains("atomic"));
    assert!(!dir.path().join("log.jsonl").exists());
    assert!(
        store
            .add_claim(
                ClaimWrite::new(
                    "sample",
                    "concerns",
                    r#""An invented sentence.""#,
                    "synthetic"
                )
                .strict_atomic(true)
            )
            .is_err()
    );
    // A primitive literal is already atomic.
    store
        .add_claim(ClaimWrite::new("sample", "concerns", "-0.18", "synthetic").strict_atomic(true))
        .unwrap();
}

#[test]
fn repeated_model_judgments_cannot_become_extracted_by_repetition() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let first = store
        .add_claim(ClaimWrite::from_agent(
            "model",
            AgentKind::Llm,
            "sample",
            "uses",
            "widget",
            "reasoning-one",
        ))
        .unwrap();
    store
        .add_claim(ClaimWrite::from_agent(
            "model",
            AgentKind::Llm,
            "sample",
            "uses",
            "widget",
            "reasoning-two",
        ))
        .unwrap();
    store.consolidate_report().unwrap();
    let claim = store.get_claim(first).unwrap();
    assert_eq!(claim.provenance, Provenance::Inferred);
    assert_eq!(claim.confidence, 0.45);
    assert_eq!(claim.source_refs.len(), 2);
}

#[test]
fn replay_refuses_dangling_or_cross_scope_typed_event_links_without_replacing_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let event = store
        .record_event(EventWrite::new(
            "s",
            "decision",
            "Synthetic rationale",
            "synthetic",
        ))
        .unwrap();
    let id = store
        .add_claim(ClaimWrite::new("sample", "concerns", "widget", "synthetic").event_id(event))
        .unwrap();
    store.close().unwrap();
    let original = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let mut claim: serde_json::Value = serde_json::from_str(original.trim()).unwrap();
    for bad in [
        serde_json::json!({"event_id":event + 1}),
        serde_json::json!({"scope":"proj/other"}),
    ] {
        let mut changed = claim.clone();
        changed
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        std::fs::write(dir.path().join("log.jsonl"), format!("{changed}\n")).unwrap();
        assert!(replay(dir.path(), true).is_err());
        assert_eq!(
            Store::open(dir.path())
                .unwrap()
                .get_claim(id)
                .unwrap()
                .event_id,
            Some(event)
        );
    }
    // Duplicate claim IDs with different source links are not idempotent.
    claim["event_id"] = serde_json::json!(event + 1);
    std::fs::write(dir.path().join("log.jsonl"), format!("{original}{claim}\n")).unwrap();
    assert!(replay(dir.path(), true).is_err());
}

#[test]
fn new_inferred_hyperedges_are_capped_and_historical_claim_scores_are_retained() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let id = store
        .add_hyperedge(aver_core::HyperedgeInput {
            predicate: "concerns".into(),
            provenance: Provenance::Inferred,
            confidence: 0.99,
            source_refs: vec!["synthetic".into()],
            participants: vec![aver_core::HyperedgeParticipantInput {
                role: "context".into(),
                entity: "sample".into(),
            }],
        })
        .unwrap();
    assert_eq!(store.get_hyperedge(id).unwrap().confidence, 0.75);
    store.close().unwrap();
    use std::io::Write;
    let historical = serde_json::json!({"kind":"add_claim","ts":1,"claim_id":1,"subject":"legacy","predicate":"concerns","object":"widget","source":"synthetic","agent_id":"model","agent_kind":"LLM","confidence":0.99,"provenance":"INFERRED","scope":"global"});
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("log.jsonl"))
            .unwrap(),
        "{historical}"
    )
    .unwrap();
    replay(dir.path(), true).unwrap();
    assert_eq!(
        Store::open(dir.path())
            .unwrap()
            .get_claim(1)
            .unwrap()
            .confidence,
        0.99
    );
}

#[test]
fn consolidation_keeps_direct_claims_and_replacements_in_their_own_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut ids = Vec::new();
    for (scope, owner) in [("proj/one", "team:one"), ("proj/two", "team:two")] {
        let event = store
            .record_event(
                EventWrite::new("synthetic", "decision", "Invented rationale.", "synthetic")
                    .scope(scope),
            )
            .unwrap();
        for (predicate, object) in [("concerns", "topic:sample"), ("owned_by", owner)] {
            let id = store
                .add_claim(
                    ClaimWrite::new("decision:invented", predicate, object, "synthetic")
                        .scope(scope)
                        .event_id(event),
                )
                .unwrap();
            ids.push((id, event));
        }
    }
    store.consolidate_report().unwrap();
    for (id, event) in ids {
        let claim = store.get_claim(id).unwrap();
        assert_eq!(claim.status, aver_core::ClaimStatus::Active);
        assert_eq!(
            claim.source_refs,
            vec!["synthetic".to_string(), format!("event:{event}")]
        );
    }
}
