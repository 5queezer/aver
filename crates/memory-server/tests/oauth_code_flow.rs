use memory_server::auth::{AuthDb, hash_token};
use memory_server::oauth::pkce_s256_challenge;

#[test]
fn auth_code_exchange_requires_matching_pkce_verifier() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_s256_challenge(verifier);
    let code = db
        .store_authorization_code("client-1", "user-1", &challenge)
        .unwrap();

    assert!(
        db.exchange_authorization_code(&code, "client-1", "wrong-verifier")
            .is_err()
    );

    let access_token = db
        .exchange_authorization_code(&code, "client-1", verifier)
        .unwrap();
    let token_hash = hash_token(&access_token);

    assert_eq!(
        db.validate_access_token(&token_hash).unwrap(),
        Some("user-1".to_string())
    );
    assert!(
        db.exchange_authorization_code(&code, "client-1", verifier)
            .is_err()
    );
}
