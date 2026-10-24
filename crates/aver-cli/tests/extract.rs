//! Roadmap Phase 3 exit test: `aver extract` turns a source tree into
//! EXTRACTED claims in the project scope that `aver recall` returns.

use assert_cmd::prelude::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCOPE: &str = "proj/fixture";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extract")
}

fn aver(memory_dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("aver").unwrap();
    cmd.arg("--memory-dir").arg(memory_dir);
    cmd.env_remove("AVER_DECISION_MODEL");
    cmd
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "aver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Run `aver extract .` from the fixture root so source paths are relative.
fn extract(memory_dir: &Path, args: &[&str]) -> String {
    stdout(
        aver(memory_dir)
            .current_dir(fixture())
            .args(["extract", "."])
            .args(args)
            .output()
            .unwrap(),
    )
}

fn recall(memory_dir: &Path, query: &str, args: &[&str]) -> String {
    stdout(
        aver(memory_dir)
            .args(["recall", query])
            .args(args)
            .output()
            .unwrap(),
    )
}

#[test]
fn extract_then_recall_returns_project_scoped_code_claims() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path().join("mem");

    let summary = extract(&memory, &["--scope", SCOPE]);
    assert!(
        summary.contains("extract: scope=proj/fixture files_scanned=2 files_failed=0"),
        "{summary}"
    );
    // Python `CsvReport(Report)` is an `extends` fact; core has no such
    // predicate, so it is counted, not written.
    assert!(summary.contains("skipped=extends:1,"), "{summary}");
    let ingested: i64 = summary
        .split("facts_ingested=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(ingested > 0, "{summary}");

    // Every written claim is still ACTIVE, EXTRACTED at 0.90, in the project
    // scope: extract does not consolidate (see `ingest_extracted`).
    let conn = rusqlite::Connection::open(memory.join("db.sqlite")).unwrap();
    let (active, total): (i64, i64) = conn
        .query_row(
            "SELECT SUM(status = 'ACTIVE' AND provenance = 'EXTRACTED'
                        AND confidence = 0.90 AND scope = ?1),
                    COUNT(*)
               FROM claims",
            [SCOPE],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((active, total), (ingested, ingested));

    let store = recall(&memory, "Store", &["--scope", SCOPE]);
    for line in [
        "[proj/fixture] File:src/store.rs defines Class:Store",
        "[proj/fixture] Class:Store defines Function:Store::open",
        "[proj/fixture] Function:Store::open calls Function:validate",
    ] {
        assert!(store.contains(line), "missing {line:?} in:\n{store}");
    }

    // Several `defines` from one file all stay recallable; consolidation
    // would retire all but the newest (see `ingest_extracted`).
    let file = recall(&memory, "src/store.rs", &["--scope", SCOPE]);
    assert!(
        file.contains("File:src/store.rs defines Function:validate"),
        "{file}"
    );
    assert!(
        file.contains("File:src/store.rs defines Class:Store"),
        "{file}"
    );

    let python = recall(&memory, "Report", &["--scope", SCOPE]);
    assert!(
        python.contains("[proj/fixture] File:tools/report.py defines Class:Report"),
        "{python}"
    );

    // The claims live in the project scope only.
    let other = recall(
        &memory,
        "Store",
        &["--scope", "proj/other", "--scope-walk", "exact"],
    );
    assert!(other.is_empty(), "{other}");
    let global = recall(
        &memory,
        "Store",
        &["--scope", "global", "--scope-walk", "exact"],
    );
    assert!(global.is_empty(), "{global}");
}

#[test]
fn dry_run_counts_per_predicate_without_touching_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path().join("mem");

    let output = extract(&memory, &["--scope", SCOPE, "--dry-run"]);
    assert!(output.contains("\ndefines="), "{output}");
    assert!(output.contains("\nimports=1\n"), "{output}");
    assert!(
        output.contains("extract (dry run): scope=proj/fixture"),
        "{output}"
    );
    assert!(output.contains("facts_ingested=0"), "{output}");
    assert!(!memory.exists(), "dry run must not create the store");
}

#[test]
fn lang_filter_limits_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    let output = extract(
        dir.path(),
        &["--scope", SCOPE, "--dry-run", "--lang", "python"],
    );
    assert!(output.contains("files_scanned=1 "), "{output}");
    assert!(!output.contains("imports="), "{output}");

    let bad = aver(dir.path())
        .current_dir(fixture())
        .args(["extract", ".", "--lang", "cobol"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
}

#[test]
fn default_scope_is_derived_from_the_project_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo.project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::copy(fixture().join("src/store.rs"), project.join("src/store.rs")).unwrap();

    // Stop git from finding a repository above the temp dir, so the slug
    // falls back to the directory name.
    let output = stdout(
        aver(&dir.path().join("mem"))
            .env("GIT_CEILING_DIRECTORIES", dir.path())
            .args(["extract", "--dry-run"])
            .arg(&project)
            .output()
            .unwrap(),
    );
    assert!(output.contains("scope=proj/demo-project "), "{output}");
}
