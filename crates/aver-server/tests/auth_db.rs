use aver_server::auth::{AuthDb, RegisteredClient, hash_token};

#[test]
fn auth_db_validates_stored_access_token_hash() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    let token_hash = hash_token("secret-token");

    db.store_access_token_hash(&token_hash, "user-1").unwrap();

    assert_eq!(
        db.validate_access_token(&token_hash).unwrap(),
        Some("user-1".to_string())
    );
    assert_eq!(
        db.validate_access_token(&hash_token("wrong-token"))
            .unwrap(),
        None
    );
}

#[test]
fn auth_db_registers_oauth_clients() {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();

    let client = db
        .register_client(
            "Aver test client",
            &["http://127.0.0.1/callback".to_string()],
        )
        .unwrap();

    assert!(client.client_id.len() > 10);
    assert_eq!(client.client_name, "Aver test client");
    assert_eq!(client.redirect_uris, ["http://127.0.0.1/callback"]);

    let loaded: RegisteredClient = db.get_client(&client.client_id).unwrap().unwrap();
    assert_eq!(loaded, client);
    assert!(db.client_allows_redirect_uri(&client.client_id, "http://127.0.0.1/callback").unwrap());
    assert!(!db.client_allows_redirect_uri(&client.client_id, "http://evil.example/callback").unwrap());
}
