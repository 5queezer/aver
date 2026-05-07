use std::path::Path;

use base64::Engine;
use rusqlite::{Connection, params};
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
            );

            CREATE TABLE IF NOT EXISTS oauth_clients (
                client_id     TEXT PRIMARY KEY,
                client_name   TEXT NOT NULL,
                redirect_uris TEXT NOT NULL,
                created_at    INTEGER NOT NULL
            );",
        )?;
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
