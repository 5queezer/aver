//! `aver-server --graph-view`: off by default, bearer + claims:read required,
//! read-only against the store. Invented data only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aver_server::{
    auth::AuthDb,
    config::ServerConfig,
    graph_view::snapshot,
    http::{build_graph_view_router, build_router, graph_page_html},
    oauth::pkce_s256_challenge,
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use rusqlite::Connection;
use tower::ServiceExt;

const SYNTHETIC_SQL: &str = include_str!("../../../contrib/graph/testdata/synthetic.sql");
const SYNTHETIC_JSON: &str = include_str!("../../../contrib/graph/testdata/synthetic.json");
/// Every committed fixture and its `export.py` golden.
const FIXTURES: [(&str, &str); 3] = [
    (SYNTHETIC_SQL, SYNTHETIC_JSON),
    (
        include_str!("../../../contrib/graph/testdata/decisions.sql"),
        include_str!("../../../contrib/graph/testdata/decisions.json"),
    ),
    (
        include_str!("../../../contrib/graph/testdata/projects.sql"),
        include_str!("../../../contrib/graph/testdata/projects.json"),
    ),
];

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// Invented store in WAL mode, like a live aver store.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("memory")).unwrap();
        let conn = Connection::open(dir.path().join("memory/db.sqlite")).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch(SYNTHETIC_SQL).unwrap();
        Self { dir }
    }

    fn memory(&self) -> PathBuf {
        self.dir.path().join("memory")
    }

    fn config(&self, auth_db: &Path) -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3317,
            base_url: "http://127.0.0.1:3317".to_string(),
            memory_dir: self.memory().to_string_lossy().to_string(),
            auth_db_path: auth_db.to_string_lossy().to_string(),
            cors_origins: Vec::new(),
            trusted_auth_header: None,
        }
    }

    fn auth_db(&self) -> PathBuf {
        self.dir.path().join("auth.db")
    }

    fn router(&self) -> Router {
        build_graph_view_router(self.config(&self.auth_db())).unwrap()
    }

    fn token(&self, scopes: &[&str]) -> String {
        let db = AuthDb::open(self.auth_db()).unwrap();
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let redirect = "http://127.0.0.1:3317/graph";
        let scopes: Vec<String> = scopes.iter().map(ToString::to_string).collect();
        let code = db
            .store_authorization_code(
                "graph-client",
                "local",
                &pkce_s256_challenge(verifier),
                redirect,
                &scopes,
            )
            .unwrap();
        db.exchange_authorization_code_for_tokens(&code, "graph-client", verifier, redirect)
            .unwrap()
            .access_token
    }

    /// Name -> bytes of every file under the memory dir. The `-shm` index is
    /// SQLite's shared lock memory (readers take read marks there), so only
    /// its presence is compared.
    fn memory_files(&self) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(self.memory())
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let bytes = if name.ends_with("-shm") {
                    Vec::new()
                } else {
                    std::fs::read(p).unwrap()
                };
                (name, bytes)
            })
            .collect()
    }
}

async fn get(
    app: Router,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, Vec<u8>, header::HeaderMap) {
    let mut request = Request::builder().uri(uri);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, body.to_vec(), headers)
}

#[test]
fn snapshot_matches_the_python_exporter_goldens() {
    for (sql, json) in FIXTURES {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        Connection::open(&db).unwrap().execute_batch(sql).unwrap();
        let expected: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(snapshot(&db).unwrap(), expected);
    }
}

/// graph.html's `prose()` is a port of `ObjectKind::is_prose`; the page side
/// runs the same cases in the headless viewer check.
#[test]
fn prose_cases_match_core_is_prose() {
    let cases: Vec<(String, bool)> = serde_json::from_str(include_str!(
        "../../../contrib/graph/testdata/prose_cases.json"
    ))
    .unwrap();
    let wrong: Vec<_> = cases
        .iter()
        .filter(|(text, expected)| aver_core::ObjectKind::is_prose(text) != *expected)
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn snapshot_keeps_legacy_projection_meaning_and_never_creates_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("legacy.sqlite");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE claims(id INTEGER,subject TEXT,predicate TEXT,object TEXT,provenance TEXT,
             confidence REAL,status TEXT,source_refs TEXT,scope TEXT);
             INSERT INTO claims VALUES(1,'sample','concerns','An invented sentence.','EXTRACTED',0.9,
             'ACTIVE','[]','global');",
        )
        .unwrap();
    let data = snapshot(&db).unwrap();
    assert_eq!(data["claims"][0]["object_kind"], "entity");
    assert_eq!(data["claims"][0]["created_at"], serde_json::Value::Null);
    assert_eq!(data["predicates"], serde_json::json!([]));
    assert_eq!(data["entities"].as_array().unwrap().len(), 2);
    assert_eq!(data["events"], serde_json::json!([]));

    let absent = dir.path().join("absent.sqlite");
    assert!(snapshot(&absent).is_err());
    assert!(!absent.exists());
}

