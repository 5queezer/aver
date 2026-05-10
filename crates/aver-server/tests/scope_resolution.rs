//! ADR-0022 — connection-scope resolution: per-call param > X-Aver-Scope
//! header > X-Aver-Scope-Default header > AVER_DEFAULT_SCOPE env > 'global'.
//!
//! Tested at the unit level for the resolver function. End-to-end HTTP
//! header tests live alongside the http.rs middleware tests once the wiring
//! lands.

use aver_server::scope_resolution::{ResolvedScope, resolve_scope};

#[test]
fn explicit_param_scope_wins_over_header() {
    let resolved = resolve_scope(Some("proj/explicit"), Some("proj/header"), None, None).unwrap();
    assert_eq!(resolved.scope, "proj/explicit");
}

#[test]
fn header_wins_over_default_header() {
    let resolved = resolve_scope(None, Some("proj/header"), Some("proj/default"), None).unwrap();
    assert_eq!(resolved.scope, "proj/header");
}

#[test]
fn default_header_wins_over_env() {
    let resolved =
        resolve_scope(None, None, Some("proj/default-header"), Some("proj/env")).unwrap();
    assert_eq!(resolved.scope, "proj/default-header");
}

#[test]
fn env_used_when_no_param_or_headers() {
    let resolved = resolve_scope(None, None, None, Some("proj/env")).unwrap();
    assert_eq!(resolved.scope, "proj/env");
}

#[test]
fn falls_back_to_global_when_nothing_set() {
    let resolved = resolve_scope(None, None, None, None).unwrap();
    assert_eq!(resolved.scope, "global");
}

#[test]
fn rejects_malformed_header() {
    // ADR-0022 §"Header semantics": malformed headers fail fast (HTTP 400),
    // not silently fall back to 'global'.
    let err = resolve_scope(None, Some("bad space"), None, None)
        .expect_err("malformed header must reject");
    let _ = err;
}

#[test]
fn rejects_malformed_default_header() {
    let err = resolve_scope(None, None, Some("bad@chr"), None)
        .expect_err("malformed default header must reject");
    let _ = err;
}

#[test]
fn rejects_malformed_env() {
    let err =
        resolve_scope(None, None, None, Some("bad/space ")).expect_err("malformed env must reject");
    let _ = err;
}

#[test]
fn resolved_scope_marks_global_for_walk_default() {
    // ADR-0022 §"Read-path default flip": a global resolution defaults to
    // walk=any (preserves today's cross-cutting reads). A non-global
    // resolution defaults to walk=ancestors.
    let global = resolve_scope(None, None, None, None).unwrap();
    assert!(global.is_global());
    let projected = resolve_scope(Some("proj/aver"), None, None, None).unwrap();
    assert!(!projected.is_global());
}

#[test]
fn empty_string_header_is_treated_as_unset() {
    // An HTTP client that always sets `X-Aver-Scope:` (empty) shouldn't
    // override a more-specific source. Treat empty as missing.
    let resolved = resolve_scope(None, Some(""), None, Some("proj/env")).unwrap();
    assert_eq!(resolved.scope, "proj/env");
}

fn _assert_resolved_is_clone_send_sync<T: Clone + Send + Sync>() {}
fn _exercise() {
    _assert_resolved_is_clone_send_sync::<ResolvedScope>();
}
