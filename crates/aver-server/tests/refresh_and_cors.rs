use aver_server::{auth::AuthDb, config::ServerConfig};

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
    assert_eq!(refreshed.refresh_token, tokens.refresh_token);
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
