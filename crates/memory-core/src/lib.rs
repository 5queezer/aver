//! Memory layer core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

pub mod retrieval;
pub mod vector;

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, params, types::Type};
use serde::Serialize;

/// Embedded migrations applied in order on every `Store::open`.
/// Each `CREATE` is `IF NOT EXISTS` so re-application is a no-op (ADR-0005).
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init",
        include_str!("../../../migrations/0001_init.sql"),
    ),
    (
        "0002_vector_chunks",
        include_str!("../../../migrations/0002_vector_chunks.sql"),
    ),
    (
        "0003_ontology",
        include_str!("../../../migrations/0003_ontology.sql"),
    ),
    (
        "0004_episodic_candidates",
        include_str!("../../../migrations/0004_episodic_candidates.sql"),
    ),
];

/// Local storage for the memory layer (ADR-0006).
///
/// Layout under `memory_dir`:
///   db.sqlite  — claims, entities, episodes, contradictions
///   log.jsonl  — append-only audit log (ADR-0005, source of truth)
pub struct Store {
    conn: Connection,
    log_path: PathBuf,
    event_log_path: PathBuf,
}

/// A claim row as exposed to consumers (ADR-0003).
#[derive(Debug, Clone)]
pub struct Claim {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub status: ClaimStatus,
    pub source_refs: Vec<String>,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub write_ts: i64,
}

impl Claim {
    pub fn text(&self) -> String {
        format!("{} {} {}", self.subject, self.predicate, self.object)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpisodicEvent {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateClaim {
    pub id: i64,
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub status: String,
    pub promoted_claim_id: Option<i64>,
    pub rejection_reason: Option<String>,
}

/// A text chunk attached to a claim for vector indexing.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorChunk {
    pub id: i64,
    pub claim_id: i64,
    pub text: String,
    pub embedding_model: String,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrivacyRejection {
    #[error("AWS access key")]
    AwsAccessKey,
    #[error("GitHub personal access token")]
    GitHubPat,
    #[error("GitHub fine-grained personal access token")]
    GitHubFineGrainedPat,
    #[error("JWT")]
    Jwt,
    #[error("OpenAI API key")]
    OpenAiKey,
    #[error("Anthropic API key")]
    AnthropicKey,
    #[error("Stripe live secret key")]
    StripeLiveKey,
    #[error("private key material")]
    PrivateKey,
    #[error("high entropy token")]
    HighEntropy,
    #[error("secrets path")]
    SecretsPath,
    #[error("environment file path")]
    EnvPath,
    #[error("memory ignore marker")]
    MemoryIgnore,
    #[error("SSH path")]
    SshPath,
    #[error("key file path")]
    KeyPath,
    #[error("AWS credentials path")]
    AwsCredentialsPath,
    #[error("config path")]
    ConfigPath,
}

pub fn privacy_filter_path(path: impl AsRef<Path>) -> Result<(), PrivacyRejection> {
    let path = path.as_ref().to_string_lossy();
    if path.contains("/.secrets.d/") || path.starts_with("~/.secrets.d/") {
        return Err(PrivacyRejection::SecretsPath);
    }
    if path.contains("/.env") {
        return Err(PrivacyRejection::EnvPath);
    }
    if path.contains("/.ssh/") {
        return Err(PrivacyRejection::SshPath);
    }
    if path.ends_with("/.aws/credentials") {
        return Err(PrivacyRejection::AwsCredentialsPath);
    }
    if path.contains("/.config/") {
        return Err(PrivacyRejection::ConfigPath);
    }
    if path.ends_with(".pem") || path.ends_with(".key") {
        return Err(PrivacyRejection::KeyPath);
    }
    Ok(())
}

pub fn privacy_filter(content: &str) -> Result<(), PrivacyRejection> {
    if content
        .lines()
        .next()
        .is_some_and(|line| line.trim() == "<!-- memory:ignore -->")
        || content.lines().any(|line| line.contains("# memory:ignore"))
    {
        return Err(PrivacyRejection::MemoryIgnore);
    }
    if content.contains("BEGIN PRIVATE KEY") {
        return Err(PrivacyRejection::PrivateKey);
    }
    if content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(is_aws_access_key)
    {
        return Err(PrivacyRejection::AwsAccessKey);
    }
    if content
        .split_whitespace()
        .any(|token| token.starts_with("ghp_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split_whitespace()
        .any(|token| token.starts_with("github_pat_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubFineGrainedPat);
    }
    if content.split_whitespace().any(is_jwt) {
        return Err(PrivacyRejection::Jwt);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk-ant-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::AnthropicKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk_live_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::StripeLiveKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::OpenAiKey);
    }
    if content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token.len() > 20 && shannon_entropy(token) > 4.5)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    Ok(())
}

fn shannon_entropy(token: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in token.bytes() {
        counts[byte as usize] += 1;
    }

    let len = token.len() as f64;
    counts
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn is_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(header), Some(claims), Some(signature), None)
            if header.starts_with("eyJ")
                && header.len() >= 10
                && claims.len() >= 10
                && signature.len() >= 10
    )
}

fn is_aws_access_key(token: &str) -> bool {
    token.len() == 20
        && token.starts_with("AKIA")
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

/// Writer class for shared-mode agent provenance (ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Human,
    Llm,
    DeterministicParser,
    ExternalTool,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "HUMAN",
            Self::Llm => "LLM",
            Self::DeterministicParser => "DETERMINISTIC_PARSER",
            Self::ExternalTool => "EXTERNAL_TOOL",
        }
    }
}

