use aver_core::{ConsolidationReport, Error, ExtractionTriggerReason, PrivacyRejection, Store};

#[test]
fn extraction_decision_reports_richer_deterministic_trigger_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .record_event("s1", "user_message", "first durable detail", "conversation")
        .unwrap();
    store
        .record_event(
            "s1",
            "correction",
            "Correction: API clients use PKCE S256.",
            "conversation",
        )
        .unwrap();
    store
        .record_event("s1", "commit_completed", "commit abc123 completed", "git")
        .unwrap();
    store
        .record_event("s1", "session_end", "session ended", "agent")
        .unwrap();
    store
        .record_event(
            "s1",
            "idle_compaction",
            "agent idle for compaction",
            "agent",
        )
        .unwrap();

    let decision = store.extraction_decision("s1", 3, Some(6)).unwrap();

    assert!(decision.should_extract);
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::EventCountThreshold)
    );
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::ObservationTokenThreshold)
    );
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::Correction)
    );
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::CommitCompleted)
    );
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::SessionEnd)
    );
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::IdleCompaction)
    );
}

#[test]
fn graph_drift_snapshot_includes_privacy_rejection_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_claim("Aver", "uses", "SQLite", "test").unwrap();

    let event_id = store
        .record_event("s1", "user_message", "ordinary event", "test")
        .unwrap();
    let token = "sk-abcdefghijklmnopqrstuvwxyz1234567890";
    assert!(matches!(
        store.record_event(
            "s1",
            "user_message",
            &format!("OPENAI_API_KEY={token}"),
            "test"
        ),
        Err(Error::Privacy(PrivacyRejection::OpenAiKey))
    ));
    assert!(matches!(
        store.add_claim("Aver", "source", "ssh key", ".ssh/id_rsa"),
        Err(Error::Privacy(PrivacyRejection::SshPath))
    ));
    assert!(matches!(
        store.record_observation(
            "s1",
            &format!("observer saw OPENAI_API_KEY={token}"),
            aver_core::ObservationRelevance::High,
            &[event_id],
            "observer"
        ),
        Err(Error::Privacy(PrivacyRejection::OpenAiKey))
    ));
    assert!(matches!(
        store.propose_candidate_claim(event_id, "Aver", "mentions", token),
        Err(Error::Privacy(PrivacyRejection::OpenAiKey))
    ));
    assert!(matches!(
        store.privacy_filter_path_recording(".secrets.d/token"),
        Err(Error::Privacy(PrivacyRejection::SecretsPath))
    ));

    let snapshot = store
        .graph_drift_snapshot(ConsolidationReport {
            merged: 1,
            superseded: 0,
            decayed: 0,
        })
        .unwrap();

    assert_eq!(
        snapshot.claim_count_by_provenance.get("USER_ASSERTED"),
        Some(&1)
    );
    assert_eq!(
        snapshot.privacy_rejection_counts.get("regex:openai"),
        Some(&3)
    );
    assert_eq!(
        snapshot.privacy_rejection_counts.get("path:secrets-dir"),
        Some(&1)
    );
    assert_eq!(snapshot.privacy_rejection_counts.get("path:ssh"), Some(&1));
    assert_eq!(snapshot.consolidation_merged, 1);
}
