use std::path::Path;

use base64::Engine;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::oauth::verify_pkce_s256;

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

/// Origin of an end user's identity record.
///
/// Slice 1 of ADR-0020 only persists `Local` users. The `Header` and `Oidc`
/// variants are reserved for later slices that introduce reverse-proxy and
/// OIDC-backed identity sources; they are not yet emitted or consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserKind {
    /// Locally-authenticated user (the only kind written today).
    Local,
    /// Reserved: identity asserted by a trusted reverse-proxy header.
    Header,
    /// Reserved: identity from an OIDC issuer, identified by the issuer URL.
    Oidc(String),
}

impl UserKind {
    /// Encodes the kind into the two columns persisted in `users`:
    /// `(kind, external_id)`.
    fn to_columns(&self) -> (&'static str, Option<String>) {
        match self {
            UserKind::Local => ("local", None),
            UserKind::Header => ("header", None),
            UserKind::Oidc(issuer) => ("oidc", Some(issuer.clone())),
        }
    }

    fn from_columns(kind: &str, external_id: Option<String>) -> anyhow::Result<Self> {
        match kind {
            "local" => Ok(UserKind::Local),
            "header" => Ok(UserKind::Header),
            "oidc" => {
                let issuer = external_id
                    .ok_or_else(|| anyhow::anyhow!("oidc user is missing external_id"))?;
                Ok(UserKind::Oidc(issuer))
            }
            other => anyhow::bail!("unknown user kind {other:?}"),
        }
    }
}

/// Aver end user record. Time fields are seconds since the UNIX epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub kind: UserKind,
    pub external_id: Option<String>,
    pub created_at: i64,
}

/// Per-user, per-OAuth-client consent grant.
///
/// `granted_scopes` is a list of OAuth scope strings; persisted as a single
/// space-separated string to match the OAuth scope convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConsent {
    pub user_id: String,
    pub client_id: String,
    pub granted_scopes: Vec<String>,
    pub granted_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// Browser session bound to a user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn encode_scopes(scopes: &[String]) -> String {
    scopes.join(" ")
}

fn decode_scopes(raw: &str) -> Vec<String> {
    raw.split_ascii_whitespace().map(str::to_string).collect()
}

/// Generates a 256-bit random session identifier.
///
/// Two `uuid::Uuid::new_v4()` values are concatenated (32 bytes total) and
/// encoded as URL-safe base64 without padding. UUID v4 uses OS-provided
/// randomness, so this matches the entropy budget the brief calls for.
fn random_session_id() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub struct AuthDb {
    conn: Connection,
}

