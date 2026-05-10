use aver_server::auth::{AuthDb, hash_token};
use aver_server::oauth::pkce_s256_challenge;

#[test]
fn auth_code_exchange_requires_matching_pkce_verifier() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_s256_challenge(verifier);
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code("client-1", "user-1", &challenge, redirect, &[])
        .unwrap();

    assert!(
        db.exchange_authorization_code(&code, "client-1", "wrong-verifier", redirect)
            .is_err()
    );

    let access_token = db
        .exchange_authorization_code(&code, "client-1", verifier, redirect)
        .unwrap();
    let token_hash = hash_token(&access_token);

    let (user_id, _scopes) = db.validate_access_token(&token_hash).unwrap().unwrap();
    assert_eq!(user_id, "user-1");
    assert!(
        db.exchange_authorization_code(&code, "client-1", verifier, redirect)
            .is_err()
    );
}

#[test]
fn expired_code_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_s256_challenge(verifier);
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code("client-1", "user-1", &challenge, redirect, &[])
        .unwrap();

    // Manually backdate the expires_at to the past by opening DB directly.
    let conn = rusqlite::Connection::open(dir.path().join("auth.db")).unwrap();
    conn.execute(
        "UPDATE authorization_codes SET expires_at = 1 WHERE code = ?1",
        rusqlite::params![code],
    )
    .unwrap();
    drop(conn);

    let err = db
        .exchange_authorization_code(&code, "client-1", verifier, redirect)
        .unwrap_err();
    assert!(
        err.to_string().contains("expired"),
        "expected 'expired' in: {err}"
    );
}

#[test]
fn redirect_uri_mismatch_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_s256_challenge(verifier);
    let redirect = "http://localhost:8080/callback";
    let code = db
        .store_authorization_code("client-1", "user-1", &challenge, redirect, &[])
        .unwrap();

    let err = db
        .exchange_authorization_code(&code, "client-1", verifier, "http://localhost:9999/other")
        .unwrap_err();
    assert!(
        err.to_string().contains("redirect_uri"),
        "expected 'redirect_uri' in: {err}"
    );
}