#[tokio::test]
async fn plain_http_mode_serves_no_graph_routes() {
    let fixture = Fixture::new();
    // Plain HTTP mode opens a writer store, so give it its own empty one.
    let mut config = fixture.config(&fixture.auth_db());
    config.memory_dir = fixture
        .dir
        .path()
        .join("plain")
        .to_string_lossy()
        .to_string();
    let app = build_router(config).unwrap();
    let token = fixture.token(&["claims:read"]);
    for uri in ["/graph", "/api/graph"] {
        let (status, _, _) = get(app.clone(), uri, Some(&token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn snapshot_requires_a_bearer_token_with_claims_read() {
    let fixture = Fixture::new();
    let app = fixture.router();

    let (status, _, headers) = get(app.clone(), "/api/graph", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    let (status, _, _) = get(app.clone(), "/api/graph", Some("invented-invalid")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let write_only = fixture.token(&["claims:write"]);
    let (status, _, _) = get(app.clone(), "/api/graph", Some(&write_only)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let reader = fixture.token(&["claims:read"]);
    let (status, body, headers) = get(app, "/api/graph", Some(&reader)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let expected: serde_json::Value = serde_json::from_str(SYNTHETIC_JSON).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        expected
    );
}

#[tokio::test]
async fn graph_view_serves_no_mcp_and_a_same_origin_page() {
    let fixture = Fixture::new();
    let token = fixture.token(&["claims:read", "claims:write"]);
    let (status, _, _) = get(fixture.router(), "/mcp", Some(&token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body, headers) = get(fixture.router(), "/graph", None).await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("connect-src 'self'") && csp.contains("default-src 'none'"));
    let html = String::from_utf8(body).unwrap();
    assert_eq!(html, graph_page_html());
    assert!(html.contains("connect-src 'self'") && !html.contains("connect-src 'none'"));
}

#[tokio::test]
async fn reads_committed_state_without_changing_the_store_under_a_live_writer() {
    let fixture = Fixture::new();
    let reader = fixture.token(&["claims:read"]);
    let app = fixture.router();
    // A concurrent writer holds the WAL open with an uncommitted change.
    let mut writer = Connection::open(fixture.memory().join("db.sqlite")).unwrap();
    writer
        .execute(
            "INSERT INTO claims VALUES(5,'decision:gamma','about','topic:example','INFERRED',0.45,\
             'ACTIVE','[]','proj/example','entity',NULL,1700000400)",
            [],
        )
        .unwrap();
    let tx = writer.transaction().unwrap();
    tx.execute("DELETE FROM claims WHERE id = 1", []).unwrap();
    let before = fixture.memory_files();

    let (status, body, _) = get(app.clone(), "/api/graph", Some(&reader)).await;
    assert_eq!(status, StatusCode::OK);
    let data: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let ids: Vec<i64> = data["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [1, 2, 3, 4, 5], "committed rows only");
    assert_eq!(fixture.memory_files(), before, "reader changed the store");

    tx.commit().unwrap();
    let (_, body, _) = get(app, "/api/graph", Some(&reader)).await;
    let data: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        data["claims"].as_array().unwrap().len(),
        4,
        "reload is live"
    );
}

#[test]
fn refuses_to_start_when_it_would_write_into_the_memory_dir() {
    let fixture = Fixture::new();
    let inside = fixture.memory().join("auth.db");
    let err = build_graph_view_router(fixture.config(&inside))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("AVER_AUTH_DB_PATH"), "{err}");
    assert!(!inside.exists());

    let default_like = fixture.config(&fixture.memory().join("sub/../auth.db"));
    assert!(build_graph_view_router(default_like).is_err());
}

#[test]
fn refuses_to_start_without_an_existing_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Fixture::new().config(&dir.path().join("auth.db"));
    config.memory_dir = dir.path().join("missing").to_string_lossy().to_string();
    assert!(build_graph_view_router(config).is_err());
    assert!(!dir.path().join("missing").exists());

    std::fs::create_dir(dir.path().join("empty")).unwrap();
    let mut config = Fixture::new().config(&dir.path().join("auth.db"));
    config.memory_dir = dir.path().join("empty").to_string_lossy().to_string();
    assert!(build_graph_view_router(config).is_err());
    assert_eq!(
        std::fs::read_dir(dir.path().join("empty")).unwrap().count(),
        0
    );
}
