//! T7 — `aver remember` appends a user-asserted claim through Store::add_claim.

use std::process::Command;

#[test]
fn remember_writes_claim_to_configured_store() {
    let dir = std::env::temp_dir().join(format!("aver-cli-remember-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let output = Command::new(env!("CARGO_BIN_EXE_aver"))
        .args([
            "--memory-dir",
            dir.to_str().unwrap(),
            "remember",
            "auth_service",
            "depends_on",
            "stripe_sdk",
            "--source",
            "cli_test",
        ])
        .output()
        .expect("aver binary should run");

    assert!(output.status.success(), "remember command should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("claim_id=1"),
        "stdout should report inserted claim id, got {stdout:?}"
    );

    let log = std::fs::read_to_string(dir.join("log.jsonl")).expect("remember should append log");
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(entry["subject"], "auth_service");
    assert_eq!(entry["predicate"], "depends_on");
    assert_eq!(entry["object"], "stripe_sdk");
    assert_eq!(entry["source"], "cli_test");

    let _ = std::fs::remove_dir_all(&dir);
}
