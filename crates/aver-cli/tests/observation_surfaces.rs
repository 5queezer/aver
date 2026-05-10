use assert_cmd::prelude::*;
use std::process::Command;

fn aver(memory_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd
}

#[test]
fn observation_coverage_command_is_required_for_continuity_checks() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    // Seed events/observation so coverage can report meaningful gaps when the
    // command is implemented.
    let first = aver(memory)
        .args([
            "record-event",
            "--session-id",
            "obs-session",
            "--kind",
            "message",
            "--payload",
            "first detail",
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_stdout = String::from_utf8(first.stdout).unwrap();
    let first_id: i64 = first_stdout
        .trim()
        .strip_prefix("event_id=")
        .unwrap()
        .parse()
        .unwrap();

    let second = aver(memory)
        .args([
            "record-event",
            "--session-id",
            "obs-session",
            "--kind",
            "message",
            "--payload",
            "second detail",
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_stdout = String::from_utf8(second.stdout).unwrap();
    let second_id: i64 = second_stdout
        .trim()
        .strip_prefix("event_id=")
        .unwrap()
        .parse()
        .unwrap();

    let obs_output = aver(memory)
        .args([
            "record-observation",
            "--session-id",
            "obs-session",
            "--content",
            "only first summarized",
            "--relevance",
            "high",
            "--source-event-ids",
            &first_id.to_string(),
            "--derivation",
            "test",
        ])
        .output()
        .unwrap();
    assert!(obs_output.status.success());

    // New command expected by Slice 8: exposes coverage accounting for event id ranges.
    let coverage = aver(memory)
        .args(["observation-coverage", "--session-id", "obs-session"])
        .output()
        .unwrap();

    assert!(
        coverage.status.success(),
        "observation-coverage command should exist and report coverage state",
    );
    let stdout = String::from_utf8(coverage.stdout).unwrap();
    assert!(stdout.contains(&first_id.to_string()));
    assert!(stdout.contains(&second_id.to_string()));
}

#[test]
fn catch_up_command_is_required_for_observation_projection_runs() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    let output = aver(memory).args(["catch-up", "--help"]).output().unwrap();

    assert!(
        output.status.success(),
        "catch-up command should be available for triggering projection catch-up",
    );
}
