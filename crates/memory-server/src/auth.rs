use std::path::Path;

use base64::Engine;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

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
