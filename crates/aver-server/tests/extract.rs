use std::path::{Path, PathBuf};

use aver_core::{AgentKind, Provenance, Store};
use aver_server::tools::{AverTools, ExtractLimits, ExtractParams};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extract")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn params(path: &Path, scope: Option<&str>, dry_run: bool) -> ExtractParams {
    ExtractParams {
        path: path.to_string_lossy().into_owned(),
        scope: scope.map(str::to_string),
        lang: None,
        dry_run,
        agent_id: "aver-test-client".to_string(),
    }
}

fn open_tools(memory: &Path, root: Option<&Path>, max_files: usize) -> AverTools {
    AverTools::open(memory)
        .unwrap()
        .with_extract_limits(ExtractLimits {
            root: root.map(Path::to_path_buf),
            max_files,
        })
}

#[test]
fn extract_writes_extracted_claims_in_scope() {
    let memory = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    copy_tree(&fixtures(), root.path());
    let tools = open_tools(memory.path(), Some(root.path()), 100);
    let view = tools
        .extract(params(root.path(), Some("proj/demo"), false))
        .unwrap();
    drop(tools);

    assert_eq!(view.scope, "proj/demo");
    assert_eq!(view.files_scanned, 2);
    assert_eq!(view.files_failed, 0);
    assert!(view.facts_ingested > 0);
    assert_eq!(view.skipped["privacy"], 0);
    assert!(view.note.is_none());

    // Claim ids are writer-minted and sparse (ADR-0026), so enumerate them
    // from the projection instead of assuming 1..=n.
    let store = Store::open(memory.path()).unwrap();
    let conn = rusqlite::Connection::open(memory.path().join("db.sqlite")).unwrap();
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM claims ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids.len(), view.facts_ingested);
    for id in ids {
        let claim = store.get_claim(id).unwrap();
        assert_eq!(claim.scope, "proj/demo");
        assert_eq!(claim.provenance, Provenance::Extracted);
        assert_eq!(claim.confidence, 0.90);
        assert_eq!(claim.agent_kind, AgentKind::DeterministicParser);
        assert_eq!(claim.agent_id, "aver-test-client");
        assert!(!claim.source_refs.is_empty());
    }
}

#[test]
fn extract_refuses_paths_outside_root_and_without_root() {
    let memory = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    copy_tree(&fixtures(), outside.path());
    let tools = open_tools(memory.path(), Some(root.path()), 100);

    let err = tools
        .extract(params(outside.path(), Some("proj/demo"), false))
        .unwrap_err();
    assert!(
        err.to_string().contains("inside AVER_EXTRACT_ROOT"),
        "{err}"
    );

    let relative = ExtractParams {
        path: "src".to_string(),
        ..params(root.path(), None, true)
    };
    let err = tools.extract(relative).unwrap_err();
    assert!(err.to_string().contains("absolute"), "{err}");

    let dotdot = root
        .path()
        .join("..")
        .join(outside.path().file_name().unwrap());
    assert!(tools.extract(params(&dotdot, None, true)).is_err());
    drop(tools);

    let disabled = open_tools(memory.path(), None, 100);
    let err = disabled
        .extract(params(root.path(), None, true))
        .unwrap_err();
    assert!(err.to_string().contains("AVER_EXTRACT_ROOT"), "{err}");
}

#[cfg(unix)]
#[test]
fn extract_refuses_symlink_escaping_root() {
    let memory = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    copy_tree(&fixtures(), outside.path());
    let link = root.path().join("link");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let err = open_tools(memory.path(), Some(root.path()), 100)
        .extract(params(&link, None, true))
        .unwrap_err();
    assert!(
        err.to_string().contains("inside AVER_EXTRACT_ROOT"),
        "{err}"
    );
}

#[test]
fn extract_dry_run_writes_nothing_and_global_scope_is_noted() {
    let memory = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    copy_tree(&fixtures(), root.path());
    let tools = open_tools(memory.path(), Some(root.path()), 100);
    let view = tools
        .extract(params(root.path(), Some("global"), true))
        .unwrap();
    drop(tools);

    assert!(view.dry_run);
    assert!(view.facts_extracted > 0);
    assert_eq!(view.facts_ingested, 0);
    assert!(view.note.unwrap().contains("project scoping"));
    let store = Store::open(memory.path()).unwrap();
    assert!(store.get_claim(1).is_err());
}

#[test]
fn extract_enforces_file_cap() {
    let memory = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    copy_tree(&fixtures(), root.path());
    let tools = open_tools(memory.path(), Some(root.path()), 1);
    let err = tools.extract(params(root.path(), None, false)).unwrap_err();
    assert!(
        err.to_string().contains("AVER_EXTRACT_MAX_FILES is 1"),
        "{err}"
    );
    drop(tools);
    let store = Store::open(memory.path()).unwrap();
    assert!(store.get_claim(1).is_err());
}
