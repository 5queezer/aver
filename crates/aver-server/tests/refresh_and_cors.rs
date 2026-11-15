use aver_server::{auth::AuthDb, config::ServerConfig};
use rusqlite::Connection;

#[test]
fn auth_code_exchange_issues_refresh_token_and_allows_refresh_grant() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &[],
        )
        .unwrap();

    let err = db
        .exchange_authorization_code_for_tokens("code", "client-1", verifier, redirect)
        .unwrap_err();
    assert!(
        err.to_string().contains("no rows") || err.to_string().contains("authorization"),
        "unexpected error: {err}"
    );

    let tokens = db
        .exchange_authorization_code_for_tokens(&code, "client-1", verifier, redirect)
        .unwrap();
    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&tokens.access_token))
            .unwrap()
            .is_some()
    );

    let refreshed = db.refresh_access_token(&tokens.refresh_token).unwrap();
    assert_ne!(refreshed.access_token, tokens.access_token);
    // Rotation: the refresh grant mints a NEW refresh token and retires the
    // presented one.
    assert_ne!(refreshed.refresh_token, tokens.refresh_token);
    assert!(db.refresh_access_token(&tokens.refresh_token).is_err());
}

#[test]
fn refresh_token_reuse_revokes_token_family() {
    // RFC 6819 §5.2.2.3: presenting an already-rotated refresh token signals
    // theft; every live token of the (user, client) family is revoked.
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &["claims:read".to_string()],
        )
        .unwrap();
    let tokens = db
        .exchange_authorization_code_for_tokens(&code, "client-1", verifier, redirect)
        .unwrap();

    // Legitimate first refresh rotates the pair.
    let rotated = db.refresh_access_token(&tokens.refresh_token).unwrap();
    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&rotated.access_token))
            .unwrap()
            .is_some()
    );

    // Attacker (or buggy client) replays the retired token.
    let err = db.refresh_access_token(&tokens.refresh_token).unwrap_err();
    assert!(
        err.to_string().contains("reuse"),
        "expected reuse-detection error, got: {err}"
    );

    // The whole family — rotated access AND refresh tokens — is dead.
    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&rotated.access_token))
            .unwrap()
            .is_none()
    );
    assert!(db.refresh_access_token(&rotated.refresh_token).is_err());
}

#[test]
fn refresh_token_reuse_revokes_only_its_own_family() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";

    let first_code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &[],
        )
        .unwrap();
    let first = db
        .exchange_authorization_code_for_tokens(&first_code, "client-1", verifier, redirect)
        .unwrap();
    let first_rotated = db.refresh_access_token(&first.refresh_token).unwrap();

    let second_code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &[],
        )
        .unwrap();
    let second = db
        .exchange_authorization_code_for_tokens(&second_code, "client-1", verifier, redirect)
        .unwrap();

    db.refresh_access_token(&first.refresh_token).unwrap_err();

    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&first_rotated.access_token))
            .unwrap()
            .is_none(),
        "replayed family must be revoked",
    );
    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&second.access_token))
            .unwrap()
            .is_some(),
        "an independent login family must survive reuse elsewhere",
    );
    assert!(db.refresh_access_token(&second.refresh_token).is_ok());
}

#[test]
fn consent_revocation_rolls_back_when_token_revocation_fails() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    let fault_db = Connection::open(&auth_db_path).unwrap();
    fault_db
        .execute(
            "INSERT INTO users (id, kind, created_at) VALUES ('user-1', 'local', 0)",
            [],
        )
        .unwrap();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";
    db.record_consent("user-1", "client-1", &["claims:read".to_string()])
        .unwrap();
    let code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &["claims:read".to_string()],
        )
        .unwrap();
    let tokens = db
        .exchange_authorization_code_for_tokens(&code, "client-1", verifier, redirect)
        .unwrap();

    fault_db
        .execute_batch(
            "CREATE TRIGGER fail_access_token_revoke
             BEFORE UPDATE OF revoked_at ON access_tokens
             BEGIN
                 SELECT RAISE(ABORT, 'injected access-token revocation failure');
             END;",
        )
        .unwrap();

    assert!(db.revoke_consent("user-1", "client-1").is_err());
    assert!(
        db.get_consent("user-1", "client-1")
            .unwrap()
            .unwrap()
            .revoked_at
            .is_none(),
        "consent update must roll back with token revocation",
    );
    assert!(
        db.validate_access_token(&aver_server::auth::hash_token(&tokens.access_token))
            .unwrap()
            .is_some(),
        "access-token state must remain unchanged after rollback",
    );
}

#[test]
fn expired_refresh_token_cannot_mint_access_token() {
    let dir = tempfile::tempdir().unwrap();
    let auth_db_path = dir.path().join("auth.db");
    let db = AuthDb::open(&auth_db_path).unwrap();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code(
            "client-1",
            "user-1",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &[],
        )
        .unwrap();
    let tokens = db
        .exchange_authorization_code_for_tokens(&code, "client-1", verifier, redirect)
        .unwrap();

    Connection::open(&auth_db_path)
        .unwrap()
        .execute(
            "UPDATE refresh_tokens SET expires_at = strftime('%s','now') - 1 WHERE token_hash = ?1",
            [aver_server::auth::hash_token(&tokens.refresh_token)],
        )
        .unwrap();

    assert!(db.refresh_access_token(&tokens.refresh_token).is_err());
}

#[test]
fn server_config_reads_comma_separated_cors_origins_from_env() {
    unsafe {
        std::env::set_var(
            "AVER_CORS_ORIGINS",
            "http://localhost:3000,https://claude.ai",
        );
    }

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(
        config.cors_origins,
        vec![
            "http://localhost:3000".to_string(),
            "https://claude.ai".to_string()
        ]
    );
}
