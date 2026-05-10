//! ADR-0022 reference shim: derive a scope from the launching process's git
//! working directory and forward MCP HTTP requests to an upstream aver-server
//! with `X-Aver-Scope` injected.
//!
//! Lifecycle is per-workspace: each Claude Code (or other harness) workspace
//! launches its own shim, which binds to an ephemeral 127.0.0.1 port and
//! forwards to the shared upstream server. The harness only needs to learn
//! the shim's URL — the rest of the rewrite is invisible.
//!
//! Layout:
//! - [`derive_scope`] / [`scope_from_git`] — slug derivation rules.
//! - The HTTP proxy lives in `main.rs`; this module exports the building
//!   blocks for unit tests.

use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

/// Per-startup scope decision: either a derived `proj/...` scope, an env-var
/// override, or the hardcoded "global" fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedScope {
    pub scope: String,
    pub source: ScopeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeSource {
    /// `--scope` CLI flag explicitly supplied.
    CliOverride,
    /// `git config remote.origin.url` hashed.
    GitOrigin,
    /// `git rev-parse --show-toplevel` absolute path hashed (no origin).
    GitToplevel,
    /// `AVER_DEFAULT_SCOPE` environment variable.
    EnvDefault,
    /// Hardcoded fallback when no other source applies.
    HardcodedGlobal,
}

/// Hash to first 12 hex chars of SHA-256.
pub fn slug_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let bytes = hasher.finalize();
    bytes
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Derive the scope a shim should inject for a given working directory.
///
/// Precedence:
/// 1. `cli_override` if supplied.
/// 2. Git remote origin URL hash, if `cwd` is inside a git repo with origin.
/// 3. Git toplevel absolute path hash (per ADR-0022 amendment / council
///    risk #3), if inside a git repo without origin.
/// 4. `AVER_DEFAULT_SCOPE` environment variable.
/// 5. `"global"`.
pub fn derive_scope(
    cwd: &Path,
    cli_override: Option<&str>,
    env_default: Option<&str>,
) -> DerivedScope {
    if let Some(s) = cli_override {
        return DerivedScope {
            scope: s.to_string(),
            source: ScopeSource::CliOverride,
        };
    }
    if let Some(scope) = scope_from_git(cwd) {
        return scope;
    }
    if let Some(env) = env_default.filter(|v| !v.trim().is_empty()) {
        return DerivedScope {
            scope: env.to_string(),
            source: ScopeSource::EnvDefault,
        };
    }
    DerivedScope {
        scope: "global".to_string(),
        source: ScopeSource::HardcodedGlobal,
    }
}

/// Try to derive a `proj/<slug>` scope from a git working directory.
/// Returns `None` if `cwd` is not inside a git worktree.
pub fn scope_from_git(cwd: &Path) -> Option<DerivedScope> {
    let toplevel = run_git(cwd, &["rev-parse", "--show-toplevel"])?;
    let toplevel = toplevel.trim();
    if toplevel.is_empty() {
        return None;
    }
    if let Some(origin) = run_git(cwd, &["config", "--get", "remote.origin.url"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(DerivedScope {
            scope: format!("proj/{}", slug_hash(&origin)),
            source: ScopeSource::GitOrigin,
        });
    }
    // Per ADR-0022 amendment / council risk #3: hash the absolute toplevel
    // path (NOT the basename). Sibling repos with the same dir name produce
    // distinct scopes.
    let abs = match Path::new(toplevel).canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => toplevel.to_string(),
    };
    Some(DerivedScope {
        scope: format!("proj/{}", slug_hash(&abs)),
        source: ScopeSource::GitToplevel,
    })
}

fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}
