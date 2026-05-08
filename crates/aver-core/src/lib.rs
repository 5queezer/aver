//! Aver core: storage, episodic log, claim CRUD.
//! See doc/adr/ for architecture decisions.

pub mod retrieval;
pub mod vector;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OptionalExtension, params, types::Type};
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
    (
        "0005_contradictions",
        include_str!("../../../migrations/0005_contradictions.sql"),
    ),
    (
        "0006_observations",
        include_str!("../../../migrations/0006_observations.sql"),
    ),
    (
        "0007_entities",
        include_str!("../../../migrations/0007_entities.sql"),
    ),
];

const ENTITY_ONTOLOGY: &[(&str, Option<&str>)] = &[
    ("Thing", None),
    ("Asset", Some("Thing")),
    ("File", Some("Asset")),
    ("Module", Some("Asset")),
    ("Config", Some("Asset")),
    ("Symbol", Some("Thing")),
    ("Function", Some("Symbol")),
    ("Class", Some("Symbol")),
    ("Constant", Some("Symbol")),
    ("Process", Some("Thing")),
    ("Service", Some("Process")),
    ("Test", Some("Process")),
    ("Job", Some("Process")),
    ("Agent", Some("Thing")),
    ("Human", Some("Agent")),
    ("Bot", Some("Agent")),
    ("Concept", Some("Thing")),
    ("Decision", Some("Concept")),
    ("Bug", Some("Concept")),
    ("Pref", Some("Concept")),
    ("Constraint", Some("Concept")),
];

const PREDICATE_ONTOLOGY: &[(&str, Option<&str>)] = &[
    ("relates_to", None),
    ("depends_on", Some("relates_to")),
    ("calls", Some("depends_on")),
    ("imports", Some("depends_on")),
    ("reads_config_from", Some("depends_on")),
    ("owns", Some("relates_to")),
    ("owned_by", Some("owns")),
    ("authored", Some("owns")),
    ("maintained", Some("owns")),
    ("concerns", Some("relates_to")),
    ("fixes", Some("concerns")),
    ("tests", Some("concerns")),
    ("decides", Some("concerns")),
];

/// Local storage for Aver (ADR-0006).
///
/// Layout under `memory_dir`:
///   db.sqlite  — claims, entities, episodes, contradictions
///   log.jsonl  — append-only audit log (ADR-0005, source of truth)
pub struct Store {
    conn: Connection,
    log_path: PathBuf,
    event_log_path: PathBuf,
    observation_log_path: PathBuf,
}

/// Runtime availability of the optional sqlite-vss extension (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteVssStatus {
    Available,
    Unavailable { reason: String },
}