impl FromStr for AgentKind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "HUMAN" => Ok(Self::Human),
            "LLM" => Ok(Self::Llm),
            "DETERMINISTIC_PARSER" => Ok(Self::DeterministicParser),
            "EXTERNAL_TOOL" => Ok(Self::ExternalTool),
            other => Err(Error::EnumParse {
                kind: "AgentKind",
                value: other.to_string(),
            }),
        }
    }
}

/// How a claim was acquired (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    UserAsserted,
    Extracted,
    Inferred,
    Ambiguous,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserAsserted => "USER_ASSERTED",
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

impl FromStr for Provenance {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "USER_ASSERTED" => Ok(Self::UserAsserted),
            "EXTRACTED" => Ok(Self::Extracted),
            "INFERRED" => Ok(Self::Inferred),
            "AMBIGUOUS" => Ok(Self::Ambiguous),
            other => Err(Error::EnumParse {
                kind: "Provenance",
                value: other.to_string(),
            }),
        }
    }
}

/// Lifecycle status of a claim (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    Active,
    Superseded,
    Invalidated,
}

impl ClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Superseded => "SUPERSEDED",
            Self::Invalidated => "INVALIDATED",
        }
    }
}

impl FromStr for ClaimStatus {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "ACTIVE" => Ok(Self::Active),
            "SUPERSEDED" => Ok(Self::Superseded),
            "INVALIDATED" => Ok(Self::Invalidated),
            other => Err(Error::EnumParse {
                kind: "ClaimStatus",
                value: other.to_string(),
            }),
        }
    }
}

impl Store {
    /// Open or create a memory store rooted at `memory_dir`.
    /// The directory is created if it does not exist; migrations are applied.
    pub fn open(memory_dir: impl AsRef<Path>) -> Result<Self, Error> {
        let memory_dir = memory_dir.as_ref();
        std::fs::create_dir_all(memory_dir)?;

        let db_path = memory_dir.join("db.sqlite");
        let log_path = memory_dir.join("log.jsonl");
        let event_log_path = memory_dir.join("events.jsonl");

        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        for (_name, sql) in MIGRATIONS {
            conn.execute_batch(sql)?;
        }

        Ok(Self {
            conn,
            log_path,
            event_log_path,
        })
    }

