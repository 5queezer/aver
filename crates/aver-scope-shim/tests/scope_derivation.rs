//! ADR-0022 — derive_scope decision tree.
//!
//! Closes council risk #3: sibling repositories with identical basenames
//! must produce distinct scopes when no origin is configured.

use std::process::Command;

use aver_scope_shim::{ScopeSource, derive_scope, scope_from_git, slug_hash};

fn git_init(dir: &std::path::Path) {
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .status()
        .expect("git init failed");
}

fn git_set_origin(dir: &std::path::Path, url: &str) {
    Command::new("git")
        .args(["config", "--local", "remote.origin.url", url])
        .current_dir(dir)
        .status()
        .expect("git config failed");
}

#[test]
fn cli_override_short_circuits() {
    let dir = tempfile::tempdir().unwrap();
    let derived = derive_scope(dir.path(), Some("proj/manual"), Some("proj/env"));
    assert_eq!(derived.scope, "proj/manual");
    assert_eq!(derived.source, ScopeSource::CliOverride);
}

#[test]
fn git_origin_drives_scope_when_present() {
    let dir = tempfile::tempdir().unwrap();
    git_init(dir.path());
    git_set_origin(dir.path(), "git@example.com:foo/bar.git");
    let derived = derive_scope(dir.path(), None, None);
    assert_eq!(derived.source, ScopeSource::GitOrigin);
    assert!(derived.scope.starts_with("proj/"));
    assert_eq!(
        derived.scope,
        format!("proj/{}", slug_hash("git@example.com:foo/bar.git"))
    );
}

#[test]
fn git_toplevel_path_drives_scope_when_origin_absent() {
    let dir = tempfile::tempdir().unwrap();
    git_init(dir.path());
    let derived = derive_scope(dir.path(), None, None);
    assert_eq!(derived.source, ScopeSource::GitToplevel);
    assert!(derived.scope.starts_with("proj/"));
}

#[test]
fn sibling_basenames_in_different_parents_produce_distinct_scopes() {
    // Council risk #3: `~/a/mcp` and `~/b/mcp` are different repos with the
    // same basename. The basename-fallback rejected by the council would
    // collide; the toplevel-abspath fallback must keep them separate.
    let parent = tempfile::tempdir().unwrap();
    let a = parent.path().join("a/mcp");
    let b = parent.path().join("b/mcp");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    git_init(&a);
    git_init(&b);
    let scope_a = derive_scope(&a, None, None);
    let scope_b = derive_scope(&b, None, None);
    assert_eq!(scope_a.source, ScopeSource::GitToplevel);
    assert_eq!(scope_b.source, ScopeSource::GitToplevel);
    assert_ne!(
        scope_a.scope, scope_b.scope,
        "sibling repos with same basename must produce distinct scopes"
    );
}

#[test]
fn env_default_used_outside_git() {
    let dir = tempfile::tempdir().unwrap();
    // `dir.path()` is a fresh temp dir that is not a git repo. (Linux mounts
    // a temp dir under /tmp which is not inside any git tree.)
    let derived = derive_scope(dir.path(), None, Some("proj/from-env"));
    // If the temp dir happened to be inside a git tree (rare but possible),
    // we'd get GitOrigin/GitToplevel. Guard the assertion accordingly.
    if scope_from_git(dir.path()).is_none() {
        assert_eq!(derived.source, ScopeSource::EnvDefault);
        assert_eq!(derived.scope, "proj/from-env");
    }
}

#[test]
fn hardcoded_global_used_when_no_other_source() {
    let dir = tempfile::tempdir().unwrap();
    let derived = derive_scope(dir.path(), None, None);
    if scope_from_git(dir.path()).is_none() {
        assert_eq!(derived.source, ScopeSource::HardcodedGlobal);
        assert_eq!(derived.scope, "global");
    }
}

#[test]
fn slug_hash_is_stable_and_short() {
    assert_eq!(slug_hash("hello"), slug_hash("hello"));
    assert_ne!(slug_hash("hello"), slug_hash("world"));
    assert_eq!(slug_hash("anything").len(), 12);
}
