//! T8 — `aver recall` searches remembered claims by keyword.

use std::process::Command;

#[test]
fn recall_prints_claim_matching_keyword() {
    let dir = std::env::temp_dir().join(format!("aver-cli-recall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let remember = Command::new(env!("CARGO_BIN_EXE_aver"))
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
        .expect("aver remember should run");
    assert!(remember.status.success(), "remember setup should succeed");

    let output = Command::new(env!("CARGO_BIN_EXE_aver"))
        .args(["--memory-dir", dir.to_str().unwrap(), "recall", "stripe"])
        .output()
        .expect("aver recall should run");

    assert!(output.status.success(), "recall command should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("auth_service depends_on stripe_sdk"),
        "stdout should include matching claim, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