impl AuthDb {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS access_tokens (
                token_hash TEXT PRIMARY KEY,
                user_id    TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS authorization_codes (
                code           TEXT PRIMARY KEY,
                client_id      TEXT NOT NULL,
                user_id        TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                redirect_uri   TEXT NOT NULL DEFAULT '',
                used_at        INTEGER,
                expires_at     INTEGER NOT NULL DEFAULT 0,
                created_at     INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS oauth_clients (
                client_id     TEXT PRIMARY KEY,
                client_name   TEXT NOT NULL,
                redirect_uris TEXT NOT NULL,
                created_at    INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS refresh_tokens (
                token_hash TEXT PRIMARY KEY,
                user_id    TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS users (
                id          TEXT PRIMARY KEY,
                kind        TEXT NOT NULL,
                external_id TEXT,
                created_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS client_consents (
                user_id         TEXT NOT NULL,
                client_id       TEXT NOT NULL,
                granted_scopes  TEXT NOT NULL,
                granted_at      INTEGER NOT NULL,
                last_used_at    INTEGER,
                revoked_at      INTEGER,
                PRIMARY KEY (user_id, client_id),
                FOREIGN KEY (user_id) REFERENCES users(id)
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id         TEXT PRIMARY KEY,
                user_id    TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
        )?;
        // Migrate existing DBs that lack the new columns (SQLite returns an
        // error if the column already exists; we intentionally ignore it).
        let _ = conn.execute_batch(
            "ALTER TABLE authorization_codes ADD COLUMN redirect_uri TEXT NOT NULL DEFAULT '';",
        );
        let _ = conn.execute_batch(
            "ALTER TABLE authorization_codes ADD COLUMN expires_at INTEGER NOT NULL DEFAULT 0;",
        );
        Ok(Self { conn })
    }

    pub fn register_client(
        &self,
        client_name: &str,
        redirect_uris: &[String],
    ) -> anyhow::Result<RegisteredClient> {
        anyhow::ensure!(!client_name.trim().is_empty(), "client_name is required");
        anyhow::ensure!(
            !redirect_uris.is_empty(),
            "at least one redirect_uri is required"
        );
        anyhow::ensure!(
            redirect_uris
                .iter()
                .all(|uri| uri.starts_with("http://") || uri.starts_with("https://")),
            "redirect_uris must be absolute HTTP(S) URLs"
        );

        let client = RegisteredClient {
            client_id: format!("aver-{}", uuid::Uuid::new_v4()),
            client_name: client_name.to_string(),
            redirect_uris: redirect_uris.to_vec(),
        };
        let redirect_uris_json = serde_json::to_string(&client.redirect_uris)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO oauth_clients (client_id, client_name, redirect_uris, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                client.client_id,
                client.client_name,
                redirect_uris_json,
                now
            ],
        )?;
        Ok(client)
    }

    pub fn get_client(&self, client_id: &str) -> anyhow::Result<Option<RegisteredClient>> {
        let mut stmt = self.conn.prepare(
            "SELECT client_id, client_name, redirect_uris FROM oauth_clients WHERE client_id = ?1",
        )?;
        let mut rows = stmt.query([client_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let redirect_uris_json: String = row.get(2)?;
        Ok(Some(RegisteredClient {
            client_id: row.get(0)?,
            client_name: row.get(1)?,
            redirect_uris: serde_json::from_str(&redirect_uris_json)?,
        }))
    }

    pub fn client_allows_redirect_uri(
        &self,
        client_id: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<bool> {
        Ok(self
            .get_client(client_id)?
            .is_some_and(|client| client.redirect_uris.iter().any(|uri| uri == redirect_uri)))
    }

    pub fn store_access_token_hash(&self, token_hash: &str, user_id: &str) -> anyhow::Result<()> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO access_tokens (token_hash, user_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![token_hash, user_id, now],
        )?;
        Ok(())
    }

    pub fn store_authorization_code(
        &self,
        client_id: &str,
        user_id: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<String> {
        let code = uuid::Uuid::new_v4().to_string();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let expires_at = now + 600; // 10 minutes
        self.conn.execute(
            "INSERT INTO authorization_codes
             (code, client_id, user_id, code_challenge, redirect_uri, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                code,
                client_id,
                user_id,
                code_challenge,
                redirect_uri,
                expires_at,
                now
            ],
        )?;
        Ok(code)
    }

    pub fn exchange_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<String> {
        Ok(self
            .exchange_authorization_code_for_tokens(code, client_id, code_verifier, redirect_uri)?
            .access_token)
    }

    pub fn exchange_authorization_code_for_tokens(
        &self,
        code: &str,
        client_id: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<TokenPair> {
        let (stored_client_id, user_id, code_challenge, stored_redirect_uri, used_at, expires_at): (
            String,
            String,
            String,
            String,
            Option<i64>,
            i64,
        ) = self.conn.query_row(
            "SELECT client_id, user_id, code_challenge, redirect_uri, used_at, expires_at
               FROM authorization_codes
              WHERE code = ?1",
            [code],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;

        anyhow::ensure!(used_at.is_none(), "authorization code already used");
        anyhow::ensure!(stored_client_id == client_id, "client_id mismatch");
        anyhow::ensure!(
            verify_pkce_s256(code_verifier, &code_challenge),
            "PKCE verifier mismatch"
        );
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        anyhow::ensure!(now < expires_at, "authorization code expired");
        anyhow::ensure!(stored_redirect_uri == redirect_uri, "redirect_uri mismatch");

        self.conn.execute(
            "UPDATE authorization_codes SET used_at = ?1 WHERE code = ?2",
            params![now, code],
        )?;
        self.issue_token_pair(&user_id, None)
    }

    fn issue_token_pair(
        &self,
        user_id: &str,
        existing_refresh_token: Option<String>,
    ) -> anyhow::Result<TokenPair> {
        let access_token = uuid::Uuid::new_v4().to_string();
        self.store_access_token_hash(&hash_token(&access_token), user_id)?;
        let refresh_token =
            existing_refresh_token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO refresh_tokens (token_hash, user_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![hash_token(&refresh_token), user_id, now],
        )?;
        Ok(TokenPair {
            access_token,
            refresh_token,
        })
    }

    pub fn refresh_access_token(&self, refresh_token: &str) -> anyhow::Result<TokenPair> {
        let token_hash = hash_token(refresh_token);
        let user_id: String = self.conn.query_row(
            "SELECT user_id FROM refresh_tokens WHERE token_hash = ?1",
            [token_hash],
            |row| row.get(0),
        )?;
        self.issue_token_pair(&user_id, Some(refresh_token.to_string()))
    }

    pub fn validate_access_token(&self, token_hash: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id FROM access_tokens WHERE token_hash = ?1")?;
        let mut rows = stmt.query([token_hash])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    /// Inserts the user if absent or updates `kind`/`external_id` in place.
    ///
    /// `created_at` is preserved on update so existing records are not
    /// retro-dated. The supplied `User` is stored verbatim on insert.
    pub fn upsert_user(&self, user: &User) -> anyhow::Result<()> {
        anyhow::ensure!(!user.id.trim().is_empty(), "user id is required");
        anyhow::ensure!(user.created_at > 0, "user created_at must be positive");
        let (kind_str, encoded_external_id) = user.kind.to_columns();
        // Prefer the encoded external_id from the kind variant when present
        // (e.g. Oidc carries the issuer), otherwise fall back to the explicit
        // field so callers that build a `User` by hand still round-trip.
        let external_id = encoded_external_id.or_else(|| user.external_id.clone());
        self.conn.execute(
            "INSERT INTO users (id, kind, external_id, created_at)
                  VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                  kind = excluded.kind,
                  external_id = excluded.external_id",
            params![user.id, kind_str, external_id, user.created_at],
        )?;
        Ok(())
    }

    /// Loads a user by id, or returns `None` if no such user exists.
    pub fn get_user(&self, user_id: &str) -> anyhow::Result<Option<User>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, kind, external_id, created_at FROM users WHERE id = ?1",
                [user_id],
                |row| {
                    let id: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let external_id: Option<String> = row.get(2)?;
                    let created_at: i64 = row.get(3)?;
                    Ok((id, kind, external_id, created_at))
                },
            )
            .optional()?;
        let Some((id, kind, external_id, created_at)) = row else {
            return Ok(None);
        };
        let user_kind = UserKind::from_columns(&kind, external_id.clone())?;
        Ok(Some(User {
            id,
            kind: user_kind,
            external_id,
            created_at,
        }))
    }

    /// Records (or refreshes) a client's consent for a user.
    ///
    /// On update, `granted_scopes` and `granted_at` are overwritten and any
    /// prior `revoked_at` is cleared, modelling re-grant after revocation.
    /// `last_used_at` is left untouched here; advance it via
    /// [`AuthDb::touch_consent_last_used`].
    pub fn record_consent(
        &self,
        user_id: &str,
        client_id: &str,
        granted_scopes: &[String],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!user_id.trim().is_empty(), "user_id is required");
        anyhow::ensure!(!client_id.trim().is_empty(), "client_id is required");
        let now = now_unix();
        let scopes = encode_scopes(granted_scopes);
        self.conn.execute(
            "INSERT INTO client_consents
                    (user_id, client_id, granted_scopes, granted_at, last_used_at, revoked_at)
                  VALUES (?1, ?2, ?3, ?4, NULL, NULL)
             ON CONFLICT(user_id, client_id) DO UPDATE SET
                    granted_scopes = excluded.granted_scopes,
                    granted_at = excluded.granted_at,
                    revoked_at = NULL",
            params![user_id, client_id, scopes, now],
        )?;
        Ok(())
    }

    /// Loads the consent record for a `(user, client)` pair, if any.
    ///
    /// Returns `Some` even when the record has been revoked; callers are
    /// expected to inspect `revoked_at` to decide whether the grant is live.
    pub fn get_consent(
        &self,
        user_id: &str,
        client_id: &str,
    ) -> anyhow::Result<Option<ClientConsent>> {
        let row = self
            .conn
            .query_row(
                "SELECT user_id, client_id, granted_scopes, granted_at, last_used_at, revoked_at
                   FROM client_consents
                  WHERE user_id = ?1 AND client_id = ?2",
                params![user_id, client_id],
                |row| {
                    let user_id: String = row.get(0)?;
                    let client_id: String = row.get(1)?;
                    let scopes: String = row.get(2)?;
                    let granted_at: i64 = row.get(3)?;
                    let last_used_at: Option<i64> = row.get(4)?;
                    let revoked_at: Option<i64> = row.get(5)?;
                    Ok((
                        user_id,
                        client_id,
                        scopes,
                        granted_at,
                        last_used_at,
                        revoked_at,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(user_id, client_id, scopes, granted_at, last_used_at, revoked_at)| ClientConsent {
                user_id,
                client_id,
                granted_scopes: decode_scopes(&scopes),
                granted_at,
                last_used_at,
                revoked_at,
            },
        ))
    }

    /// Marks a consent record as revoked at the current time. No-op if the
    /// `(user, client)` pair has never been granted.
    pub fn revoke_consent(&self, user_id: &str, client_id: &str) -> anyhow::Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE client_consents
                SET revoked_at = ?3
              WHERE user_id = ?1 AND client_id = ?2",
            params![user_id, client_id, now],
        )?;
        Ok(())
    }

    /// Advances `last_used_at` to the current time for an active consent.
    /// No-op if no matching record exists.
    pub fn touch_consent_last_used(&self, user_id: &str, client_id: &str) -> anyhow::Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE client_consents
                SET last_used_at = ?3
              WHERE user_id = ?1 AND client_id = ?2",
            params![user_id, client_id, now],
        )?;
        Ok(())
    }

    /// Creates a new browser session with a 256-bit random id.
    ///
    /// `ttl_secs` must be positive. The returned [`Session`] carries the
    /// generated id; callers persist it client-side as a cookie value.
    pub fn create_session(&self, user_id: &str, ttl_secs: i64) -> anyhow::Result<Session> {
        anyhow::ensure!(!user_id.trim().is_empty(), "user_id is required");
        anyhow::ensure!(ttl_secs > 0, "session ttl must be positive");
        let id = random_session_id();
        let created_at = now_unix();
        let expires_at = created_at + ttl_secs;
        self.conn.execute(
            "INSERT INTO sessions (id, user_id, created_at, expires_at)
                  VALUES (?1, ?2, ?3, ?4)",
            params![id, user_id, created_at, expires_at],
        )?;
        Ok(Session {
            id,
            user_id: user_id.to_string(),
            created_at,
            expires_at,
        })
    }

    /// Returns the session if it exists and has not yet expired.
    ///
    /// Expired sessions are reported as `None` without being deleted; pruning
    /// is a separate concern.
    pub fn get_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, user_id, created_at, expires_at FROM sessions WHERE id = ?1",
                [session_id],
                |row| {
                    let id: String = row.get(0)?;
                    let user_id: String = row.get(1)?;
                    let created_at: i64 = row.get(2)?;
                    let expires_at: i64 = row.get(3)?;
                    Ok(Session {
                        id,
                        user_id,
                        created_at,
                        expires_at,
                    })
                },
            )
            .optional()?;
        let Some(session) = row else {
            return Ok(None);
        };
        if now_unix() >= session.expires_at {
            return Ok(None);
        }
        Ok(Some(session))
    }

    /// Removes a session. No-op if the id is unknown.
    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
        Ok(())
    }
}
