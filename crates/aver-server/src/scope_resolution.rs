//! ADR-0022 — connection-scope resolution.
//!
//! Layer 2 of the scope work. Layer 1 (ADR-0021) added the `scope` column
//! and per-call MCP parameter. This module implements the precedence chain
//! that decides what scope a request operates under when the per-call
//! parameter is absent:
//!
//! ```text
//! 1. Tool parameter `scope` (ADR-0021)        — explicit per-call override
//! 2. HTTP header `X-Aver-Scope`                — per-connection identity
//! 3. HTTP header `X-Aver-Scope-Default`        — cautious fallback
//! 4. Server config `AVER_DEFAULT_SCOPE` env    — host-wide fallback
//! 5. Hardcoded default `global`                — ADR-0021 baseline
//! ```
//!
//! Validation: every level except the hardcoded default is checked against
//! the same charset the SQL trigger enforces (`[A-Za-z0-9_/-]`, non-blank).
//! Malformed sources fail fast — no silent fallback to `global`, since that
//! would re-introduce the cross-repo pollution this ADR exists to fix.

use anyhow::Context;
use aver_core::ScopeWalk;

/// Resolved scope for the current request, plus the default walk mode that
/// follows from it (ADR-0022 §"Read-path default flip").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedScope {
    pub scope: String,
    /// Walk mode used when the caller does not specify `scope_walk` explicitly.
    pub default_walk: ScopeWalk,
}

impl ResolvedScope {
    pub fn is_global(&self) -> bool {
        self.scope == "global"
    }
}

/// Validate a scope candidate without going through sqlite. Mirrors the
/// `[A-Za-z0-9_/-]` charset enforced by migration 0084.
fn validate(scope: &str, source: &'static str) -> anyhow::Result<()> {
    if scope.trim().is_empty() {
        anyhow::bail!("{source}: scope must not be blank");
    }
    let ok = scope
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'/');
    if !ok {
        anyhow::bail!(
            "{source}: scope {scope:?} contains invalid characters; allowed [A-Za-z0-9_/-]"
        );
    }
    Ok(())
}

/// Apply the precedence chain. Each argument is the raw value at that level
/// (`None` if unset). Empty strings are treated as unset for headers and env
/// to keep clients that always include the header but sometimes leave it
/// blank from accidentally overriding more-specific sources.
pub fn resolve_scope(
    param_scope: Option<&str>,
    header_scope: Option<&str>,
    header_default: Option<&str>,
    env_default: Option<&str>,
) -> anyhow::Result<ResolvedScope> {
    let pick = || -> anyhow::Result<String> {
        if let Some(s) = param_scope.filter(|v| !v.trim().is_empty()) {
            validate(s, "param scope").context("invalid tool-param scope")?;
            return Ok(s.to_string());
        }
        if let Some(s) = header_scope.filter(|v| !v.trim().is_empty()) {
            validate(s, "X-Aver-Scope").context("invalid X-Aver-Scope header")?;
            return Ok(s.to_string());
        }
        if let Some(s) = header_default.filter(|v| !v.trim().is_empty()) {
            validate(s, "X-Aver-Scope-Default").context("invalid X-Aver-Scope-Default header")?;
            return Ok(s.to_string());
        }
        if let Some(s) = env_default.filter(|v| !v.trim().is_empty()) {
            validate(s, "AVER_DEFAULT_SCOPE").context("invalid AVER_DEFAULT_SCOPE env")?;
            return Ok(s.to_string());
        }
        Ok("global".to_string())
    };
    let scope = pick()?;
    let default_walk = if scope == "global" {
        ScopeWalk::Any
    } else {
        ScopeWalk::Ancestors
    };
    Ok(ResolvedScope {
        scope,
        default_walk,
    })
}
