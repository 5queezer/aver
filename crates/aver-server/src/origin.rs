//! Browser-origin validation helper for ADR-0020.
//!
//! Slice 1 only exposes the helper; later slices mount it on
//! `/oauth/authorize` and the consent endpoints. The function is deliberately
//! permissive for non-browser clients (curl, MCP HTTP clients) which omit
//! both the `Origin` and `Sec-Fetch-Site` request headers.

use axum::http::{HeaderMap, header};
use url::Url;

/// Reasons a request was rejected by [`validate_browser_origin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginError {
    /// The browser told us this navigation came from a different site.
    CrossSiteFetch,
    /// `Origin` header value was syntactically invalid.
    InvalidOriginHeader,
    /// `Origin` did not match any of the configured base URLs.
    OriginNotAllowed,
}

impl std::fmt::Display for OriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OriginError::CrossSiteFetch => f.write_str("cross-site fetch rejected"),
            OriginError::InvalidOriginHeader => f.write_str("invalid Origin header"),
            OriginError::OriginNotAllowed => f.write_str("Origin not in allowed base URLs"),
        }
    }
}

impl std::error::Error for OriginError {}

/// Validates that a request looks like it came from one of `allowed_base_urls`
/// (or from a non-browser client that simply omits the relevant headers).
///
/// Rules:
/// - If `Sec-Fetch-Site: cross-site` is present, the request is rejected.
///   Other `Sec-Fetch-Site` values (`same-origin`, `same-site`, `none`) are
///   accepted.
/// - If an `Origin` header is present, it must parse and its
///   `(scheme, host, port)` must match at least one entry in
///   `allowed_base_urls`. `Origin: null` is treated as a mismatch.
/// - If neither header is present, the request is allowed (e.g. curl, the
///   MCP HTTP client).
pub fn validate_browser_origin(
    headers: &HeaderMap,
    allowed_base_urls: &[Url],
) -> Result<(), OriginError> {
    if let Some(value) = headers.get("sec-fetch-site")
        && let Ok(site) = value.to_str()
        && site.eq_ignore_ascii_case("cross-site")
    {
        return Err(OriginError::CrossSiteFetch);
    }

    let Some(origin_header) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin_str = origin_header
        .to_str()
        .map_err(|_| OriginError::InvalidOriginHeader)?;

    let origin_url = Url::parse(origin_str).map_err(|_| OriginError::InvalidOriginHeader)?;
    if allowed_base_urls
        .iter()
        .any(|base| origins_match(&origin_url, base))
    {
        Ok(())
    } else {
        Err(OriginError::OriginNotAllowed)
    }
}

fn origins_match(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn allowed() -> Vec<Url> {
        vec![Url::parse("http://127.0.0.1:3317").unwrap()]
    }

    #[test]
    fn allows_when_both_headers_absent() {
        let headers = HeaderMap::new();
        assert_eq!(validate_browser_origin(&headers, &allowed()), Ok(()));
    }

    #[test]
    fn allows_same_origin_sec_fetch_site() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert_eq!(validate_browser_origin(&headers, &allowed()), Ok(()));
    }

    #[test]
    fn allows_same_site_and_none_sec_fetch_site() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));
        assert_eq!(validate_browser_origin(&headers, &allowed()), Ok(()));

        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("none"));
        assert_eq!(validate_browser_origin(&headers, &allowed()), Ok(()));
    }

    #[test]
    fn rejects_cross_site_sec_fetch_site() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::CrossSiteFetch),
        );
    }

    #[test]
    fn rejects_cross_site_even_with_matching_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:3317"),
        );
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::CrossSiteFetch),
        );
    }

    #[test]
    fn allows_matching_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:3317"),
        );
        assert_eq!(validate_browser_origin(&headers, &allowed()), Ok(()));
    }

    #[test]
    fn rejects_mismatched_origin_host() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example:3317"),
        );
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::OriginNotAllowed),
        );
    }

    #[test]
    fn rejects_mismatched_origin_port() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:9999"),
        );
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::OriginNotAllowed),
        );
    }

    #[test]
    fn rejects_mismatched_origin_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://127.0.0.1:3317"),
        );
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::OriginNotAllowed),
        );
    }

    #[test]
    fn rejects_invalid_origin_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("not-a-url"));
        assert_eq!(
            validate_browser_origin(&headers, &allowed()),
            Err(OriginError::InvalidOriginHeader),
        );
    }

    #[test]
    fn rejects_null_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(validate_browser_origin(&headers, &allowed()).is_err());
    }

    #[test]
    fn matches_origin_against_multiple_allowed_bases() {
        let bases = vec![
            Url::parse("http://127.0.0.1:3317").unwrap(),
            Url::parse("https://aver.local").unwrap(),
        ];
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://aver.local"),
        );
        assert_eq!(validate_browser_origin(&headers, &bases), Ok(()));
    }
}