    /// Whether a table with the given name exists. Test/inspection helper.
    pub fn has_table(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [name],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Append a USER_ASSERTED claim. Pre-allocates the claim id, writes
    /// the JSONL log line first (source of truth, ADR-0005), then mirrors
    /// into SQLite with the same explicit id so audit replay can rebuild
    /// the DB from the log without id drift.
    /// Default confidence is 0.95 per ADR-0003's policy table.
    pub fn add_claim(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.add_claim_from_agent(
            "local",
            AgentKind::Human,
            subject,
            predicate,
            object,
            source,
        )
    }

    pub fn add_claim_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
    ) -> Result<i64, Error> {
        validate_agent_id(agent_id)?;
        privacy_filter(&format!(
            "{agent_id} {} {subject} {predicate} {object} {source}",
            agent_kind.as_str()
        ))?;

        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Pre-allocate the claim id. Single-writer assumption: rusqlite's
        // Connection is !Sync, so within a process this is race-free; SQLite
        // WAL serializes writers across processes.
        let claim_id: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                    r.get(0)
                })?;

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            claim_id,
            subject,
            predicate,
            object,
            source,
            agent_id,
            agent_kind: agent_kind.as_str(),
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(agent_id), &entry)?;

        let source_refs = serde_json::to_string(&[source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind, write_ts,
                                 created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 'USER_ASSERTED', 0.95, 'ACTIVE', ?5,
                     ?6, ?7, ?8, ?8, ?8)",
            params![
                claim_id,
                subject,
                predicate,
                object,
                source_refs,
                agent_id,
                agent_kind.as_str(),
                now
            ],
        )?;
        Ok(claim_id)
    }

    fn agent_log_path(&self, agent_id: &str) -> PathBuf {
        self.log_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("agents")
            .join(agent_id)
            .join("log.jsonl")
    }

    pub fn record_event(
        &self,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        self.record_event_from_agent("local", AgentKind::Human, session_id, kind, payload, source)
    }

    pub fn record_event_from_agent(
        &self,
        agent_id: &str,
        agent_kind: AgentKind,
        session_id: &str,
        kind: &str,
        payload: &str,
        source: &str,
    ) -> Result<i64, Error> {
        validate_agent_id(agent_id)?;
        privacy_filter(&format!(
            "{agent_id} {} {session_id} {kind} {payload} {source}",
            agent_kind.as_str()
        ))?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let event_id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM episodic_events",
            [],
            |r| r.get(0),
        )?;
        let entry = EventLogEntry {
            kind: "record_event",
            ts: now,
            event_id,
            session_id,
            event_kind: kind,
            payload,
            source,
            agent_id,
            agent_kind: agent_kind.as_str(),
        };
        append_jsonl(&self.event_log_path, &entry)?;
        self.conn.execute(
            "INSERT INTO episodic_events (id, session_id, kind, payload, source, agent_id, agent_kind, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event_id,
                session_id,
                kind,
                payload,
                source,
                agent_id,
                agent_kind.as_str(),
                now
            ],
        )?;
        Ok(event_id)
    }

    pub fn get_event(&self, id: i64) -> Result<EpisodicEvent, Error> {
        let (id, session_id, kind, payload, source, agent_id, agent_kind, ts): (
            i64,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
        ) = self.conn.query_row(
            "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts
               FROM episodic_events WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )?;

        Ok(EpisodicEvent {
            id,
            session_id,
            kind,
            payload,
            source,
            agent_id,
            agent_kind: agent_kind.parse()?,
            ts,
        })
    }

    pub fn propose_candidate_claim(
        &self,
        event_id: i64,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<i64, Error> {
        if !self.event_exists(event_id)? {
            return Err(Error::MissingEventProvenance { event_id });
        }
        privacy_filter(&format!("{subject} {predicate} {object}"))?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO candidate_claims (event_id, subject, predicate, object, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, subject, predicate, object, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn event_exists(&self, event_id: i64) -> Result<bool, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM episodic_events WHERE id = ?1",
                [event_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn promote_candidate_claim(&self, candidate_id: i64) -> Result<i64, Error> {
        let candidate = self.get_candidate_claim(candidate_id)?;
        let event = self.get_event(candidate.event_id)?;
        let source = format!("event:{}", event.id);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let claim_id: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM claims", [], |r| {
                    r.get(0)
                })?;

        let entry = LogEntry {
            kind: "add_claim",
            ts: now,
            claim_id,
            subject: &candidate.subject,
            predicate: &candidate.predicate,
            object: &candidate.object,
            source: &source,
            agent_id: &event.agent_id,
            agent_kind: event.agent_kind.as_str(),
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(&event.agent_id), &entry)?;

        let source_refs = serde_json::to_string(&[source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind, write_ts,
                                 created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ACTIVE', ?7, ?8, ?9, ?10, ?10, ?10)",
            params![
                claim_id,
                candidate.subject,
                candidate.predicate,
                candidate.object,
                candidate.provenance.as_str(),
                candidate.confidence,
                source_refs,
                event.agent_id,
                event.agent_kind.as_str(),
                now
            ],
        )?;
        self.conn.execute(
            "UPDATE candidate_claims
                SET status = 'PROMOTED', promoted_claim_id = ?1
              WHERE id = ?2",
            params![claim_id, candidate_id],
        )?;
        Ok(claim_id)
    }

    pub fn reject_candidate_claim(&self, candidate_id: i64, reason: &str) -> Result<(), Error> {
        privacy_filter(reason)?;
        self.conn.execute(
            "UPDATE candidate_claims
                SET status = 'REJECTED', rejection_reason = ?1
              WHERE id = ?2",
            params![reason, candidate_id],
        )?;
        Ok(())
    }

    pub fn get_candidate_claim(&self, id: i64) -> Result<CandidateClaim, Error> {
        self.conn
            .query_row(
                "SELECT id, event_id, subject, predicate, object, provenance, confidence, status,
                    promoted_claim_id, rejection_reason
               FROM candidate_claims WHERE id = ?1",
                [id],
                |row| {
                    let provenance: String = row.get(5)?;
                    let provenance = provenance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
                    })?;
                    Ok(CandidateClaim {
                        id: row.get(0)?,
                        event_id: row.get(1)?,
                        subject: row.get(2)?,
                        predicate: row.get(3)?,
                        object: row.get(4)?,
                        provenance,
                        confidence: row.get(6)?,
                        status: row.get(7)?,
                        promoted_claim_id: row.get(8)?,
                        rejection_reason: row.get(9)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    /// Retrieve a claim by id.
    pub fn get_claim(&self, id: i64) -> Result<Claim, Error> {
        let (
            id,
            subject,
            predicate,
            object,
            provenance,
            confidence,
            status,
            source_refs,
            agent_id,
            agent_kind,
            write_ts,
        ): (
            i64,
            String,
            String,
            String,
            String,
            f64,
            String,
            String,
            String,
            String,
            i64,
        ) = self.conn.query_row(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts
               FROM claims WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )?;

        Ok(Claim {
            id,
            subject,
            predicate,
            object,
            provenance: provenance.parse()?,
            confidence,
            status: status.parse()?,
            source_refs: serde_json::from_str(&source_refs)?,
            agent_id,
            agent_kind: agent_kind.parse()?,
            write_ts,
        })
    }

    /// Insert vector chunk metadata for a claim. The actual sqlite-vss index
    /// is wired separately; this table is the durable join point between
    /// claims and embeddings.
    pub fn add_vector_chunk(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![claim_id, text, embedding_model, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert vector chunk metadata with its embedding vector serialized for
    /// deterministic local storage. The sqlite-vss virtual table can index
    /// the same vector later; this row remains the durable join point.
    pub fn add_vector_chunk_with_embedding(
        &self,
        claim_id: i64,
        text: &str,
        embedding_model: &str,
        embedding: &[f32],
    ) -> Result<i64, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let embedding_json = serde_json::to_string(embedding)?;
        self.conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![claim_id, text, embedding_model, embedding_json, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert vector chunk metadata using the canonical claim text rendering.
    pub fn add_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        let claim = self.get_claim(claim_id)?;
        self.add_vector_chunk(claim_id, &claim.text(), embedding_model)
    }

    /// Embed the canonical claim text and persist the resulting vector chunk.
    pub fn add_embedded_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
        client: &impl vector::EmbeddingClient,
    ) -> Result<i64, Error> {
        let claim = self.get_claim(claim_id)?;
        let text = claim.text();
        let embedding = client.embed(&text)?;
        self.add_vector_chunk_with_embedding(claim_id, &text, embedding_model, &embedding)
    }

    /// Retrieve vector chunk metadata by id.
    pub fn get_vector_chunk(&self, id: i64) -> Result<VectorChunk, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, text, embedding_model, embedding_json FROM vector_chunks WHERE id = ?1",
                [id],
                |row| {
                    let embedding_json: Option<String> = row.get(4)?;
                    Ok(VectorChunk {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        text: row.get(2)?,
                        embedding_model: row.get(3)?,
                        embedding: parse_optional_embedding(embedding_json)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    /// List vector chunk metadata for a claim in stable insertion order.
    pub fn list_vector_chunks_for_claim(&self, claim_id: i64) -> Result<Vec<VectorChunk>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let rows = stmt.query_map([claim_id], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut chunks = Vec::new();
        for row in rows {
            chunks.push(row?);
        }
        Ok(chunks)
    }

    /// Rank persisted vector chunks by normalized cosine similarity to the
    /// query embedding, returning the best matches first.
    pub fn rank_vector_chunks_by_embedding(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, claim_id, text, embedding_model, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            let embedding_json: Option<String> = row.get(4)?;
            Ok(VectorChunk {
                id: row.get(0)?,
                claim_id: row.get(1)?,
                text: row.get(2)?,
                embedding_model: row.get(3)?,
                embedding: parse_optional_embedding(embedding_json)?,
            })
        })?;

        let mut scored = Vec::new();
        for row in rows {
            let chunk = row?;
            if let Some(embedding) = &chunk.embedding
                && let Some(score) = vector::normalized_cosine_score(query_embedding, embedding)
            {
                scored.push((score, chunk));
            }
        }
        scored.sort_by(|(a_score, a_chunk), (b_score, b_chunk)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_chunk.id.cmp(&b_chunk.id))
        });
        scored.truncate(top_k);
        Ok(scored.into_iter().map(|(_, chunk)| chunk).collect())
    }

    /// Embed a query with the provided client, then rank persisted vector
    /// chunks by similarity. Tests use `MockEmbeddingClient`; production can
    /// pass `OllamaEmbeddingClient` without changing storage logic.
    pub fn recall_vector_chunks(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<VectorChunk>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let query_embedding = client.embed(query)?;
        self.rank_vector_chunks_by_embedding(&query_embedding, top_k)
    }

    /// Vector recall that returns claim rows instead of internal chunk
    /// metadata, preserving the chunk ranking order.
    pub fn recall_vector_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let chunks = self.recall_vector_chunks(query, client, usize::MAX)?;
        let mut seen = HashSet::new();
        let mut claims = Vec::new();
        for chunk in chunks {
            if seen.insert(chunk.claim_id) {
                claims.push(self.get_claim(chunk.claim_id)?);
                if claims.len() == top_k {
                    break;
                }
            }
        }
        Ok(claims)
    }

    /// Hybrid recall over vector chunks plus text fallback. Vector-ranked
    /// claims are returned first; text recall fills any remaining slots with
    /// distinct claims so sparse vector indexes remain useful.
    pub fn recall_hybrid_claims(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }

        let mut claims = self.recall_vector_claims(query, client, top_k)?;
        let mut seen: HashSet<i64> = claims.iter().map(|claim| claim.id).collect();
        for claim in self.recall_text(query)? {
            if claims.len() == top_k {
                break;
            }
            if seen.insert(claim.id) {
                claims.push(claim);
            }
        }
        Ok(claims)
    }

    /// Text-only keyword recall over active claims. This is the v0.1
    /// precursor to HybridRAG: cheap SQLite substring matching across the
    /// claim triple fields, ordered deterministically by id.
    pub fn consolidate(&self) -> Result<usize, Error> {
        self.merge_duplicate_source_refs()?;
        let duplicate_changed = self.conn.execute(
            "UPDATE claims
                SET status = 'SUPERSEDED'
              WHERE id NOT IN (
                    SELECT MIN(id)
                      FROM claims
                     GROUP BY subject, predicate, object
              )
                AND status = 'ACTIVE'",
            [],
        )?;
        let conflict_changed = self.conn.execute(
            "UPDATE claims
                SET status = 'SUPERSEDED'
              WHERE status = 'ACTIVE'
                AND EXISTS (
                    SELECT 1
                      FROM claims newer
                     WHERE newer.subject = claims.subject
                       AND newer.predicate = claims.predicate
                       AND newer.object <> claims.object
                       AND newer.id > claims.id
                )",
            [],
        )?;
        Ok(duplicate_changed + conflict_changed)
    }

    fn merge_duplicate_source_refs(&self) -> Result<(), Error> {
        let mut stmt = self.conn.prepare(
            "SELECT subject, predicate, object, MIN(id)
               FROM claims
              GROUP BY subject, predicate, object
             HAVING COUNT(*) > 1",
        )?;
        let groups = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for (subject, predicate, object, survivor_id) in groups {
            let mut source_refs = Vec::new();
            let mut refs_stmt = self.conn.prepare(
                "SELECT source_refs
                   FROM claims
                  WHERE subject = ?1 AND predicate = ?2 AND object = ?3
                  ORDER BY id",
            )?;
            let refs_rows = refs_stmt.query_map(params![subject, predicate, object], |row| {
                row.get::<_, String>(0)
            })?;
            for refs_json in refs_rows {
                for source_ref in serde_json::from_str::<Vec<String>>(&refs_json?)? {
                    if !source_refs.contains(&source_ref) {
                        source_refs.push(source_ref);
                    }
                }
            }
            let merged = serde_json::to_string(&source_refs)?;
            self.conn.execute(
                "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
                params![merged, survivor_id],
            )?;
        }
        Ok(())
    }

    pub fn recall_text(&self, query: &str) -> Result<Vec<Claim>, Error> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts
               FROM claims
              WHERE status = 'ACTIVE'
                AND (subject LIKE ?1 OR predicate LIKE ?1 OR object LIKE ?1)
              ORDER BY id",
        )?;

        let rows = stmt.query_map(params![pattern], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })?;

        let mut claims = Vec::new();
        for row in rows {
            let (
                id,
                subject,
                predicate,
                object,
                provenance,
                confidence,
                status,
                source_refs,
                agent_id,
                agent_kind,
                write_ts,
            ) = row?;
            claims.push(Claim {
                id,
                subject,
                predicate,
                object,
                provenance: provenance.parse()?,
                confidence,
                status: status.parse()?,
                source_refs: serde_json::from_str(&source_refs)?,
                agent_id,
                agent_kind: agent_kind.parse()?,
                write_ts,
            });
        }
        Ok(claims)
    }
}

