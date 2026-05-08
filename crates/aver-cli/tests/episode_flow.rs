use assert_cmd::prelude::*;
use std::process::Command;

fn aver(memory_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd
}

#[test]
fn record_event_propose_promote_recall_integration() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    // Record an event.
    let event_output = aver(memory)
        .args([
            "record-event",
            "--session-id",
            "s1",
            "--kind",
            "message",
            "--payload",
            "I prefer tabs",
        ])
        .output()
        .unwrap();
    assert!(event_output.status.success(), "record-event failed");
    let stdout = String::from_utf8(event_output.stdout).unwrap();
    let event_id: i64 = stdout
        .trim()
        .strip_prefix("event_id=")
        .unwrap()
        .parse()
        .unwrap();

    // Check should-extract-memories.
    let output = aver(memory)
        .args([
            "should-extract-memories",
            "--session-id",
            "s1",
            "--event-threshold",
            "1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("should_extract=true")
    );

    // Propose a candidate.
    let propose_output = aver(memory)
        .args([
            "propose",
            "--event-id",
            &event_id.to_string(),
            "User",
            "prefers",
            "tabs",
        ])
        .output()
        .unwrap();
    assert!(propose_output.status.success(), "propose failed");
    let propose_stdout = String::from_utf8(propose_output.stdout).unwrap();
    let candidate_id: i64 = propose_stdout
        .trim()
        .strip_prefix("candidate_id=")
        .unwrap()
        .parse()
        .unwrap();

    // List candidates.
    let list_output = aver(memory)
        .args([
            "list-candidates",
            "--session-id",
            "s1",
            "--status",
            "PENDING",
        ])
        .output()
        .unwrap();
    assert!(list_output.status.success());
    assert!(
        String::from_utf8(list_output.stdout)
            .unwrap()
            .contains("User")
    );

    // Promote candidate.
    let promote_output = aver(memory)
        .args(["promote", "--candidate-id", &candidate_id.to_string()])
        .output()
        .unwrap();
    assert!(promote_output.status.success());
    assert!(
        String::from_utf8(promote_output.stdout)
            .unwrap()
            .contains("claim_id=")
    );

    // Recall the promoted claim.
    let recall_output = aver(memory).args(["recall", "tabs"]).output().unwrap();
    assert!(recall_output.status.success());
    assert!(
        String::from_utf8(recall_output.stdout)
            .unwrap()
            .contains("User")
    );
}

#[test]
fn reject_candidate_sets_status() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    let event_output = aver(memory)
        .args([
            "record-event",
            "--session-id",
            "s2",
            "--kind",
            "msg",
            "--payload",
            "I prefer spaces",
        ])
        .output()
        .unwrap();
    assert!(event_output.status.success());
    let stdout = String::from_utf8(event_output.stdout).unwrap();
    let event_id: i64 = stdout
        .trim()
        .strip_prefix("event_id=")
        .unwrap()
        .parse()
        .unwrap();

    let propose_output = aver(memory)
        .args([
            "propose",
            "--event-id",
            &event_id.to_string(),
            "User",
            "prefers",
            "spaces",
        ])
        .output()
        .unwrap();
    assert!(propose_output.status.success());
    let propose_stdout = String::from_utf8(propose_output.stdout).unwrap();
    let candidate_id: i64 = propose_stdout
        .trim()
        .strip_prefix("candidate_id=")
        .unwrap()
        .parse()
        .unwrap();

    let reject_output = aver(memory)
        .args([
            "reject",
            "--candidate-id",
            &candidate_id.to_string(),
            "--reason",
            "duplicate",
        ])
        .output()
        .unwrap();
    assert!(reject_output.status.success());
    assert!(
        String::from_utf8(reject_output.stdout)
            .unwrap()
            .contains("rejected")
    );

    let list_output = aver(memory)
        .args(["list-candidates", "--status", "REJECTED"])
        .output()
        .unwrap();
    assert!(list_output.status.success());
    assert!(
        String::from_utf8(list_output.stdout)
            .unwrap()
            .contains("REJECTED")
    );
}
