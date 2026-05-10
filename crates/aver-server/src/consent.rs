//! Browser consent flow for ADR-0020 slice 2 (Profile A — loopback).
//!
//! This module renders the HTML consent screen at `GET /oauth/authorize` and
//! handles the form submission at `POST /oauth/authorize/decision`. It is
//! intentionally narrow: only loopback (Profile A) is implemented here.
//! Non-loopback callers fall through to the legacy `approval_token` gate
//! which is still wired up in `http.rs`; that gate is removed in slice 6.
//!
//! Anti-CSRF design:
//! The anti-CSRF token is `HMAC-SHA256(server_secret, session_id || "|" ||
//! client_id || "|" || code_challenge)` encoded as URL-safe base64 without
//! padding. The 32-byte `server_secret` is generated lazily and persisted in
//! the `server_secrets` table so it survives restarts (a fresh secret would
//! invalidate every in-flight consent screen). The token is therefore stable
//! across the GET that renders the form and the POST that submits it without
//! any per-request server state — the cookie's session id is the only piece
//! of mutable input we need to bind. We chose this over storing a per-session
//! nonce row because it requires no schema and no cleanup.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use url::Url;

use crate::auth::{AuthDb, Session, User, UserKind};
use crate::origin::validate_browser_origin;

/// Cookie name for the Aver browser session.
pub const SESSION_COOKIE_NAME: &str = "aver_session";

/// Session lifetime (24h). ADR-0020 §Profile A.
pub const SESSION_TTL_SECS: i64 = 24 * 60 * 60;

/// Fixed identifier for the Profile A loopback user.
pub const LOCAL_USER_ID: &str = "local";

/// Name under which the anti-CSRF HMAC key is persisted in `server_secrets`.
pub const CSRF_SECRET_NAME: &str = "csrf_hmac";

type HmacSha256 = Hmac<Sha256>;

