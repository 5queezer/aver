//! `aver recall-misses` summarizes the aver-server recall-miss log
//! (roadmap Phase 4 trial) without opening the store.

use assert_cmd::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recall_misses")
}

fn run(args: &[&str]) -> String {
    let output = Command::cargo_bin("aver")
        .unwrap()
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "aver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn json_summary_counts_the_fixture() {
    let dir = fixture_dir();
    let out = run(&[
        "recall-misses",
        "--memory-dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    let summary: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(summary["total"], 4);
    assert_eq!(summary["miss"], 3);
    assert_eq!(summary["weak"], 1);
    assert_eq!(summary["skipped_lines"], 1);
    assert_eq!(summary["by_retrieval"]["hybrid"], 3);
    assert_eq!(summary["by_retrieval"]["keyword"], 1);
    assert_eq!(summary["by_scope_walk"]["ancestors"], 2);
    assert_eq!(summary["by_scope_walk"]["any"], 1);
    assert_eq!(summary["by_scope_walk"]["exact"], 1);
    assert_eq!(summary["top_scopes"][0]["scope"], "proj/aver");
    assert_eq!(summary["top_scopes"][0]["count"], 2);
    assert_eq!(summary["top_scopes"].as_array().unwrap().len(), 3);
    assert_eq!(summary["query_len"]["median"], 20);
    assert_eq!(summary["query_len"]["p90"], 40);
    assert_eq!(summary["alpha_histogram"]["0.7-0.8"], 2);
    assert_eq!(summary["alpha_histogram"]["0.9-1.0"], 1);
    assert_eq!(summary["alpha_histogram"]["none"], 1);
    assert_eq!(summary["alpha_histogram"]["0.0-0.1"], 0);
    assert_eq!(summary["per_day"]["2026-10-20"], 2);
    assert_eq!(summary["per_day"]["2026-10-21"], 1);
    assert_eq!(summary["per_day"]["2026-10-22"], 1);
}

#[test]
fn since_filters_earlier_events_and_text_output_reports_counts() {
    let dir = fixture_dir();
    // Global --memory-dir before the subcommand works too.
    let out = run(&[
        "--memory-dir",
        dir.to_str().unwrap(),
        "recall-misses",
        "--since",
        "2026-10-21",
    ]);
    assert!(out.contains("events: 2 (miss=1 weak=1)"), "{out}");
    assert!(out.contains("by retrieval: hybrid=2"), "{out}");
    assert!(out.contains("query_len: median=30 p90=40"), "{out}");
    assert!(!out.contains("2026-10-20"), "{out}");
}

#[test]
fn missing_log_exits_zero_with_message() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&[
        "recall-misses",
        "--memory-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert!(out.contains("no recall-miss log at"), "{out}");
    assert!(
        !dir.path().join("db.sqlite").exists(),
        "recall-misses must not open the store"
    );
}

#[test]
fn invalid_since_is_rejected() {
    let dir = fixture_dir();
    let output = Command::cargo_bin("aver")
        .unwrap()
        .args([
            "recall-misses",
            "--memory-dir",
            dir.to_str().unwrap(),
            "--since",
            "last week",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
