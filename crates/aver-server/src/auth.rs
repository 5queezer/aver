use std::path::Path;

use base64::Engine;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::oauth::verify_pkce_s256;

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
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
                used_at        INTEGER,
                created_at     INTEGER NOT NULL
            );",
        )?;
        Ok(Self { conn })
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
    ) -> anyhow::Result<String> {
        let code = uuid::Uuid::new_v4().to_string();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO authorization_codes (code, client_id, user_id, code_challenge, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![code, client_id, user_id, code_challenge, now],
        )?;
        Ok(code)
    }

    pub fn exchange_authorization_code(
        &self,
        code: &str,
        client_id: &str,
        code_verifier: &str,
    ) -> anyhow::Result<String> {
        let (stored_client_id, user_id, code_challenge, used_at): (
            String,
            String,
            String,
            Option<i64>,
        ) = self.conn.query_row(
            "SELECT client_id, user_id, code_challenge, used_at
               FROM authorization_codes
              WHERE code = ?1",
            [code],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        anyhow::ensure!(used_at.is_none(), "authorization code already used");
        anyhow::ensure!(stored_client_id == client_id, "client_id mismatch");
        anyhow::ensure!(
            verify_pkce_s256(code_verifier, &code_challenge),
            "PKCE verifier mismatch"
        );

        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "UPDATE authorization_codes SET used_at = ?1 WHERE code = ?2",
            params![now, code],
        )?;
        let access_token = uuid::Uuid::new_v4().to_string();
        self.store_access_token_hash(&hash_token(&access_token), &user_id)?;
        Ok(access_token)
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
}
