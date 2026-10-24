//! ADR-0020 slice 3: OAuth scope catalog. Each MCP tool declares the scope
//! it requires next to its handler in `mcp.rs` (see [`ToolScope`]).
//!
//! Six scopes mirror the current MCP tool groups. The canonical string form
//! is what clients send in `scope=` and what discovery advertises under
//! `scopes_supported`. The order in [`SUPPORTED`] is the deterministic
//! rendering order used by [`serialize_scope_list`].

use std::fmt;
use std::str::FromStr;

/// OAuth scopes recognised by Aver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    ClaimsRead,
    ClaimsWrite,
    EventsWrite,
    CandidatesManage,
    ObservationsRead,
    ObservationsWrite,
}

/// All scopes Aver advertises and accepts, in canonical order. The order is
/// the source of truth for both `scopes_supported` and
/// [`serialize_scope_list`] output.
pub const SUPPORTED: &[Scope] = &[
    Scope::ClaimsRead,
    Scope::ClaimsWrite,
    Scope::EventsWrite,
    Scope::CandidatesManage,
    Scope::ObservationsRead,
    Scope::ObservationsWrite,
];

impl Scope {
    /// Canonical wire string for this scope (`claims:read`, etc.).
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::ClaimsRead => "claims:read",
            Scope::ClaimsWrite => "claims:write",
            Scope::EventsWrite => "events:write",
            Scope::CandidatesManage => "candidates:manage",
            Scope::ObservationsRead => "observations:read",
            Scope::ObservationsWrite => "observations:write",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by [`Scope::from_str`] / [`parse_scope_list`] for an
/// unrecognised scope string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeParseError {
    pub raw: String,
}

impl fmt::Display for ScopeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown scope: {:?}", self.raw)
    }
}

impl std::error::Error for ScopeParseError {}

impl FromStr for Scope {
    type Err = ScopeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claims:read" => Ok(Scope::ClaimsRead),
            "claims:write" => Ok(Scope::ClaimsWrite),
            "events:write" => Ok(Scope::EventsWrite),
            "candidates:manage" => Ok(Scope::CandidatesManage),
            "observations:read" => Ok(Scope::ObservationsRead),
            "observations:write" => Ok(Scope::ObservationsWrite),
            other => Err(ScopeParseError {
                raw: other.to_string(),
            }),
        }
    }
}

/// Splits an OAuth `scope=` parameter (whitespace-separated) into [`Scope`]s.
///
/// Empty / whitespace-only input yields an empty vec. Any unknown token is
/// reported as [`ScopeParseError`]; valid tokens to the left are not
/// returned (callers should treat the whole list as malformed).
pub fn parse_scope_list(s: &str) -> Result<Vec<Scope>, ScopeParseError> {
    let mut out = Vec::new();
    for raw in s.split_ascii_whitespace() {
        out.push(Scope::from_str(raw)?);
    }
    Ok(out)
}

/// Permissively splits a scope list, dropping unknown tokens silently.
///
/// Used when reading a token row's persisted `granted_scopes` column —
/// per the ADR-0020 brief, an unknown token must not poison the row; valid
/// scopes still apply. Returns scopes in input order.
pub fn parse_scope_list_lossy(s: &str) -> Vec<Scope> {
    s.split_ascii_whitespace()
        .filter_map(|raw| Scope::from_str(raw).ok())
        .collect()
}

/// Encodes a list of scopes as a deterministic, space-separated string.
///
/// Output ordering matches [`SUPPORTED`]; duplicates are collapsed.
pub fn serialize_scope_list(scopes: &[Scope]) -> String {
    let mut out = String::new();
    for s in SUPPORTED {
        if scopes.iter().any(|x| x == s) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(s.as_str());
        }
    }
    out
}

/// The OAuth scope an MCP tool requires. Each handler in
/// [`crate::mcp::AverMcpService`] declares one as a constant next to its
/// `#[tool]` attribute; [`crate::mcp::AverMcpService::TOOLS`] lists them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolScope {
    pub name: &'static str,
    pub scope: Scope,
}

impl ToolScope {
    pub const fn new(name: &'static str, scope: Scope) -> Self {
        Self { name, scope }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_lists_all_six_in_canonical_order() {
        let serialized: Vec<&str> = SUPPORTED.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            serialized,
            vec![
                "claims:read",
                "claims:write",
                "events:write",
                "candidates:manage",
                "observations:read",
                "observations:write",
            ],
        );
    }

    #[test]
    fn parse_round_trips_through_serialize() {
        let raw = "claims:read claims:write events:write candidates:manage observations:read observations:write";
        let parsed = parse_scope_list(raw).unwrap();
        assert_eq!(parsed.len(), 6);
        assert_eq!(serialize_scope_list(&parsed), raw);
    }

    #[test]
    fn serialize_is_deterministic_regardless_of_input_order() {
        let unordered = vec![
            Scope::ObservationsWrite,
            Scope::ClaimsRead,
            Scope::EventsWrite,
        ];
        assert_eq!(
            serialize_scope_list(&unordered),
            "claims:read events:write observations:write",
        );
    }

    #[test]
    fn serialize_collapses_duplicates() {
        let dup = vec![Scope::ClaimsRead, Scope::ClaimsRead, Scope::ClaimsWrite];
        assert_eq!(serialize_scope_list(&dup), "claims:read claims:write");
    }

    #[test]
    fn parse_empty_string_yields_empty_vec() {
        assert!(parse_scope_list("").unwrap().is_empty());
        assert!(parse_scope_list("   \t\n").unwrap().is_empty());
    }

    #[test]
    fn parse_unknown_scope_returns_error() {
        let err = parse_scope_list("claims:read totally:made-up").unwrap_err();
        assert_eq!(err.raw, "totally:made-up");
    }

    #[test]
    fn parse_lossy_drops_unknown_tokens() {
        let parsed = parse_scope_list_lossy("claims:read junk events:write");
        assert_eq!(parsed, vec![Scope::ClaimsRead, Scope::EventsWrite]);
    }

    #[test]
    fn parse_lossy_with_only_unknown_yields_empty() {
        assert!(parse_scope_list_lossy("nope nada").is_empty());
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert!("claims:purple".parse::<Scope>().is_err());
    }
}
