use memory_server::auth::{AuthDb, hash_token};

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