fn validate_agent_id(agent_id: &str) -> Result<(), Error> {
    if agent_id.is_empty()
        || !agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(Error::InvalidAgentId {
            value: agent_id.to_string(),
        });
    }
    Ok(())
}

fn parse_optional_embedding(value: Option<String>) -> rusqlite::Result<Option<Vec<f32>>> {
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
            })
        })
        .transpose()
}

#[derive(Serialize)]
struct EventLogEntry<'a> {
    kind: &'a str,
    ts: i64,
    event_id: i64,
    session_id: &'a str,
    event_kind: &'a str,
    payload: &'a str,
    source: &'a str,
    agent_id: &'a str,
    agent_kind: &'a str,
}

#[derive(Serialize)]
struct LogEntry<'a> {
    kind: &'a str,
    ts: i64,
    claim_id: i64,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    source: &'a str,
    agent_id: &'a str,
    agent_kind: &'a str,
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("embedding: {0}")]
    Embedding(#[from] vector::EmbeddingError),
    #[error("privacy filter rejected content: {0:?}")]
    Privacy(#[from] PrivacyRejection),
    #[error("invalid {kind} value in database: {value:?}")]
    EnumParse { kind: &'static str, value: String },
    #[error("invalid agent_id for partitioned log path: {value:?}")]
    InvalidAgentId { value: String },
    #[error("candidate claim must cite an existing event: {event_id}")]
    MissingEventProvenance { event_id: i64 },
}