/// Returns `Some(local_user)` only when the request originates from a
/// loopback IP. Upserts the fixed `local` user on the first call.
///
/// `headers` is currently unused — kept in the signature to mirror the
/// brief and to leave room for future profiles that read headers (e.g.
/// trust headers or Authorization).
pub fn authenticate_loopback(
    remote_addr: SocketAddr,
    _headers: &HeaderMap,
    auth_db: &AuthDb,
) -> Option<User> {
    if !remote_addr.ip().is_loopback() {
        return None;
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let user = User {
        id: LOCAL_USER_ID.to_string(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now,
    };
    if let Err(err) = auth_db.upsert_user(&user) {
        tracing_unavailable_warn(&format!("upsert local user failed: {err}"));
        return None;
    }
    // Re-read so we return the original `created_at` on subsequent calls.
    auth_db.get_user(LOCAL_USER_ID).ok().flatten()
}

/// Logging stub: aver-server does not yet pull in `tracing`. Keeps the call
/// site honest about the failure without panicking; in practice the upsert
/// path is exercised in tests so silent failure is acceptable.
fn tracing_unavailable_warn(_msg: &str) {}

/// Reads the session cookie from `headers` and returns the bound user if the
/// session is still valid.
pub fn current_session(headers: &HeaderMap, auth_db: &AuthDb) -> Option<(Session, User)> {
    let cookie_value = read_cookie(headers, SESSION_COOKIE_NAME)?;
    let session = auth_db.get_session(&cookie_value).ok().flatten()?;
    let user = auth_db.get_user(&session.user_id).ok().flatten()?;
    Some((session, user))
}

/// Parses a `Cookie:` header and returns the named cookie value, if any.
fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=')
            && k == name
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Builds the `Set-Cookie` header value for the session id.
/// Loopback HTTP — no `Secure`. `HttpOnly`, `SameSite=Lax`, `Path=/`.
fn session_cookie_header(session_id: &str, ttl_secs: i64) -> String {
    format!(
        "{name}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={ttl}",
        name = SESSION_COOKIE_NAME,
        value = session_id,
        ttl = ttl_secs,
    )
}

/// Ensures a session exists for `user_id`. Returns the session and whether
/// it was newly created (so the caller knows to emit `Set-Cookie`).
fn ensure_session(
    auth_db: &AuthDb,
    headers: &HeaderMap,
    user_id: &str,
) -> anyhow::Result<(Session, bool)> {
    if let Some((session, user)) = current_session(headers, auth_db)
        && user.id == user_id
    {
        return Ok((session, false));
    }
    let session = auth_db.create_session(user_id, SESSION_TTL_SECS)?;
    Ok((session, true))
}

/// Computes the anti-CSRF token bound to `(session, client, code_challenge)`.
pub fn compute_csrf_token(
    server_secret: &[u8],
    session_id: &str,
    client_id: &str,
    code_challenge: &str,
) -> String {
    let mut mac = HmacSha256::new_from_slice(server_secret).expect("HMAC accepts any key length");
    mac.update(session_id.as_bytes());
    mac.update(b"|");
    mac.update(client_id.as_bytes());
    mac.update(b"|");
    mac.update(code_challenge.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Constant-time-ish equality. `hmac::Mac::verify` cannot be used directly
/// because we already encoded the token; comparing equal-length base64 is
/// fine for this surface and avoids re-deriving the raw bytes.
pub fn verify_csrf_token(
    server_secret: &[u8],
    session_id: &str,
    client_id: &str,
    code_challenge: &str,
    presented: &str,
) -> bool {
    let expected = compute_csrf_token(server_secret, session_id, client_id, code_challenge);
    constant_time_eq(expected.as_bytes(), presented.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// HTML escape: `& < > " '` only (no full Unicode entities). Sufficient for
/// the small set of attribute and text contexts we render.
pub fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

/// True iff the consent record covers every requested scope (treating an
/// empty `requested` set as the implicit "default access" scope).
pub fn consent_covers(granted: &[String], requested: &[String]) -> bool {
    requested.iter().all(|r| granted.iter().any(|g| g == r))
}

/// Standard set of security headers for the consent surface.
fn consent_security_headers() -> [(header::HeaderName, HeaderValue); 4] {
    [
        (
            header::HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; style-src 'unsafe-inline'"),
        ),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ),
    ]
}

/// Renders a self-contained HTML response with the consent security headers.
fn html_response(status: StatusCode, body: String, set_cookie: Option<String>) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    for (k, v) in consent_security_headers() {
        headers.insert(k, v);
    }
    if let Some(cookie) = set_cookie
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        headers.insert(header::SET_COOKIE, value);
    }
    response
}

fn html_error(status: StatusCode, title: &str, detail: &str) -> Response {
    let body = format!(
        concat!(
            "<!doctype html><html><head><meta charset=\"utf-8\">",
            "<title>{title}</title>",
            "<style>body{{font-family:system-ui,sans-serif;max-width:480px;",
            "margin:4em auto;padding:1em;color:#111}}h1{{font-size:1.2em}}",
            "p{{color:#444}}</style></head><body><h1>{title}</h1>",
            "<p>{detail}</p></body></html>",
        ),
        title = html_escape(title),
        detail = html_escape(detail),
    );
    html_response(status, body, None)
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

fn parse_scope(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split_ascii_whitespace()
        .map(str::to_string)
        .collect()
}

fn append_query_pair(url: &mut Url, key: &str, value: &str) {
    url.query_pairs_mut().append_pair(key, value);
}

/// Renders the consent screen body. Caller wraps in `html_response`.
#[allow(clippy::too_many_arguments)]
fn render_consent_page(
    client_id: &str,
    client_name: &str,
    redirect_uri: &str,
    client_registered_at: i64,
    scopes: &[String],
    state: Option<&str>,
    code_challenge: &str,
    code_challenge_method: &str,
    raw_scope: Option<&str>,
    csrf_token: &str,
) -> String {
    let scope_html = if scopes.is_empty() {
        "<li>Default access</li>".to_string()
    } else {
        scopes
            .iter()
            .map(|s| format!("<li><code>{}</code></li>", html_escape(s)))
            .collect::<String>()
    };
    let state_input = match state {
        Some(s) => format!(
            "<input type=\"hidden\" name=\"state\" value=\"{}\">",
            html_escape(s)
        ),
        None => String::new(),
    };
    let scope_input = match raw_scope {
        Some(s) => format!(
            "<input type=\"hidden\" name=\"scope\" value=\"{}\">",
            html_escape(s)
        ),
        None => String::new(),
    };
    format!(
        concat!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
            "<title>Authorize client — Aver</title>",
            "<style>body{{font-family:system-ui,sans-serif;max-width:560px;",
            "margin:3em auto;padding:1.5em;color:#111}}",
            "h1{{font-size:1.3em}}dl{{margin:1em 0}}dt{{font-weight:600;",
            "margin-top:.5em}}dd{{margin:0 0 .25em 0;font-family:monospace;",
            "font-size:.9em;word-break:break-all}}ul{{margin:.5em 0}}",
            ".actions{{margin-top:1.5em}}button{{font-size:1em;padding:.5em 1em;",
            "margin-right:.5em}}label{{display:block;margin-top:1em}}",
            ".note{{color:#666;font-size:.9em;margin-top:1em}}</style>",
            "</head><body>",
            "<h1>Allow this client to access your Aver memory?</h1>",
            "<dl>",
            "<dt>Client name</dt><dd>{client_name}</dd>",
            "<dt>Client ID</dt><dd>{client_id}</dd>",
            "<dt>Redirect URI</dt><dd>{redirect_uri}</dd>",
            "<dt>Registered at (unix)</dt><dd>{registered_at}</dd>",
            "</dl>",
            "<p>Requested access:</p><ul>{scope_html}</ul>",
            "<form method=\"POST\" action=\"/oauth/authorize/decision\">",
            "<input type=\"hidden\" name=\"client_id\" value=\"{client_id_attr}\">",
            "<input type=\"hidden\" name=\"redirect_uri\" value=\"{redirect_uri_attr}\">",
            "<input type=\"hidden\" name=\"code_challenge\" value=\"{code_challenge}\">",
            "<input type=\"hidden\" name=\"code_challenge_method\" value=\"{code_challenge_method}\">",
            "{scope_input}",
            "{state_input}",
            "<input type=\"hidden\" name=\"csrf_token\" value=\"{csrf}\">",
            "<label><input type=\"checkbox\" name=\"remember\" value=\"1\"> ",
            "Remember this client</label>",
            "<div class=\"actions\">",
            "<button type=\"submit\" name=\"decision\" value=\"approve\">Approve</button>",
            "<button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button>",
            "</div></form>",
            "<p class=\"note\">Dynamic-registered MCP clients have no curated ",
            "name or logo. Approve only if you initiated this flow.</p>",
            "</body></html>",
        ),
        client_name = html_escape(client_name),
        client_id = html_escape(client_id),
        client_id_attr = html_escape(client_id),
        redirect_uri = html_escape(redirect_uri),
        redirect_uri_attr = html_escape(redirect_uri),
        registered_at = client_registered_at,
        scope_html = scope_html,
        state_input = state_input,
        scope_input = scope_input,
        code_challenge = html_escape(code_challenge),
        code_challenge_method = html_escape(code_challenge_method),
        csrf = html_escape(csrf_token),
    )
}

/// Shared state injected by `http.rs`. We re-declare a thin trait bound here
/// to avoid a circular module dependency: `http.rs` passes any `Arc<Mutex<AuthDb>>`
/// plus the configured base URL.
pub struct ConsentDeps {
    pub auth_db: Arc<Mutex<AuthDb>>,
    pub base_url: String,
}

/// Looks up the registered client's `(client_name, redirect_uris, created_at)`
/// from the `oauth_clients` table. Returns `None` if the row is missing.
fn lookup_client_meta(
    auth_db: &AuthDb,
    client_id: &str,
) -> anyhow::Result<Option<(String, Vec<String>, i64)>> {
    // We tunnel through a fresh prepared statement on the borrowed connection
    // by going via the public AuthDb surface and a small extra query for the
    // timestamp. Slice 1 already exposes `get_client` for name+uris; we use a
    // raw query for `created_at` to keep this slice's auth-db API additive.
    let Some(client) = auth_db.get_client(client_id)? else {
        return Ok(None);
    };
    let created_at = auth_db.client_created_at(client_id)?.unwrap_or(0);
    Ok(Some((client.client_name, client.redirect_uris, created_at)))
}

/// Loopback branch of `GET /oauth/authorize`. Returns `Ok(response)` when
/// this branch handled the request (consent screen, immediate code mint, or
/// HTML error). Returns `Err(query)` to signal "fall through to the legacy
/// approval_token handler" — the caller will resume the existing flow.
pub async fn handle_loopback_get_authorize(
    State(deps): State<Arc<ConsentDeps>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    Query(query): Query<AuthorizeQuery>,
    headers: HeaderMap,
) -> Response {
    // Validate the easy params first; surface as HTML errors not JSON.
    if query.response_type != "code" {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Unsupported response_type",
            "Aver only implements response_type=code.",
        );
    }
    if query.code_challenge_method != "S256" {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Unsupported code_challenge_method",
            "PKCE S256 is required.",
        );
    }
    if query.code_challenge.is_empty() {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Missing code_challenge",
            "code_challenge is required.",
        );
    }

    let allowed = match parse_allowed_origins(&deps.base_url) {
        Ok(v) => v,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server misconfiguration",
                "AVER_BASE_URL is not a valid URL.",
            );
        }
    };
    if let Err(err) = validate_browser_origin(&headers, &allowed) {
        return html_error(
            StatusCode::FORBIDDEN,
            "Cross-site request rejected",
            &err.to_string(),
        );
    }

    let auth_db_guard = match deps.auth_db.lock() {
        Ok(g) => g,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Auth database lock poisoned.",
            );
        }
    };
    let auth_db: &AuthDb = &auth_db_guard;

    let user = match authenticate_loopback(remote_addr, &headers, auth_db) {
        Some(u) => u,
        None => {
            // Should not happen — caller already checked. Defensive HTML 403.
            return html_error(
                StatusCode::FORBIDDEN,
                "Loopback only",
                "This endpoint requires a loopback connection.",
            );
        }
    };

    let (client_name, redirect_uris, registered_at) =
        match lookup_client_meta(auth_db, &query.client_id) {
            Ok(Some(v)) => v,
            Ok(None) => {
                return html_error(
                    StatusCode::BAD_REQUEST,
                    "Unknown client",
                    "No OAuth client is registered with that client_id.",
                );
            }
            Err(_) => {
                return html_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Server error",
                    "Failed to look up the OAuth client.",
                );
            }
        };
    if !redirect_uris.iter().any(|u| u == &query.redirect_uri) {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Redirect URI mismatch",
            "The redirect_uri does not match any registered URI for this client.",
        );
    }

    // Session: ensure we have one; emit Set-Cookie when newly created.
    let (session, set_cookie_needed) = match ensure_session(auth_db, &headers, &user.id) {
        Ok(v) => v,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Failed to create a browser session.",
            );
        }
    };
    let set_cookie = if set_cookie_needed {
        Some(session_cookie_header(&session.id, SESSION_TTL_SECS))
    } else {
        None
    };

    let scopes = parse_scope(query.scope.as_deref());

    // Skip the screen if a live consent already covers the requested scopes.
    if let Ok(Some(consent)) = auth_db.get_consent(&user.id, &query.client_id)
        && consent.revoked_at.is_none()
        && consent_covers(&consent.granted_scopes, &scopes)
    {
        let _ = auth_db.touch_consent_last_used(&user.id, &query.client_id);
        let code = match auth_db.store_authorization_code(
            &query.client_id,
            &user.id,
            &query.code_challenge,
            &query.redirect_uri,
        ) {
            Ok(c) => c,
            Err(_) => {
                return html_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Server error",
                    "Failed to mint an authorization code.",
                );
            }
        };
        return redirect_with_code(
            &query.redirect_uri,
            &code,
            query.state.as_deref(),
            set_cookie,
        );
    }

    let secret = match auth_db.get_or_create_server_secret(CSRF_SECRET_NAME) {
        Ok(v) => v,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Failed to load the anti-CSRF secret.",
            );
        }
    };
    let csrf = compute_csrf_token(
        &secret,
        &session.id,
        &query.client_id,
        &query.code_challenge,
    );

    let body = render_consent_page(
        &query.client_id,
        &client_name,
        &query.redirect_uri,
        registered_at,
        &scopes,
        query.state.as_deref(),
        &query.code_challenge,
        &query.code_challenge_method,
        query.scope.as_deref(),
        &csrf,
    );
    html_response(StatusCode::OK, body, set_cookie)
}

