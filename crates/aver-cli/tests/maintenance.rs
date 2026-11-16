use std::{path::Path, process::Command, process::Output};

use assert_cmd::prelude::*;

fn aver(memory_dir: &Path) -> Command {
    let mut command = Command::cargo_bin("aver").unwrap();
    command.arg("--memory-dir").arg(memory_dir);
    command
}

fn run(memory_dir: &Path, args: &[&str]) -> Output {
    let output = aver(memory_dir).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "aver {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout(output: Output) -> String {
    String::from_utf8(output.stdout).unwrap()
}

fn remember(memory_dir: &Path, subject: &str, predicate: &str, object: &str) -> i64 {
    let output = stdout(run(
        memory_dir,
        &["remember", subject, predicate, object, "--source", "test"],
    ));
    output
        .trim()
        .strip_prefix("claim_id=")
        .unwrap()
        .parse()
        .unwrap()
}

fn record_event(memory_dir: &Path) -> i64 {
    let output = stdout(run(
        memory_dir,
        &[
            "record-event",
            "--session-id",
            "synthetic",
            "--kind",
            "decision",
            "--payload",
            "Invented evidence",
        ],
    ));
    output
        .trim()
        .strip_prefix("event_id=")
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn expand_command_prints_local_graph() {
    let dir = tempfile::tempdir().unwrap();
    remember(dir.path(), "service", "depends_on", "library");
    remember(dir.path(), "library", "depends_on", "database");

    let output = stdout(run(dir.path(), &["expand", "service", "--hops", "2"]));

    assert!(output.contains("service,library,database"), "{output}");
    assert!(output.contains("service depends_on library"), "{output}");
    assert!(output.contains("library depends_on database"), "{output}");
}

#[test]
fn reverify_command_records_claim_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let claim_id = remember(dir.path(), "service", "uses", "database");
    let event_id = record_event(dir.path());

    let output = stdout(run(
        dir.path(),
        &[
            "reverify",
            "--claim-id",
            &claim_id.to_string(),
            "--evidence",
            &format!("event:{event_id}"),
        ],
    ));

    assert!(output.contains(&format!("claim_id={claim_id}")), "{output}");
    assert!(output.contains("verified_at="), "{output}");
}

#[test]
fn confirmations_command_lists_claim_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let claim_id = remember(dir.path(), "service", "uses", "database");
    let event_id = record_event(dir.path());
    run(
        dir.path(),
        &[
            "reverify",
            "--claim-id",
            &claim_id.to_string(),
            "--evidence",
            &format!("event:{event_id}"),
        ],
    );

    let output = stdout(run(
        dir.path(),
        &["confirmations", "--claim-id", &claim_id.to_string()],
    ));

    assert!(output.contains("agent_id=local"), "{output}");
    assert!(
        output.contains(&format!("evidence=event:{event_id}")),
        "{output}"
    );
}

#[test]
fn contradict_command_records_contradiction() {
    let dir = tempfile::tempdir().unwrap();
    let claim_id = remember(dir.path(), "service", "uses", "database");

    let output = stdout(run(
        dir.path(),
        &[
            "contradict",
            "--triple-id",
            &claim_id.to_string(),
            "--reason",
            "invented conflicting evidence",
        ],
    ));

    assert_eq!(output.trim(), "contradicted");
    let store = aver_core::Store::open(dir.path()).unwrap();
    let contradictions = store.list_contradictions(claim_id).unwrap();
    assert_eq!(contradictions.len(), 1);
    assert_eq!(contradictions[0].reason, "invented conflicting evidence");
}

#[test]
fn consolidate_command_supersedes_duplicate_claims() {
    let dir = tempfile::tempdir().unwrap();
    remember(dir.path(), "service", "uses", "database");
    remember(dir.path(), "service", "uses", "database");

    let output = stdout(run(dir.path(), &["consolidate"]));

    assert!(output.contains("superseded=1"), "{output}");
}

#[test]
fn vacuum_command_writes_analyzed_copy() {
    let dir = tempfile::tempdir().unwrap();
    remember(dir.path(), "service", "uses", "database");
    let copy = dir.path().join("vacuumed.sqlite");

    let output = stdout(run(
        dir.path(),
        &["vacuum", "--analyze", "--into", copy.to_str().unwrap()],
    ));

    assert!(copy.exists());
    assert!(output.contains("vacuum: pages"), "{output}");
    assert!(
        output.contains(&format!("into: {}", copy.display())),
        "{output}"
    );
}

#[test]
fn migrate_literals_command_reclassifies_legacy_objects_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("log.jsonl"),
        concat!(
            r#"{"kind":"add_claim","ts":1,"claim_id":1,"subject":"sample","predicate":"concerns","object":"An invented complete sentence.","source":"synthetic","agent_id":"mcp","agent_kind":"EXTERNAL_TOOL","confidence":0.9,"provenance":"EXTRACTED","scope":"global"}"#,
            "\n"
        ),
    )
    .unwrap();
    run(dir.path(), &["replay", "--force"]);

    let migrated = stdout(run(dir.path(), &["migrate-literals"]));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&migrated).unwrap(),
        serde_json::json!({"claims_reclassified": 1, "entities_removed": 1})
    );
    let repeated = stdout(run(dir.path(), &["migrate-literals"]));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&repeated).unwrap(),
        serde_json::json!({"claims_reclassified": 0, "entities_removed": 0})
    );
}
