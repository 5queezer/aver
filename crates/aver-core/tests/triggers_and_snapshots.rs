use aver_core::{EventWrite, ExtractionTriggerReason, Store};

#[test]
fn extraction_decision_reports_richer_deterministic_trigger_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .record_event(EventWrite::new(
            "s1",
            "user_message",
            "first durable detail",
            "conversation",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "correction",
            "Correction: API clients use PKCE S256.",
            "conversation",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "commit_completed",
            "commit abc123 completed",
            "git",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "session_end",
            "session ended",
            "agent",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "idle_compaction",
            "agent idle for compaction",
            "agent",
        ))
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
fn extraction_decision_surfaces_uncovered_coverage_gap() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    store
        .record_event(EventWrite::new(
            "s1",
            "user_message",
            "first uncovered event",
            "conversation",
        ))
        .unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "assistant_action",
            "second uncovered event",
            "tool",
        ))
        .unwrap();

    let decision = store.extraction_decision("s1", 100, Some(1_000)).unwrap();

    assert!(decision.should_extract);
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::UncoveredCoverageGap)
    );
}

#[test]
fn extraction_decision_surfaces_coverage_gap_without_token_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .record_event(EventWrite::new(
            "s1",
            "user_message",
            "uncovered event",
            "conversation",
        ))
        .unwrap();

    // The coverage signal must not depend on a token threshold being set.
    let decision = store.extraction_decision("s1", 100, None).unwrap();

    assert!(decision.should_extract);
    assert!(
        decision
            .reasons
            .contains(&ExtractionTriggerReason::UncoveredCoverageGap)
    );
}
