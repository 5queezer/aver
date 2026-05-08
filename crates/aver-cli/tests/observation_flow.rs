use assert_cmd::prelude::*;
use std::process::Command;

fn aver(memory_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd
}

#[test]
fn record_observation_and_compaction_summary() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    // Record an event to attach observation to.
    let event_output = aver(memory)
        .args([
            "record-event",
            "--session-id",
            "obs-session",
            "--kind",
            "tool_result",
            "--payload",
            "cargo test passed",
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

    // Record observation.
    let obs_output = aver(memory)
        .args([
            "record-observation",
            "--session-id",
            "obs-session",
            "--content",
            "cargo test passed on first attempt",
            "--relevance",
            "high",
            "--source-event-ids",
            &event_id.to_string(),
            "--derivation",
            "direct-observation",
        ])
        .output()
        .unwrap();
    assert!(obs_output.status.success(), "record-observation failed");
    let obs_stdout = String::from_utf8(obs_output.stdout).unwrap();
    assert!(
        obs_stdout.contains("observation_id="),
        "expected observation_id in: {obs_stdout}"
    );
    let obs_id = obs_stdout.trim().strip_prefix("observation_id=").unwrap();

    // Recall observation.
    let recall_output = aver(memory)
        .args(["recall-observation", "--id", obs_id])
        .output()
        .unwrap();
    assert!(recall_output.status.success());
    assert!(
        String::from_utf8(recall_output.stdout)
            .unwrap()
            .contains("cargo test passed")
    );

    // Compaction summary.
    let summary_output = aver(memory)
        .args(["compaction-summary", "--session-id", "obs-session"])
        .output()
        .unwrap();
    assert!(summary_output.status.success());
    assert!(
        String::from_utf8(summary_output.stdout)
            .unwrap()
            .contains("[high]")
    );
}