fn redirect_with_code(
    redirect_uri: &str,
    code: &str,
    state: Option<&str>,
    set_cookie: Option<String>,
) -> Response {
    let mut url = match Url::parse(redirect_uri) {
        Ok(u) => u,
        Err(_) => {
            return html_error(
                StatusCode::BAD_REQUEST,
                "Invalid redirect_uri",
                "redirect_uri did not parse.",
            );
        }
    };
    append_query_pair(&mut url, "code", code);
    if let Some(s) = state {
        append_query_pair(&mut url, "state", s);
    }
    let mut response = Redirect::to(url.as_str()).into_response();
    if let Some(cookie) = set_cookie
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn redirect_with_error(redirect_uri: &str, error: &str, state: Option<&str>) -> Response {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Invalid redirect_uri",
            "redirect_uri did not parse.",
        );
    };
    append_query_pair(&mut url, "error", error);
    if let Some(s) = state {
        append_query_pair(&mut url, "state", s);
    }
    Redirect::to(url.as_str()).into_response()
}

fn parse_allowed_origins(base_url: &str) -> Result<Vec<Url>, ()> {
    Url::parse(base_url).map(|u| vec![u]).map_err(|_| ())
}

#[derive(Debug, Deserialize)]
pub struct DecisionForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    pub csrf_token: String,
    pub decision: String,
    #[serde(default)]
    pub remember: Option<String>,
}

