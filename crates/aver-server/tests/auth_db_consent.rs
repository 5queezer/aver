use aver_server::auth::{AuthDb, ClientConsent, User, UserKind, hash_token};

fn open_db() -> (tempfile::TempDir, AuthDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = AuthDb::open(dir.path().join("auth.db")).unwrap();
    (dir, db)
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[test]
fn schema_creation_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.db");
    let _first = AuthDb::open(&path).unwrap();
    drop(_first);
    // Re-opening must not error: ADR-0019 §schema requires re-application to
    // be a no-op.
    let _second = AuthDb::open(&path).unwrap();
}

#[test]
fn upsert_user_inserts_and_round_trips() {
    let (_dir, db) = open_db();
    let user = User {
        id: "user-1".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    };
    db.upsert_user(&user).unwrap();
    let loaded = db.get_user("user-1").unwrap().unwrap();
    assert_eq!(loaded, user);
}

#[test]
fn upsert_user_updates_kind_in_place() {
    let (_dir, db) = open_db();
    let initial_created_at = now() - 100;
    db.upsert_user(&User {
        id: "user-2".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: initial_created_at,
    })
    .unwrap();
    db.upsert_user(&User {
        id: "user-2".into(),
        kind: UserKind::Oidc("https://issuer.example".into()),
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    let loaded = db.get_user("user-2").unwrap().unwrap();
    assert_eq!(loaded.kind, UserKind::Oidc("https://issuer.example".into()));
    assert_eq!(
        loaded.external_id.as_deref(),
        Some("https://issuer.example"),
    );
    // created_at is preserved on update.
    assert_eq!(loaded.created_at, initial_created_at);
}

#[test]
fn get_user_returns_none_for_unknown() {
    let (_dir, db) = open_db();
    assert!(db.get_user("nope").unwrap().is_none());
}

#[test]
fn record_and_get_consent_round_trip() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    db.record_consent(
        "u",
        "client-1",
        &["recall".to_string(), "remember".to_string()],
    )
    .unwrap();
    let consent: ClientConsent = db.get_consent("u", "client-1").unwrap().unwrap();
    assert_eq!(consent.user_id, "u");
    assert_eq!(consent.client_id, "client-1");
    assert_eq!(
        consent.granted_scopes,
        vec!["recall".to_string(), "remember".to_string()],
    );
    assert!(consent.last_used_at.is_none());
    assert!(consent.revoked_at.is_none());
}

#[test]
fn record_consent_replaces_scopes_and_clears_revocation() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    db.record_consent("u", "c", &["recall".into()]).unwrap();
    db.revoke_consent("u", "c").unwrap();
    let revoked = db.get_consent("u", "c").unwrap().unwrap();
    assert!(revoked.revoked_at.is_some());

    db.record_consent("u", "c", &["recall".into(), "remember".into()])
        .unwrap();
    let regranted = db.get_consent("u", "c").unwrap().unwrap();
    assert!(regranted.revoked_at.is_none());
    assert_eq!(
        regranted.granted_scopes,
        vec!["recall".to_string(), "remember".to_string()],
    );
}

#[test]
fn revoke_consent_invalidates_existing_access_tokens() {
    let (_dir, db) = open_db();
    let verifier = "verifier";
    let redirect = "http://localhost:8080/callback";
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    let client = db
        .register_client("Aver test client", &[redirect.to_string()])
        .unwrap();
    db.record_consent("u", &client.client_id, &["claims:read".into()])
        .unwrap();
    let code = db
        .store_authorization_code(
            &client.client_id,
            "u",
            &aver_server::oauth::pkce_s256_challenge(verifier),
            redirect,
            &["claims:read".into()],
        )
        .unwrap();
    let tokens = db
        .exchange_authorization_code_for_tokens(&code, &client.client_id, verifier, redirect)
        .unwrap();
    let access_hash = hash_token(&tokens.access_token);
    assert!(db.validate_access_token(&access_hash).unwrap().is_some());

    db.revoke_consent("u", &client.client_id).unwrap();

    assert_eq!(db.validate_access_token(&access_hash).unwrap(), None);
    assert!(db.refresh_access_token(&tokens.refresh_token).is_err());
}

#[test]
fn touch_consent_last_used_updates_timestamp() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    db.record_consent("u", "c", &["recall".into()]).unwrap();
    assert!(
        db.get_consent("u", "c")
            .unwrap()
            .unwrap()
            .last_used_at
            .is_none()
    );
    db.touch_consent_last_used("u", "c").unwrap();
    let touched = db.get_consent("u", "c").unwrap().unwrap();
    assert!(touched.last_used_at.is_some());
}

#[test]
fn get_consent_returns_none_for_unknown() {
    let (_dir, db) = open_db();
    assert!(db.get_consent("u", "c").unwrap().is_none());
}

#[test]
fn create_session_persists_and_returns_unique_ids() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    let s1 = db.create_session("u", 3600).unwrap();
    let s2 = db.create_session("u", 3600).unwrap();
    assert_ne!(s1.id, s2.id);
    assert!(s1.id.len() >= 32, "session id too short: {}", s1.id);
    assert_eq!(s1.user_id, "u");
    assert!(s1.expires_at > s1.created_at);

    let loaded = db.get_session(&s1.id).unwrap().unwrap();
    assert_eq!(loaded, s1);
}

#[test]
fn get_session_returns_none_when_unknown() {
    let (_dir, db) = open_db();
    assert!(db.get_session("nope").unwrap().is_none());
}

#[test]
fn get_session_returns_none_when_expired() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    let session = db.create_session("u", 1).unwrap();
    // Wait past expiry. ttl=1 second, so 2s is enough.
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(db.get_session(&session.id).unwrap().is_none());
}

#[test]
fn delete_session_removes_record() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    let session = db.create_session("u", 3600).unwrap();
    db.delete_session(&session.id).unwrap();
    assert!(db.get_session(&session.id).unwrap().is_none());
}

#[test]
fn create_session_rejects_non_positive_ttl() {
    let (_dir, db) = open_db();
    db.upsert_user(&User {
        id: "u".into(),
        kind: UserKind::Local,
        external_id: None,
        created_at: now(),
    })
    .unwrap();
    assert!(db.create_session("u", 0).is_err());
    assert!(db.create_session("u", -1).is_err());
}
