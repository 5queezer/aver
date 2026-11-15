use assert_cmd::prelude::*;
use std::fmt::Write as _;
use std::process::Command;

fn aver(memory_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd
}

#[test]
fn replay_strict_fails_and_lenient_quarantines() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path();

    let valid_line = |id: i64, subject: &str| {
        format!(
            r#"{{"kind":"add_claim","ts":1,"claim_id":{id},"subject":"{subject}","predicate":"depends_on","object":"o","source":"s","agent_id":"local","agent_kind":"HUMAN","confidence":0.95,"provenance":"USER_ASSERTED","scope":"global"}}"#
        )
    };
    let mut log = String::new();
    writeln!(log, "{}", valid_line(1, "good-one")).unwrap();
    log.push_str("{\"kind\":\"no_such_kind\",\"ts\":1}\n");
    writeln!(log, "{}", valid_line(2, "good-two")).unwrap();
    std::fs::write(memory.join("log.jsonl"), log).unwrap();

    // Default (strict) replay aborts on the unknown kind.
    aver(memory).arg("replay").assert().failure();

    // --lenient quarantines the bad line and rebuilds the rest.
    let output = aver(memory)
        .args(["replay", "--lenient"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("quarantined=1"),
        "expected quarantine count in output: {stdout}"
    );
    assert!(
        stdout.contains("claims=2"),
        "expected both valid claims applied: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("quarantined:") && stderr.contains("no_such_kind"),
        "expected quarantine diagnostics on stderr: {stderr}"
    );

    // The rebuilt store opens and serves the two surviving claims.
    aver(memory).arg("status").assert().success();
}
