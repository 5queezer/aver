//! ADR-0021 — `--scope` flag round-trip on the aver CLI.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::tempdir;

fn aver(memory_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd
}

#[test]
fn remember_with_scope_flag_persists_explicit_scope() {
    let dir = tempdir().unwrap();
    aver(dir.path())
        .args(["remember", "user", "prefers", "tabs"])
        .args(["--source", "cli-test"])
        .args(["--scope", "proj/aver"])
        .assert()
        .success();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM claims ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope, "proj/aver");
}

#[test]
fn remember_without_scope_flag_defaults_to_global() {
    let dir = tempdir().unwrap();
    aver(dir.path())
        .args(["remember", "user", "prefers", "tabs"])
        .args(["--source", "cli-test"])
        .assert()
        .success();

    let conn = Connection::open(dir.path().join("db.sqlite")).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT scope FROM claims ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope, "global");
}

#[test]
fn recall_with_scope_walk_filters_results() {
    let dir = tempdir().unwrap();
    aver(dir.path())
        .args(["remember", "aver", "uses", "rusqlite"])
        .args(["--source", "t"])
        .args(["--scope", "proj/aver"])
        .assert()
        .success();
    aver(dir.path())
        .args(["remember", "aver", "uses", "elsewhere"])
        .args(["--source", "t"])
        .args(["--scope", "proj/other"])
        .assert()
        .success();

    let output = aver(dir.path())
        .args(["recall", "uses"])
        .args(["--scope", "proj/aver"])
        .args(["--scope-walk", "exact"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("[proj/aver]"));
    assert!(!stdout.contains("[proj/other]"), "stdout: {stdout}");
}