pub async fn handle_authorize_decision(
    State(deps): State<Arc<ConsentDeps>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<DecisionForm>,
) -> Response {
    if !remote_addr.ip().is_loopback() {
        return html_error(
            StatusCode::FORBIDDEN,
            "Loopback only",
            "This endpoint is only available on loopback connections.",
        );
    }

    let allowed = match parse_allowed_origins(&deps.base_url) {
        Ok(v) => v,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server misconfiguration",
                "AVER_BASE_URL is not a valid URL.",
            );
        }
    };
    if let Err(err) = validate_browser_origin(&headers, &allowed) {
        return html_error(
            StatusCode::FORBIDDEN,
            "Cross-site POST rejected",
            &err.to_string(),
        );
    }

    if form.code_challenge_method != "S256" {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Unsupported code_challenge_method",
            "PKCE S256 is required.",
        );
    }

    let auth_db_guard = match deps.auth_db.lock() {
        Ok(g) => g,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Auth database lock poisoned.",
            );
        }
    };
    let auth_db: &AuthDb = &auth_db_guard;

    let (session, user) = match current_session(&headers, auth_db) {
        Some(v) => v,
        None => {
            return html_error(
                StatusCode::BAD_REQUEST,
                "Missing session",
                "No valid Aver session cookie was presented.",
            );
        }
    };
    if user.id != LOCAL_USER_ID {
        return html_error(
            StatusCode::FORBIDDEN,
            "Wrong user",
            "Session is bound to a non-loopback user.",
        );
    }

    let secret = match auth_db.get_or_create_server_secret(CSRF_SECRET_NAME) {
        Ok(v) => v,
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Failed to load the anti-CSRF secret.",
            );
        }
    };
    if !verify_csrf_token(
        &secret,
        &session.id,
        &form.client_id,
        &form.code_challenge,
        &form.csrf_token,
    ) {
        return html_error(
            StatusCode::BAD_REQUEST,
            "Invalid CSRF token",
            "The form's anti-CSRF token did not match the session.",
        );
    }

    // Re-validate redirect_uri against the registered client.
    match auth_db.client_allows_redirect_uri(&form.client_id, &form.redirect_uri) {
        Ok(true) => {}
        Ok(false) => {
            return html_error(
                StatusCode::BAD_REQUEST,
                "Redirect URI mismatch",
                "The redirect_uri does not match any registered URI for this client.",
            );
        }
        Err(_) => {
            return html_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server error",
                "Failed to check the registered redirect_uri.",
            );
        }
    }

    match form.decision.as_str() {
        "approve" => {
            let scopes = parse_scope(form.scope.as_deref());
            if auth_db
                .record_consent(&user.id, &form.client_id, &scopes)
                .is_err()
            {
                return html_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Server error",
                    "Failed to record consent.",
                );
            }
            let code = match auth_db.store_authorization_code(
                &form.client_id,
                &user.id,
                &form.code_challenge,
                &form.redirect_uri,
            ) {
                Ok(c) => c,
                Err(_) => {
                    return html_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Server error",
                        "Failed to mint an authorization code.",
                    );
                }
            };
            redirect_with_code(&form.redirect_uri, &code, form.state.as_deref(), None)
        }
        "deny" => redirect_with_error(&form.redirect_uri, "access_denied", form.state.as_deref()),
        _ => html_error(
            StatusCode::BAD_REQUEST,
            "Invalid decision",
            "Form value 'decision' must be 'approve' or 'deny'.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_handles_metacharacters() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("<x>"), "&lt;x&gt;");
        assert_eq!(html_escape("\"q\""), "&quot;q&quot;");
        assert_eq!(html_escape("'apos'"), "&#x27;apos&#x27;");
        assert_eq!(html_escape("plain text 123"), "plain text 123");
    }

    #[test]
    fn csrf_token_round_trips() {
        let secret = b"01234567890123456789012345678901";
        let token = compute_csrf_token(secret, "sess", "client", "challenge");
        assert!(verify_csrf_token(
            secret,
            "sess",
            "client",
            "challenge",
            &token
        ));
    }

    #[test]
    fn csrf_token_rejects_mismatched_inputs() {
        let secret = b"01234567890123456789012345678901";
        let token = compute_csrf_token(secret, "sess", "client", "challenge");
        assert!(!verify_csrf_token(
            secret,
            "OTHER",
            "client",
            "challenge",
            &token
        ));
        assert!(!verify_csrf_token(
            secret,
            "sess",
            "OTHER",
            "challenge",
            &token
        ));
        assert!(!verify_csrf_token(
            secret, "sess", "client", "OTHER", &token
        ));
        assert!(!verify_csrf_token(
            b"different-secret-padded-32-bytes",
            "sess",
            "client",
            "challenge",
            &token
        ));
    }

    #[test]
    fn consent_covers_treats_empty_request_as_satisfied() {
        let granted = vec!["claims:read".to_string()];
        assert!(consent_covers(&granted, &[]));
    }

    #[test]
    fn consent_covers_requires_superset() {
        let granted = vec!["claims:read".to_string(), "events:write".to_string()];
        assert!(consent_covers(&granted, &["claims:read".to_string()]));
        assert!(!consent_covers(
            &granted,
            &["claims:read".to_string(), "claims:write".to_string()]
        ));
    }

    #[test]
    fn read_cookie_finds_named_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("foo=1; aver_session=abc; bar=2"),
        );
        assert_eq!(
            read_cookie(&headers, SESSION_COOKIE_NAME),
            Some("abc".to_string())
        );
    }
}
