use std::collections::BTreeMap;

use aver_core::{ConsolidationReport, ExtractionTriggerReason, PrivacyRejection, Store};

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

    let mut privacy = BTreeMap::new();
    privacy.insert(PrivacyRejection::OpenAiKey, 2_u64);
    privacy.insert(PrivacyRejection::SecretsPath, 1_u64);

    let snapshot = store
        .graph_drift_snapshot(
            ConsolidationReport {
                merged: 1,
                superseded: 0,
                decayed: 0,
            },
            privacy,
        )
        .unwrap();

    assert_eq!(
        snapshot.claim_count_by_provenance.get("USER_ASSERTED"),
        Some(&1)
    );
    assert_eq!(snapshot.privacy_rejection_counts.get("OpenAiKey"), Some(&2));
    assert_eq!(
        snapshot.privacy_rejection_counts.get("SecretsPath"),
        Some(&1)
    );
    assert_eq!(snapshot.consolidation_merged, 1);
}
