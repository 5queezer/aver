//! Object typing and the explicit, append-only legacy projection cleanup.

use rusqlite::{Connection, params};

use crate::apply::apply;
use crate::log::{LogRecord, append_jsonl};
use crate::privacy::{privacy_filter, privacy_filter_path};
use crate::validation::validate_claim_field;
use crate::{Error, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Entity,
    Literal,
}

impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Literal => "literal",
        }
    }

    /// Conservative deterministic boundary; explicit literals handle short
    /// fragments whose meaning cannot be established from punctuation alone.
    pub fn classify(object: &str) -> Self {
        let text = object.trim();
        let scalar = serde_json::from_str::<serde_json::Value>(text)
            .is_ok_and(|v| !v.is_array() && !v.is_object());
        if scalar || Self::is_prose(object) {
            Self::Literal
        } else {
            Self::Entity
        }
    }

    /// Prose warning/strict-mode policy, shared by adapters and migration.
    pub fn is_prose(object: &str) -> bool {
        // Quoting a JSON string changes its value kind, not its atomicity.
        if let Ok(decoded) = serde_json::from_str::<String>(object) {
            return Self::is_prose(&decoded);
        }
        let text = object.trim();
        // A short single-token path or URI can legitimately end in sentence
        // punctuation. Do not convert identifiers such as /file. or /query?.
        let token = !text.chars().any(char::is_whitespace)
            && !object.chars().any(char::is_control)
            && text.chars().count() <= 160;
        let uri_scheme = text.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
        if token
            && (matches!(text, "." | "..")
                || text.contains('/')
                || text.contains('\\')
                || uri_scheme)
        {
            return false;
        }
        text.chars().count() > 160
            || object.chars().any(char::is_control)
            || text.split_whitespace().count() > 8
            || (text.chars().any(char::is_whitespace)
                && text
                    .chars()
                    .any(|c| matches!(c, '!' | '?' | ';' | '。' | '！' | '？')))
            || text.contains(". ")
            || text.ends_with(['.', '!', '?', '。', '！', '？'])
    }
}

impl std::str::FromStr for ObjectKind {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "entity" => Ok(Self::Entity),
            "literal" => Ok(Self::Literal),
            _ => Err(Error::EnumParse {
                kind: "ObjectKind",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LiteralMigrationReport {
    pub claims_reclassified: usize,
    pub entities_removed: usize,
}

impl Store {
    /// Counts only; does not change logs or projection rows.
    pub fn preview_literal_migration(&self) -> Result<LiteralMigrationReport, Error> {
        let (claims, entities) = literal_migration_plan(&self.conn)?;
        validate_migration_names(&entities)?;
        Ok(LiteralMigrationReport {
            claims_reclassified: claims.len(),
            entities_removed: entities.len(),
        })
    }

    /// Explicit maintenance operation. Old log bytes and claim text are never
    /// edited; bounded records describe exactly which projection rows change.
    /// Re-run safely after interruption, or rebuild the projection with replay.
    pub fn migrate_literals(&self) -> Result<LiteralMigrationReport, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let (claim_ids, entity_names) = literal_migration_plan(&self.conn)?;
            // Projection-only legacy rows are untrusted. Validate the entire
            // new log payload before even the first claim-id chunk is durable.
            validate_migration_names(&entity_names)?;
            let before: usize = self
                .conn
                .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))?;
            let ts = time::OffsetDateTime::now_utc().unix_timestamp();
            for ids in claim_ids.chunks(256) {
                let record = LogRecord::MigrateLiterals {
                    ts,
                    claim_ids: ids.to_vec(),
                    entity_names: Vec::new(),
                };
                append_jsonl(&self.log_path, &record)?;
                apply(&self.conn, &record)?;
            }
            for names in entity_names.chunks(256) {
                let record = LogRecord::MigrateLiterals {
                    ts,
                    claim_ids: Vec::new(),
                    entity_names: names.to_vec(),
                };
                append_jsonl(&self.log_path, &record)?;
                apply(&self.conn, &record)?;
            }
            let after: usize = self
                .conn
                .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))?;
            Ok(LiteralMigrationReport {
                claims_reclassified: claim_ids.len(),
                entities_removed: before - after,
            })
        })();
        match result {
            Ok(report) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(report)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
}

fn literal_migration_plan(conn: &Connection) -> Result<(Vec<i64>, Vec<String>), Error> {
    let mut stmt =
        conn.prepare("SELECT id, object FROM claims WHERE object_kind = 'entity' ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut claim_ids = Vec::new();
    for row in rows {
        let (id, text) = row?;
        if ObjectKind::classify(&text) == ObjectKind::Literal {
            claim_ids.push(id);
        }
    }
    drop(stmt);
    let mut stmt = conn.prepare("SELECT name FROM entities ORDER BY name")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut entity_names = Vec::new();
    for row in rows {
        let name = row?;
        if ObjectKind::classify(&name) == ObjectKind::Literal && conn.query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM claims WHERE subject = ?1) AND NOT EXISTS(SELECT 1 FROM hyperedge_participants WHERE entity = ?1)",
                    [&name], |r| r.get::<_, bool>(0),
                )? {
                    entity_names.push(name);
                }
    }
    drop(stmt);
    Ok((claim_ids, entity_names))
}

fn validate_migration_names(names: &[String]) -> Result<(), Error> {
    for name in names {
        validate_claim_field("entity", name)?;
        privacy_filter(name)?;
        privacy_filter_path(name)?;
    }
    Ok(())
}

fn removable_entity(conn: &Connection, name: &str) -> Result<bool, Error> {
    Ok(conn.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM claims WHERE subject = ?1 OR (object = ?1 AND object_kind = 'entity'))
            AND NOT EXISTS(SELECT 1 FROM hyperedge_participants WHERE entity = ?1)",
        [name], |r| r.get(0),
    )?)
}

pub(crate) fn apply_literal_migration(
    conn: &Connection,
    claim_ids: &[i64],
    entity_names: &[String],
) -> Result<(), Error> {
    for id in claim_ids {
        let changed = conn.execute(
            "UPDATE claims SET object_kind = 'literal' WHERE id = ?1",
            [id],
        )?;
        if changed == 0 {
            return Err(Error::MissingClaim { claim_id: *id });
        }
    }
    for name in entity_names {
        if removable_entity(conn, name)? {
            conn.execute("DELETE FROM entities WHERE name = ?1", params![name])?;
        }
    }
    Ok(())
}