struct ClaimWrite<'a> {
    agent_id: &'a str,
    agent_kind: AgentKind,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    source: &'a str,
    confidence: f64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateClaimDraft {
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationRelevance {
    Low,
    Medium,
    High,
    Critical,
}

impl ObservationRelevance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

impl FromStr for ObservationRelevance {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            other => Err(Error::EnumParse {
                kind: "ObservationRelevance",
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub id: String,
    pub session_id: String,
    pub content: String,
    pub relevance: ObservationRelevance,
    pub source_event_ids: Vec<i64>,
    pub agent_id: String,
    pub agent_kind: AgentKind,
    pub derivation: String,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationDraft {
    pub content: String,
    pub relevance: ObservationRelevance,
    pub source_event_ids: Vec<i64>,
    pub derivation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservationRecall {
    pub observation: Observation,
    pub events: Vec<EpisodicEvent>,
}

pub trait Observer {
    fn observe(&self, events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockObserver {
    drafts: Vec<ObservationDraft>,
}

impl MockObserver {
    pub fn new(drafts: Vec<ObservationDraft>) -> Self {
        Self { drafts }
    }
}

impl Observer for MockObserver {
    fn observe(&self, _events: &[EpisodicEvent]) -> Result<Vec<ObservationDraft>, Error> {
        Ok(self.drafts.clone())
    }
}

#[derive(Debug, Clone)]
pub struct GraphExpansion {
    pub nodes: Vec<String>,
    pub edges: Vec<Claim>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionRecord {
    pub id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewClaim<'a> {
    pub subject: &'a str,
    pub predicate: &'a str,
    pub object: &'a str,
    pub source: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Community {
    pub id: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsolidationReport {
    pub merged: usize,
    pub superseded: usize,
    pub decayed: usize,
}

pub trait ClaimExtractor {
    fn extract(&self, events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error>;
}

#[derive(Debug, Clone)]
pub struct MockClaimExtractor {
    drafts: Vec<CandidateClaimDraft>,
}

impl MockClaimExtractor {
    pub fn new(drafts: Vec<CandidateClaimDraft>) -> Self {
        Self { drafts }
    }
}

impl ClaimExtractor for MockClaimExtractor {
    fn extract(&self, _events: &[EpisodicEvent]) -> Result<Vec<CandidateClaimDraft>, Error> {
        Ok(self.drafts.clone())
    }
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
    if path.starts_with(".secrets.d/")
        || path.contains("/.secrets.d/")
        || path.starts_with("~/.secrets.d/")
        || path.starts_with(".age/")
        || path.contains("/.age/")
        || path.starts_with(".gnupg/")
        || path.contains("/.gnupg/")
        || path == ".netrc"
        || path.ends_with("/.netrc")
        || path == ".git-credentials"
        || path.ends_with("/.git-credentials")
        || path == "auth.json"
        || path.ends_with("/auth.json")
        || path == ".nuget/NuGet/NuGet.Config"
        || path.ends_with("/.nuget/NuGet/NuGet.Config")
        || path == ".m2/settings.xml"
        || path.ends_with("/.m2/settings.xml")
        || path == ".gradle/gradle.properties"
        || path.ends_with("/.gradle/gradle.properties")
        || path == ".bundle/config"
        || path.ends_with("/.bundle/config")
        || path == ".vault-token"
        || path.ends_with("/.vault-token")
        || path == ".sentryclirc"
        || path.ends_with("/.sentryclirc")
        || path == ".npmrc"
        || path.ends_with("/.npmrc")
        || path == ".pnpmrc"
        || path.ends_with("/.pnpmrc")
        || path == ".yarnrc.yml"
        || path.ends_with("/.yarnrc.yml")
        || path == ".pypirc"
        || path.ends_with("/.pypirc")
        || path == ".gem/credentials"
        || path.ends_with("/.gem/credentials")
        || path == ".cargo/credentials.toml"
        || path.ends_with("/.cargo/credentials.toml")
        || path == ".docker/config.json"
        || path.ends_with("/.docker/config.json")
        || path == ".kube/config"
        || path.ends_with("/.kube/config")
        || path == ".azure/accessTokens.json"
        || path.ends_with("/.azure/accessTokens.json")
        || path == ".azure/msal_token_cache.json"
        || path.ends_with("/.azure/msal_token_cache.json")
        || path.ends_with("application_default_credentials.json")
        || path == ".terraform.d/credentials.tfrc.json"
        || path.ends_with("/.terraform.d/credentials.tfrc.json")
        || path == ".pulumi/credentials.json"
        || path.ends_with("/.pulumi/credentials.json")
        || path == ".oci/config"
        || path.ends_with("/.oci/config")
        || path.ends_with(".kdbx")
        || path.ends_with(".kdb")
    {
        return Err(PrivacyRejection::SecretsPath);
    }
    if path == ".env" || path == ".envrc" || path.starts_with(".env.") || path.contains("/.env") {
        return Err(PrivacyRejection::EnvPath);
    }
    if path.starts_with(".ssh/") || path.contains("/.ssh/") {
        return Err(PrivacyRejection::SshPath);
    }
    if path == ".aws/credentials"
        || path.ends_with("/.aws/credentials")
        || path == ".aws/config"
        || path.ends_with("/.aws/config")
        || path.starts_with(".aws/sso/cache/")
        || path.contains("/.aws/sso/cache/")
    {
        return Err(PrivacyRejection::AwsCredentialsPath);
    }
    if path.starts_with(".config/") || path.contains("/.config/") {
        return Err(PrivacyRejection::ConfigPath);
    }
    if path.ends_with(".pem")
        || path.ends_with(".key")
        || path.ends_with(".p12")
        || path.ends_with(".pfx")
        || path.ends_with(".ppk")
        || path.ends_with(".jks")
        || path.ends_with(".keystore")
    {
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
    if content.contains("BEGIN PRIVATE KEY")
        || content.contains("BEGIN OPENSSH PRIVATE KEY")
        || content.contains("BEGIN ENCRYPTED PRIVATE KEY")
        || content.contains("BEGIN PGP PRIVATE KEY BLOCK")
        || content.contains("BEGIN RSA PRIVATE KEY")
        || content.contains("BEGIN EC PRIVATE KEY")
        || content.contains("BEGIN DSA PRIVATE KEY")
        || content.contains("BEGIN SSH2 ENCRYPTED PRIVATE KEY")
        || content.contains("PuTTY-User-Key-File-")
        || content.contains("AGE-SECRET-KEY-")
    {
        return Err(PrivacyRejection::PrivateKey);
    }
    if content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(is_aws_access_key)
    {
        return Err(PrivacyRejection::AwsAccessKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("ghp_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("gho_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("ghu_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split_whitespace()
        .any(|token| token.starts_with("github_pat_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubFineGrainedPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("glpat-") && token.len() >= 20)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("hf_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("lin_api_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("npm_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("tskey-auth-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("tskey-api-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
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
        .any(|token| {
            (token.starts_with("xoxb-") || token.starts_with("xoxp-") || token.starts_with("xapp-"))
                && token.len() >= 20
        })
    {
        return Err(PrivacyRejection::HighEntropy);
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
        let observation_log_path = memory_dir.join("observations.jsonl");

        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        for (_name, sql) in MIGRATIONS {
            conn.execute_batch(sql)?;
        }
        seed_ontology(&conn)?;

        Ok(Self {
            conn,
            log_path,
            event_log_path,
            observation_log_path,
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

    /// Probe sqlite-vss capability without requiring a network or system service.
    pub fn sqlite_vss_status(&self) -> Result<SqliteVssStatus, Error> {
        let available = self
            .conn
            .query_row("SELECT vss_version()", [], |_| Ok(()))
            .is_ok();
        if available {
            Ok(SqliteVssStatus::Available)
        } else {
            Ok(SqliteVssStatus::Unavailable {
                reason: "sqlite-vss extension is not loaded".to_string(),
            })
        }
    }

    /// Prepare the optional sqlite-vss vector index when the extension exists.
    pub fn prepare_sqlite_vss_index(&self, dimensions: usize) -> Result<SqliteVssStatus, Error> {
        match self.sqlite_vss_status()? {
            SqliteVssStatus::Available => {
                self.conn.execute_batch(&format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS vector_index USING vss0(embedding({dimensions}))"
                ))?;
                Ok(SqliteVssStatus::Available)
            }
            unavailable => Ok(unavailable),
        }
    }

    pub fn vector_index_table_exists(&self) -> Result<bool, Error> {
        Ok(self.has_table("vector_index"))
    }

    pub fn predicate_implies(&self, predicate: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("predicate", ancestor)?;
        if predicate == ancestor {
            return Ok(true);
        }
        let Some(predicate_id) = self.predicate_type_id(predicate)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.predicate_type_id(ancestor)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM predicate_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![predicate_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_is_a(&self, type_name: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("entity_type", type_name)?;
        validate_claim_field("entity_type", ancestor)?;
        if type_name == ancestor {
            return Ok(true);
        }
        let Some(type_id) = self.entity_type_id(type_name)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.entity_type_id(ancestor)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM entity_type_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![type_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }

    pub fn entity_type_name(&self, entity: &str) -> Result<String, Error> {
        validate_claim_field("entity", entity)?;
        self.conn
            .query_row(
                "SELECT entity_types.name
                   FROM entities
                   JOIN entity_types ON entity_types.id = entities.type_id
                  WHERE entities.name = ?1",
                [entity],
                |row| row.get(0),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEntity {
                    entity: entity.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn entity_is_a_type(&self, entity: &str, ancestor: &str) -> Result<bool, Error> {
        let type_name = self.entity_type_name(entity)?;
        self.entity_type_is_a(&type_name, ancestor)
    }

    fn expand_predicate_filter(&self, predicates: &[&str]) -> Result<HashSet<String>, Error> {
        let mut allowed = HashSet::new();
        for predicate in predicates {
            allowed.insert((*predicate).to_string());
            let mut stmt = self.conn.prepare(
                "SELECT child.name
                   FROM predicate_types child
                   JOIN predicate_closure closure ON closure.child_id = child.id
                   JOIN predicate_types ancestor ON ancestor.id = closure.ancestor_id
                  WHERE ancestor.name = ?1
                  ORDER BY child.id",
            )?;
            let rows = stmt.query_map([*predicate], |row| row.get::<_, String>(0))?;
            for row in rows {
                allowed.insert(row?);
            }
        }
        Ok(allowed)
    }

    fn ensure_entity(&self, entity: &str, now: i64) -> Result<(), Error> {
        let inferred_type = self.infer_entity_type_name(entity)?;
        let type_id = self.entity_type_id(&inferred_type)?.unwrap_or_else(|| {
            self.entity_type_id("Thing")
                .expect("Thing lookup should not fail")
                .expect("ontology bootstrap should seed Thing")
        });
        let thing_id = self
            .entity_type_id("Thing")?
            .expect("ontology bootstrap should seed Thing");
        let current: Option<i64> = self
            .conn
            .query_row(
                "SELECT type_id FROM entities WHERE name = ?1",
                [entity],
                |row| row.get(0),
            )
            .optional()?;
        match current {
            None => {
                self.conn.execute(
                    "INSERT INTO entities (name, type_id, created_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![entity, type_id, now],
                )?;
            }
            Some(existing) if existing == thing_id && type_id != thing_id => {
                self.conn.execute(
                    "UPDATE entities SET type_id = ?2, last_seen_at = ?3 WHERE name = ?1",
                    params![entity, type_id, now],
                )?;
            }
            Some(_) => {
                self.conn.execute(
                    "UPDATE entities SET last_seen_at = ?2 WHERE name = ?1",
                    params![entity, now],
                )?;
            }
        }
        Ok(())
    }

    fn infer_entity_type_name(&self, entity: &str) -> Result<String, Error> {
        if let Some((prefix, _rest)) = entity.split_once(':')
            && self.entity_type_id(prefix)?.is_some()
        {
            return Ok(prefix.to_string());
        }
        match entity {
            "User" => Ok("Human".to_string()),
            "Claude" | "Pi" => Ok("Bot".to_string()),
            _ => Ok("Thing".to_string()),
        }
    }

    fn entity_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM entity_types WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }

    fn predicate_type_id(&self, name: &str) -> Result<Option<i64>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM predicate_types WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
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
        self.add_claim_with_confidence(subject, predicate, object, source, 0.95)
    }

    pub fn add_claim_with_confidence(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        confidence: f64,
    ) -> Result<i64, Error> {
        self.insert_claim(ClaimWrite {
            agent_id: "local",
            agent_kind: AgentKind::Human,
            subject,
            predicate,
            object,
            source,
            confidence,
        })
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
        self.insert_claim(ClaimWrite {
            agent_id,
            agent_kind,
            subject,
            predicate,
            object,
            source,
            confidence: 0.95,
        })
    }

    fn insert_claim(&self, write: ClaimWrite<'_>) -> Result<i64, Error> {
        validate_claim_field("subject", write.subject)?;
        validate_claim_field("predicate", write.predicate)?;
        validate_claim_field("object", write.object)?;
        validate_claim_field("source", write.source)?;
        if !(0.0..=1.0).contains(&write.confidence) {
            return Err(Error::InvalidConfidence {
                value: write.confidence,
            });
        }
        validate_agent_id(write.agent_id)?;
        privacy_filter(&format!(
            "{} {} {} {} {} {}",
            write.agent_id,
            write.agent_kind.as_str(),
            write.subject,
            write.predicate,
            write.object,
            write.source
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
            subject: write.subject,
            predicate: write.predicate,
            object: write.object,
            source: write.source,
            agent_id: write.agent_id,
            agent_kind: write.agent_kind.as_str(),
            confidence: write.confidence,
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(write.agent_id), &entry)?;

        self.ensure_entity(write.subject, now)?;
        self.ensure_entity(write.object, now)?;

        let source_refs = serde_json::to_string(&[write.source])?;
        self.conn.execute(
            "INSERT INTO claims (id, subject, predicate, object, provenance, confidence,
                                 status, source_refs, agent_id, agent_kind, write_ts,
                                 created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, 'USER_ASSERTED', ?5, 'ACTIVE', ?6,
                     ?7, ?8, ?9, ?9, ?9)",
            params![
                claim_id,
                write.subject,
                write.predicate,
                write.object,
                write.confidence,
                source_refs,
                write.agent_id,
                write.agent_kind.as_str(),
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
        validate_event_field("session_id", session_id)?;
        validate_event_field("kind", kind)?;
        validate_event_field("source", source)?;
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
        ) = self
            .conn
            .query_row(
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
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingEvent { event_id: id },
                other => Error::Sqlite(other),
            })?;

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

    pub fn propose_claims_from_extractor(
        &self,
        session_id: &str,
        extractor: &impl ClaimExtractor,
    ) -> Result<Vec<i64>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = extractor.extract(&events)?;
        let mut candidate_ids = Vec::new();
        for draft in drafts {
            candidate_ids.push(self.propose_candidate_claim(
                draft.event_id,
                &draft.subject,
                &draft.predicate,
                &draft.object,
            )?);
        }
        Ok(candidate_ids)
    }

    pub fn list_events_for_session(&self, session_id: &str) -> Result<Vec<EpisodicEvent>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM episodic_events WHERE session_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(self.get_event(row?)?);
        }
        Ok(events)
    }

    pub fn record_observation(
        &self,
        session_id: &str,
        content: &str,
        relevance: ObservationRelevance,
        source_event_ids: &[i64],
        derivation: &str,
    ) -> Result<String, Error> {
        validate_event_field("session_id", session_id)?;
        validate_observation_field("content", content)?;
        validate_observation_field("derivation", derivation)?;
        if source_event_ids.is_empty() {
            return Err(Error::MissingEventProvenance { event_id: 0 });
        }
        privacy_filter(&format!("{session_id} {content} {derivation}"))?;

        let mut events = Vec::new();
        for event_id in source_event_ids {
            let event = self.get_event(*event_id)?;
            if event.session_id != session_id {
                return Err(Error::MissingEventProvenance {
                    event_id: *event_id,
                });
            }
            events.push(event);
        }
        let first_event = events
            .first()
            .expect("source_event_ids is checked non-empty before event lookup");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let id = observation_id(session_id, content, source_event_ids);
        let source_event_ids_json = serde_json::to_string(source_event_ids)?;
        let entry = ObservationLogEntry {
            kind: "record_observation",
            ts: now,
            observation_id: &id,
            session_id,
            content,
            relevance: relevance.as_str(),
            source_event_ids,
            agent_id: &first_event.agent_id,
            agent_kind: first_event.agent_kind.as_str(),
            derivation,
        };
        append_jsonl(&self.observation_log_path, &entry)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO observations
             (id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                session_id,
                content,
                relevance.as_str(),
                source_event_ids_json,
                first_event.agent_id,
                first_event.agent_kind.as_str(),
                derivation,
                now
            ],
        )?;
        Ok(id)
    }

    pub fn propose_observations_from_observer(
        &self,
        session_id: &str,
        observer: &impl Observer,
    ) -> Result<Vec<String>, Error> {
        let events = self.list_events_for_session(session_id)?;
        let drafts = observer.observe(&events)?;
        let mut ids = Vec::new();
        for draft in drafts {
            ids.push(self.record_observation(
                session_id,
                &draft.content,
                draft.relevance,
                &draft.source_event_ids,
                &draft.derivation,
            )?);
        }
        Ok(ids)
    }

    pub fn get_observation(&self, id: &str) -> Result<Observation, Error> {
        validate_observation_field("id", id)?;
        self.conn
            .query_row(
                "SELECT id, session_id, content, relevance, source_event_ids, agent_id, agent_kind, derivation, ts
                   FROM observations WHERE id = ?1",
                [id],
                |row| {
                    let relevance: String = row.get(3)?;
                    let relevance = relevance.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err))
                    })?;
                    let source_event_ids_json: String = row.get(4)?;
                    let source_event_ids = serde_json::from_str(&source_event_ids_json).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err))
                    })?;
                    let agent_kind: String = row.get(6)?;
                    let agent_kind = agent_kind.parse().map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(err))
                    })?;
                    Ok(Observation {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        content: row.get(2)?,
                        relevance,
                        source_event_ids,
                        agent_id: row.get(5)?,
                        agent_kind,
                        derivation: row.get(7)?,
                        ts: row.get(8)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingObservation {
                    observation_id: id.to_string(),
                },
                other => Error::Sqlite(other),
            })
    }

    pub fn list_observations_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<Observation>, Error> {
        validate_event_field("session_id", session_id)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM observations WHERE session_id = ?1 ORDER BY ts, id")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut observations = Vec::new();
        for row in rows {
            observations.push(self.get_observation(&row?)?);
        }
        Ok(observations)
    }

    pub fn recall_observation(&self, id: &str) -> Result<ObservationRecall, Error> {
        let observation = self.get_observation(id)?;
        let mut events = Vec::new();
        for event_id in &observation.source_event_ids {
            events.push(self.get_event(*event_id)?);
        }
        Ok(ObservationRecall {
            observation,
            events,
        })
    }

    pub fn assemble_compaction_summary(&self, session_id: &str) -> Result<String, Error> {
        let observations = self.list_observations_for_session(session_id)?;
        let mut summary = String::from("# Aver session continuity summary\n\n");
        if observations.is_empty() {
            summary.push_str("No observations recorded.\n");
            return Ok(summary);
        }
        for observation in observations {
            summary.push_str(&format!(
                "- [{}] {} (id={}, source_events={:?})\n",
                observation.relevance.as_str(),
                observation.content,
                observation.id,
                observation.source_event_ids
            ));
        }
        Ok(summary)
    }

    pub fn prune_observations(&self, session_id: &str, keep: usize) -> Result<usize, Error> {
        validate_event_field("session_id", session_id)?;
        let mut observations = self.list_observations_for_session(session_id)?;
        if observations.len() <= keep {
            return Ok(0);
        }
        observations.sort_by_key(|observation| {
            (
                observation.relevance.rank(),
                observation.ts,
                observation.id.clone(),
            )
        });
        let drop_count = observations.len() - keep;
        for observation in observations.iter().take(drop_count) {
            self.conn
                .execute("DELETE FROM observations WHERE id = ?1", [&observation.id])?;
        }
        Ok(drop_count)
    }

    pub fn should_extract_memories(
        &self,
        session_id: &str,
        event_threshold: usize,
    ) -> Result<bool, Error> {
        validate_event_field("session_id", session_id)?;
        if event_threshold == 0 {
            return Err(Error::InvalidEventThreshold);
        }
        let explicit_remember = self
            .conn
            .query_row(
                "SELECT 1 FROM episodic_events
                  WHERE session_id = ?1 AND kind = 'explicit_remember'
                  LIMIT 1",
                [session_id],
                |_| Ok(()),
            )
            .is_ok();
        if explicit_remember {
            return Ok(true);
        }

        let event_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM episodic_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(event_count >= event_threshold)
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
        validate_claim_field("subject", subject)?;
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("object", object)?;
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
        if candidate.status == "REJECTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        if let Some(claim_id) = candidate.promoted_claim_id {
            return Ok(claim_id);
        }
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
            confidence: candidate.confidence,
        };
        append_jsonl(&self.log_path, &entry)?;
        append_jsonl(&self.agent_log_path(&event.agent_id), &entry)?;
        self.ensure_entity(&candidate.subject, now)?;
        self.ensure_entity(&candidate.object, now)?;

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
        let candidate = match self.get_candidate_claim(candidate_id) {
            Ok(candidate) => candidate,
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingCandidate { candidate_id });
            }
            Err(err) => return Err(err),
        };
        if candidate.status == "PROMOTED" {
            return Err(Error::InvalidCandidateStatus {
                candidate_id,
                status: candidate.status,
            });
        }
        validate_rejection_reason(reason)?;
        privacy_filter(reason)?;
        let rows_changed = self.conn.execute(
            "UPDATE candidate_claims
                SET status = 'REJECTED', rejection_reason = ?1
              WHERE id = ?2",
            params![reason, candidate_id],
        )?;
        if rows_changed == 0 {
            return Err(Error::MissingCandidate { candidate_id });
        }
        Ok(())
    }

    pub fn list_candidate_claims(
        &self,
        session_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<CandidateClaim>, Error> {
        if let Some(session_id) = session_id {
            validate_event_field("session_id", session_id)?;
        }
        if let Some(status) = status {
            validate_candidate_status_filter(status)?;
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM candidate_claims ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut candidates = Vec::new();
        for row in rows {
            let candidate = self.get_candidate_claim(row?)?;
            if let Some(status) = status
                && candidate.status != status
            {
                continue;
            }
            if let Some(session_id) = session_id {
                let event = self.get_event(candidate.event_id)?;
                if event.session_id != session_id {
                    continue;
                }
            }
            candidates.push(candidate);
        }
        Ok(candidates)
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
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::MissingCandidate { candidate_id: id }
                }
                other => Error::Sqlite(other),
            })
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
        ) = self
            .conn
            .query_row(
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
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingClaim { claim_id: id },
                other => Error::Sqlite(other),
            })?;

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
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
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
        self.ensure_claim_exists(claim_id)?;
        validate_vector_chunk_text(text)?;
        validate_embedding_model(embedding_model)?;
        validate_embedding_vector(embedding)?;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let embedding_json = serde_json::to_string(embedding)?;
        self.conn.execute(
            "INSERT INTO vector_chunks (claim_id, text, embedding_model, embedding_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![claim_id, text, embedding_model, embedding_json, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn ensure_claim_exists(&self, claim_id: i64) -> Result<(), Error> {
        match self.get_claim(claim_id) {
            Ok(_) => Ok(()),
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                Err(Error::MissingClaim { claim_id })
            }
            Err(err) => Err(err),
        }
    }

    /// Insert vector chunk metadata using the canonical claim text rendering.
    pub fn add_vector_chunk_for_claim(
        &self,
        claim_id: i64,
        embedding_model: &str,
    ) -> Result<i64, Error> {
        self.ensure_claim_exists(claim_id)?;
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
        self.ensure_claim_exists(claim_id)?;
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
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Error::MissingVectorChunk { chunk_id: id },
                other => Error::Sqlite(other),
            })
    }

    /// List vector chunk metadata for a claim in stable insertion order.
    pub fn list_vector_chunks_for_claim(&self, claim_id: i64) -> Result<Vec<VectorChunk>, Error> {
        self.ensure_claim_exists(claim_id)?;
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
        validate_top_k(top_k)?;
        validate_embedding_vector(query_embedding)?;
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
        validate_recall_query(query)?;

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
                let claim = self.get_claim(chunk.claim_id)?;
                if claim.status != ClaimStatus::Active {
                    continue;
                }
                claims.push(claim);
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
        self.recall_hybrid_claims_with_alpha(
            query,
            client,
            top_k,
            retrieval::HybridWeights::for_query(query),
        )
    }

    pub fn recall_hybrid_claims_with_alpha(
        &self,
        query: &str,
        client: &impl vector::EmbeddingClient,
        top_k: usize,
        weights: retrieval::HybridWeights,
    ) -> Result<Vec<Claim>, Error> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        validate_recall_query(query)?;
        let query_embedding = client.embed(query)?;

        let mut vector_scores = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT claim_id, embedding_json
               FROM vector_chunks
              WHERE embedding_json IS NOT NULL
              ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (claim_id, embedding_json) = row?;
            if let Some(embedding) = parse_optional_embedding(embedding_json)?
                && let Some(score) = vector::normalized_cosine_score(&query_embedding, &embedding)
            {
                vector_scores
                    .entry(claim_id)
                    .and_modify(|current: &mut f64| *current = current.max(f64::from(score)))
                    .or_insert(f64::from(score));
            }
        }
        drop(stmt);

        let text_claims = self.recall_text(query).unwrap_or_default();
        let mut candidate_ids: HashSet<i64> = text_claims.iter().map(|claim| claim.id).collect();
        candidate_ids.extend(vector_scores.keys().copied());

        let mut candidates = Vec::new();
        for claim_id in candidate_ids {
            let claim = self.get_claim(claim_id)?;
            if claim.status != ClaimStatus::Active {
                continue;
            }
            let vector_score = vector_scores.get(&claim_id).copied().unwrap_or(0.0);
            let graph_score = graph_score_for_query_claim(query, &claim);
            candidates.push((weights.blend(vector_score, graph_score), claim));
        }
        candidates.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        candidates.truncate(top_k);
        Ok(candidates.into_iter().map(|(_, claim)| claim).collect())
    }

    /// Text-only keyword recall over active claims. This is the v0.1
    /// precursor to HybridRAG: cheap SQLite substring matching across the
    /// claim triple fields, ordered deterministically by id.
    pub fn expand(
        &self,
        entity: &str,
        hops: usize,
        predicates: Option<&[&str]>,
    ) -> Result<GraphExpansion, Error> {
        if entity.trim().is_empty() {
            return Err(Error::InvalidGraphEntity);
        }
        if hops == 0 {
            return Err(Error::InvalidGraphHops);
        }

        let predicate_filter = if let Some(items) = predicates {
            if items.is_empty() || items.iter().any(|item| item.trim().is_empty()) {
                return Err(Error::InvalidPredicateFilter);
            }
            Some(self.expand_predicate_filter(items)?)
        } else {
            None
        };
        let mut nodes = vec![entity.to_string()];
        let mut seen_nodes = HashSet::from([entity.to_string()]);
        let mut seen_edges = HashSet::new();
        let mut queue = VecDeque::from([(entity.to_string(), 0usize)]);
        let mut edges = Vec::new();

        while let Some((current, depth)) = queue.pop_front() {
            if depth == hops {
                continue;
            }
            for claim in self.active_claim_edges_for_entity(&current)? {
                if predicate_filter
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(claim.predicate.as_str()))
                {
                    continue;
                }
                if seen_edges.insert(claim.id) {
                    for node in [&claim.subject, &claim.object] {
                        if seen_nodes.insert(node.clone()) {
                            nodes.push(node.clone());
                            queue.push_back((node.clone(), depth + 1));
                        }
                    }
                    edges.push(claim);
                }
            }
        }

        Ok(GraphExpansion { nodes, edges })
    }

    fn active_claim_edges_for_entity(&self, entity: &str) -> Result<Vec<Claim>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
                AND (subject = ?1 OR object = ?1)
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([entity], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.get_claim(id)).collect()
    }

    pub fn detect_communities(&self) -> Result<Vec<Community>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
              ORDER BY id",
        )?;
        let claims = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|id| self.get_claim(id))
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen_nodes = HashSet::new();
        let mut communities = Vec::new();
        for claim in &claims {
            if seen_nodes.contains(&claim.subject) && seen_nodes.contains(&claim.object) {
                continue;
            }
            let graph = self.expand(&claim.subject, usize::MAX, None)?;
            let members: Vec<String> = graph
                .nodes
                .into_iter()
                .filter(|node| seen_nodes.insert(node.clone()))
                .collect();
            if !members.is_empty() {
                let id = format!("community:{}", members[0]);
                communities.push(Community { id, members });
            }
        }
        Ok(communities)
    }

    pub fn contradict(
        &self,
        claim_id: i64,
        reason: &str,
        new_claim: Option<NewClaim<'_>>,
    ) -> Result<ContradictionRecord, Error> {
        match self.get_claim(claim_id) {
            Ok(_) => {}
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingClaim { claim_id });
            }
            Err(err) => return Err(err),
        }
        validate_contradiction_reason(reason)?;
        privacy_filter(reason)?;
        let new_claim_id = if let Some(claim) = new_claim {
            Some(self.add_claim(claim.subject, claim.predicate, claim.object, claim.source)?)
        } else {
            None
        };
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        self.conn.execute(
            "INSERT INTO contradictions (claim_id, reason, new_claim_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![claim_id, reason, new_claim_id, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_contradiction(id)
    }

    pub fn list_contradictions(&self, claim_id: i64) -> Result<Vec<ContradictionRecord>, Error> {
        match self.get_claim(claim_id) {
            Ok(_) => {}
            Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(Error::MissingClaim { claim_id });
            }
            Err(err) => return Err(err),
        }
        let mut stmt = self.conn.prepare(
            "SELECT id
               FROM contradictions
              WHERE claim_id = ?1
              ORDER BY id",
        )?;
        let ids = stmt
            .query_map([claim_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.get_contradiction(id))
            .collect()
    }

    fn get_contradiction(&self, id: i64) -> Result<ContradictionRecord, Error> {
        self.conn
            .query_row(
                "SELECT id, claim_id, reason, new_claim_id, status, created_at
               FROM contradictions
              WHERE id = ?1",
                [id],
                |row| {
                    Ok(ContradictionRecord {
                        id: row.get(0)?,
                        claim_id: row.get(1)?,
                        reason: row.get(2)?,
                        new_claim_id: row.get(3)?,
                        status: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    pub fn should_merge_synonym(similarity: f64) -> bool {
        similarity >= 0.92
    }

    pub fn decay_contradicted_confidence(&self) -> Result<usize, Error> {
        Ok(self.conn.execute(
            "UPDATE claims
                SET confidence = MAX(0.0, ROUND(confidence - 0.10, 2))
              WHERE status = 'ACTIVE'
                AND EXISTS (
                    SELECT 1
                      FROM contradictions
                     WHERE contradictions.claim_id = claims.id
                       AND contradictions.status = 'RECORDED'
                )",
            [],
        )?)
    }

    pub fn decay_inferred_confidence_at(
        &self,
        now_ts: i64,
        tau_seconds: f64,
    ) -> Result<usize, Error> {
        if tau_seconds <= 0.0 || !tau_seconds.is_finite() {
            return Err(Error::InvalidDecayTau { value: tau_seconds });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, confidence, last_seen_at
               FROM claims
              WHERE status = 'ACTIVE' AND provenance = 'INFERRED'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut changed = 0;
        for (id, confidence, last_seen_at) in rows {
            let delta = now_ts.saturating_sub(last_seen_at) as f64;
            let decayed = confidence * (-delta / tau_seconds).exp();
            self.conn.execute(
                "UPDATE claims SET confidence = ?1 WHERE id = ?2",
                params![decayed, id],
            )?;
            changed += 1;
        }
        Ok(changed)
    }

    pub fn consolidate(&self) -> Result<usize, Error> {
        Ok(self.consolidate_report()?.superseded)
    }

    pub fn consolidate_report(&self) -> Result<ConsolidationReport, Error> {
        let merged = self.merge_duplicate_source_refs()?;
        let decayed = self.decay_contradicted_confidence()?;
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
        Ok(ConsolidationReport {
            merged,
            superseded: duplicate_changed + conflict_changed,
            decayed,
        })
    }

    fn merge_duplicate_source_refs(&self) -> Result<usize, Error> {
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

        let mut merged_groups = 0;
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
            let survivor = self.get_claim(survivor_id)?;
            let should_promote =
                survivor.provenance == Provenance::Inferred && source_refs.len() >= 2;
            if should_promote {
                self.conn.execute(
                    "UPDATE claims
                        SET source_refs = ?1, provenance = 'EXTRACTED', confidence = MAX(confidence, 0.75)
                      WHERE id = ?2",
                    params![merged, survivor_id],
                )?;
            } else {
                self.conn.execute(
                    "UPDATE claims SET source_refs = ?1 WHERE id = ?2",
                    params![merged, survivor_id],
                )?;
            }
            merged_groups += 1;
        }
        Ok(merged_groups)
    }

    pub fn recall_text(&self, query: &str) -> Result<Vec<Claim>, Error> {
        let query_tokens = query_tokens_for_recall(query);
        if query_tokens.is_empty() {
            return Err(Error::InvalidRecallQuery);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, subject, predicate, object, provenance, confidence, status, source_refs,
                    agent_id, agent_kind, write_ts
               FROM claims
              WHERE status = 'ACTIVE'
              ORDER BY id",
        )?;

        let rows = stmt.query_map([], |row| {
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

        let mut scored_claims = Vec::new();
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
            let claim = Claim {
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
            };
            let score = recall_token_score(&query_tokens, &claim);
            if score > 0 {
                scored_claims.push((score, claim));
            }
        }
        let max_score = scored_claims
            .iter()
            .map(|(score, _)| *score)
            .max()
            .unwrap_or(0);
        let name_anchor_subject = if query_tokens.len() == 2 && max_score >= 4 {
            scored_claims
                .iter()
                .find(|(score, claim)| {
                    *score == max_score && claim.predicate.eq_ignore_ascii_case("name")
                })
                .map(|(_, claim)| claim.subject.clone())
        } else {
            None
        };
        let has_name_anchor = name_anchor_subject.is_some();
        let minimum_score = if has_name_anchor {
            2
        } else if max_score >= 5 {
            max_score - 1
        } else if max_score >= 3 {
            3
        } else if max_score >= 2 {
            2
        } else {
            1
        };
        scored_claims.retain(|(score, claim)| {
            *score >= minimum_score
                && name_anchor_subject
                    .as_ref()
                    .is_none_or(|subject| *score == max_score || claim.subject == *subject)
        });
        scored_claims.sort_by(|(a_score, a_claim), (b_score, b_claim)| {
            b_score
                .cmp(a_score)
                .then_with(|| a_claim.id.cmp(&b_claim.id))
        });
        Ok(scored_claims.into_iter().map(|(_, claim)| claim).collect())
    }
}

fn graph_score_for_query_claim(query: &str, claim: &Claim) -> f64 {
    let query_tokens: HashSet<String> = query_tokens_for_recall(query).into_iter().collect();
    if query_tokens.is_empty() {
        return 0.0;
    }
    let endpoint_tokens: HashSet<String> =
        tokenize_for_recall(&format!("{} {}", claim.subject, claim.object))
            .into_iter()
            .collect();
    if endpoint_tokens.is_empty() {
        return 0.0;
    }
    let overlap = query_tokens.intersection(&endpoint_tokens).count() as f64;
    overlap / query_tokens.len() as f64
}

fn query_tokens_for_recall(query: &str) -> Vec<String> {
    let mut tokens = tokenize_for_recall(query);
    if tokens.len() >= 3 {
        let acronym: String = tokens
            .iter()
            .filter_map(|token| token.chars().next())
            .collect();
        if acronym.len() >= 2 && !tokens.contains(&acronym) {
            tokens.push(acronym);
        }
    }
    tokens
}

fn tokenize_for_recall(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .flat_map(camel_case_parts)
        .map(|token| normalize_recall_token(&token))
        .collect()
}

fn camel_case_parts(token: &str) -> Vec<String> {
    if token
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        let mut parts = vec![token.to_string()];
        if let Some(split) = token.find(|ch: char| ch.is_ascii_digit())
            && split > 0
        {
            parts.push(token[..split].to_string());
            let digits = &token[split..];
            parts.push(digits.to_string());
            if digits.len() > 1 {
                parts.extend(digits.chars().map(|digit| digit.to_string()));
            }
        }
        return parts;
    }

    let mut parts = Vec::new();
    let mut start = 0;
    for (idx, ch) in token.char_indices().skip(1) {
        if ch.is_ascii_uppercase() {
            parts.push(token[start..idx].to_string());
            start = idx;
        }
    }
    parts.push(token[start..].to_string());
    let base_parts = parts.clone();
    for part in &base_parts {
        if let Some(split) = part.find(|ch: char| ch.is_ascii_digit())
            && split > 0
        {
            parts.push(part[..split].to_string());
            let digits = &part[split..];
            parts.push(digits.to_string());
            if digits.len() > 1 {
                parts.extend(digits.chars().map(|digit| digit.to_string()));
            }
        }
    }
    if base_parts.len() >= 2 {
        let acronym: String = base_parts
            .iter()
            .filter_map(|part| part.chars().next())
            .collect();
        parts.push(acronym);
    }
    parts
}

fn normalize_recall_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if lower == "children" {
        "child".to_string()
    } else if lower == "people" {
        "person".to_string()
    } else if lower.len() > 4 && lower.ends_with("ee") {
        lower.trim_end_matches("ee").to_string()
    } else if lower.len() > 4 && lower.ends_with("ies") {
        format!("{}y", lower.trim_end_matches("ies"))
    } else if lower.len() > 3 && lower.ends_with('s') {
        lower.trim_end_matches('s').to_string()
    } else {
        lower
    }
}

fn recall_token_score(query_tokens: &[String], claim: &Claim) -> usize {
    let claim_text = claim.text();
    let exact_text = claim_text.to_ascii_lowercase();
    let exact_tokens: HashSet<String> = exact_text
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let sub_tokens: HashSet<String> = tokenize_for_recall(&claim_text).into_iter().collect();
    query_tokens
        .iter()
        .map(|token| {
            if exact_tokens.contains(token) {
                2
            } else if sub_tokens.contains(token) {
                1
            } else {
                0
            }
        })
        .sum()
}

fn validate_vector_chunk_text(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidVectorChunkText)
    } else {
        Ok(())
    }
}

fn validate_embedding_model(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidEmbeddingModel)
    } else {
        Ok(())
    }
}

