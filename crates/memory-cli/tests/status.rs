//! T6 — minimal CLI status command opens the configured store and reports
//! readiness. This starts the v0.1 `memory status` surface from ADR-0013.

use std::process::Command;

#[test]
fn status_reports_store_ready_for_memory_dir() {
    let dir = std::env::temp_dir().join(format!("memory-cli-status-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let output = Command::new(env!("CARGO_BIN_EXE_memory"))
        .args(["--memory-dir", dir.to_str().unwrap(), "status"])
        .output()
        .expect("memory binary should run");

    assert!(output.status.success(), "status command should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("memory store: ok"),
        "stdout should report ready store, got {stdout:?}"
    );
    assert!(
        dir.join("db.sqlite").exists(),
        "status should open/create DB"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