fn validate_embedding_vector(value: &[f32]) -> Result<(), Error> {
    if value.is_empty() || value.iter().any(|component| !component.is_finite()) {
        Err(Error::InvalidEmbeddingVector)
    } else {
        Ok(())
    }
}

fn validate_top_k(top_k: usize) -> Result<(), Error> {
    if top_k == 0 {
        Err(Error::InvalidTopK)
    } else {
        Ok(())
    }
}

fn validate_claim_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidClaimField { field })
    } else {
        Ok(())
    }
}

fn validate_contradiction_reason(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidContradictionReason)
    } else {
        Ok(())
    }
}

fn validate_candidate_status_filter(value: &str) -> Result<(), Error> {
    match value {
        "PENDING" | "PROMOTED" | "REJECTED" => Ok(()),
        _ => Err(Error::InvalidCandidateStatusFilter {
            status: value.to_string(),
        }),
    }
}

fn validate_recall_query(value: &str) -> Result<(), Error> {
    if query_tokens_for_recall(value).is_empty() {
        Err(Error::InvalidRecallQuery)
    } else {
        Ok(())
    }
}

fn validate_rejection_reason(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidRejectionReason)
    } else {
        Ok(())
    }
}

fn validate_event_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidEventField { field })
    } else {
        Ok(())
    }
}

fn validate_observation_field(field: &'static str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidObservationField { field })
    } else {
        Ok(())
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
struct ObservationLogEntry<'a> {
    kind: &'a str,
    ts: i64,
    observation_id: &'a str,
    session_id: &'a str,
    content: &'a str,
    relevance: &'a str,
    source_event_ids: &'a [i64],
    agent_id: &'a str,
    agent_kind: &'a str,
    derivation: &'a str,
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
    confidence: f64,
}

fn observation_id(session_id: &str, content: &str, source_event_ids: &[i64]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in session_id
        .as_bytes()
        .iter()
        .chain([0xff].iter())
        .chain(content.as_bytes().iter())
        .chain([0xfe].iter())
        .chain(
            source_event_ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .collect::<Vec<_>>()
                .iter(),
        )
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")[..12].to_string()
}

fn seed_ontology(conn: &Connection) -> Result<(), Error> {
    seed_type_table(conn, "entity_types", ENTITY_ONTOLOGY)?;
    seed_type_table(conn, "predicate_types", PREDICATE_ONTOLOGY)?;
    rebuild_closure(conn, "entity_types", "entity_type_closure")?;
    rebuild_closure(conn, "predicate_types", "predicate_closure")?;
    Ok(())
}

fn seed_type_table(
    conn: &Connection,
    table: &str,
    ontology: &[(&str, Option<&str>)],
) -> Result<(), Error> {
    for (name, _parent) in ontology {
        conn.execute(
            &format!("INSERT OR IGNORE INTO {table} (name) VALUES (?1)"),
            [name],
        )?;
    }
    for (name, parent) in ontology {
        let parent_id = if let Some(parent) = parent {
            conn.query_row(
                &format!("SELECT id FROM {table} WHERE name = ?1"),
                [parent],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        } else {
            None
        };
        conn.execute(
            &format!("UPDATE {table} SET parent_id = ?2 WHERE name = ?1"),
            params![name, parent_id],
        )?;
    }
    Ok(())
}

fn rebuild_closure(conn: &Connection, type_table: &str, closure_table: &str) -> Result<(), Error> {
    conn.execute(&format!("DELETE FROM {closure_table}"), [])?;
    let mut stmt = conn.prepare(&format!("SELECT id, parent_id FROM {type_table}"))?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let mut parents = HashMap::new();
    for row in rows {
        let (id, parent_id) = row?;
        parents.insert(id, parent_id);
    }
    for child_id in parents.keys().copied() {
        let mut ancestor = Some(child_id);
        while let Some(ancestor_id) = ancestor {
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO {closure_table} (child_id, ancestor_id) VALUES (?1, ?2)"
                ),
                params![child_id, ancestor_id],
            )?;
            ancestor = parents.get(&ancestor_id).copied().flatten();
        }
    }
    Ok(())
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
    #[error("invalid vector chunk text: must not be empty")]
    InvalidVectorChunkText,
    #[error("invalid embedding model: must not be empty")]
    InvalidEmbeddingModel,
    #[error("invalid embedding vector: must not be empty")]
    InvalidEmbeddingVector,
    #[error("invalid claim {field}: must not be empty")]
    InvalidClaimField { field: &'static str },
    #[error("invalid event {field}: must not be empty")]
    InvalidEventField { field: &'static str },
    #[error("invalid observation {field}: must not be empty")]
    InvalidObservationField { field: &'static str },
    #[error("invalid recall query: must not be empty")]
    InvalidRecallQuery,
    #[error("invalid top_k: must be greater than zero")]
    InvalidTopK,
    #[error("invalid graph entity: must not be empty")]
    InvalidGraphEntity,
    #[error("invalid graph hops: must be greater than zero")]
    InvalidGraphHops,
    #[error("invalid predicate filter: must not be empty")]
    InvalidPredicateFilter,
    #[error("invalid event threshold: must be greater than zero")]
    InvalidEventThreshold,
    #[error("invalid rejection reason: must not be empty")]
    InvalidRejectionReason,
    #[error("invalid contradiction reason: must not be empty")]
    InvalidContradictionReason,
    #[error("invalid confidence value: {value}")]
    InvalidConfidence { value: f64 },
    #[error("invalid decay tau: {value}")]
    InvalidDecayTau { value: f64 },
    #[error("candidate claim must cite an existing event: {event_id}")]
    MissingEventProvenance { event_id: i64 },
    #[error("missing event: event {event_id} does not exist")]
    MissingEvent { event_id: i64 },
    #[error("missing candidate claim: candidate {candidate_id} does not exist")]
    MissingCandidate { candidate_id: i64 },
    #[error("missing observation: observation {observation_id} does not exist")]
    MissingObservation { observation_id: String },
    #[error("invalid candidate claim status for candidate {candidate_id}: {status}")]
    InvalidCandidateStatus { candidate_id: i64, status: String },
    #[error("invalid candidate status filter: {status}")]
    InvalidCandidateStatusFilter { status: String },
    #[error("missing entity: {entity}")]
    MissingEntity { entity: String },
    #[error("missing claim: claim {claim_id} does not exist")]
    MissingClaim { claim_id: i64 },
    #[error("missing vector chunk: vector chunk {chunk_id} does not exist")]
    MissingVectorChunk { chunk_id: i64 },
}
