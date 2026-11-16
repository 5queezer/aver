use aver_core::{CandidateWrite, ClaimWrite, EventWrite, RecallQuery, ReverifyWrite};
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use aver_core::{AgentKind, ReplayMode, ScopeWalk, Store, replay_with_mode, vacuum};
use aver_extractor::LANGUAGES;
use aver_extractor::bridge::{TreeExtraction, extract_tree};
use clap::{Parser, Subcommand};

/// Agent id recorded on claims written by `aver extract`.
const EXTRACT_AGENT_ID: &str = "aver_extract";

#[derive(Debug, Parser)]
#[command(name = "aver", version, about = "Aver local-first claim memory CLI")]
struct Cli {
    /// Directory containing db.sqlite and log.jsonl.
    #[arg(long, default_value = ".aver")]
    memory_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

/// ADR-0032 claim validity flags shared by `remember` and `add-triple`.
#[derive(Debug, clap::Args)]
struct ValidityArgs {
    /// Context entity the claim was established under.
    #[arg(long)]
    context: Option<String>,
    /// Start of the evidence window (Unix seconds); needs --context.
    #[arg(long)]
    valid_from: Option<i64>,
    /// End of the evidence window (Unix seconds); needs --context.
    #[arg(long)]
    valid_until: Option<i64>,
}

impl ValidityArgs {
    fn apply(self, write: ClaimWrite) -> ClaimWrite {
        let write = match self.context {
            Some(context) => write.context(context),
            None => write,
        };
        write.valid_window(self.valid_from, self.valid_until)
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the store and report readiness.
    Status,
    /// Append a user-asserted claim.
    Remember {
        subject: String,
        predicate: String,
        object: String,
        #[arg(long)]
        source: String,
        /// ADR-0021 memory scope. Defaults to "global".
        #[arg(long)]
        scope: Option<String>,
        #[command(flatten)]
        validity: ValidityArgs,
    },
    /// Search active claims by keyword.
    Recall {
        query: String,
        /// ADR-0021 memory scope filter. Defaults to "global" + walk=any.
        #[arg(long)]
        scope: Option<String>,
        /// Walk mode: exact | ancestors | descendants | any.
        #[arg(long)]
        scope_walk: Option<String>,
        /// ADR-0032: only claims established under this context entity.
        #[arg(long)]
        context: Option<String>,
    },
    /// Confirm a claim with new evidence (ADR-0032 explicit reverify).
    Reverify {
        #[arg(long)]
        claim_id: i64,
        /// Evidence reference (repeatable): event:<id>, claim:<id> or an item id.
        #[arg(long = "evidence", required = true)]
        evidence: Vec<String>,
        /// Extend the claim's window end (Unix seconds); never shrinks it.
        #[arg(long)]
        valid_until: Option<i64>,
        /// The claim's scope. Defaults to "global".
        #[arg(long)]
        scope: Option<String>,
        #[arg(long, default_value = "local")]
        agent_id: String,
        /// HUMAN, LLM, DETERMINISTIC_PARSER or EXTERNAL_TOOL.
        #[arg(long, default_value = "HUMAN")]
        agent_kind: String,
    },
    /// List every explicit confirmation of a claim, oldest first.
    Confirmations {
        #[arg(long)]
        claim_id: i64,
    },

    /// Record an episodic event from the agent.
    RecordEvent {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        payload: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },

    /// Check whether a session has enough events to trigger memory extraction.
    ShouldExtractMemories {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        event_threshold: usize,
    },

    /// Propose a candidate claim from a recorded event.
    Propose {
        #[arg(long)]
        event_id: i64,
        subject: String,
        predicate: String,
        object: String,
        #[arg(long)]
        scope: Option<String>,
    },

    /// List candidate claims, optionally filtered by session and/or status.
    ListCandidates {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },

    /// Promote a candidate claim to durable memory.
    Promote {
        #[arg(long)]
        candidate_id: i64,
    },

    /// Reject a candidate claim with a reason.
    Reject {
        #[arg(long)]
        candidate_id: i64,
        #[arg(long)]
        reason: String,
    },

    /// Expand an entity into its local claim-graph neighborhood.
    Expand {
        entity: String,
        #[arg(long, default_value = "2")]
        hops: usize,
        /// Comma-separated predicate names.
        #[arg(long)]
        predicates: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_walk: Option<String>,
    },

    /// Append a structured triple with an explicit confidence.
    AddTriple {
        subject: String,
        predicate: String,
        object: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        confidence: Option<f64>,
        #[arg(long)]
        scope: Option<String>,
        #[command(flatten)]
        validity: ValidityArgs,
    },

    /// Record a contradiction for an existing claim.
    Contradict {
        #[arg(long)]
        triple_id: i64,
        #[arg(long)]
        reason: String,
    },

    /// Run the on-demand consolidation pass.
    Consolidate,

    /// Embed stored vector chunks that do not have an embedding yet.
    Backfill,

    /// Compact the SQLite database (ADR-0019 §2).
    Vacuum {
        /// Run ANALYZE after vacuum to refresh planner stats.
        #[arg(long)]
        analyze: bool,
        /// Vacuum into a copy at PATH instead of in-place.
        #[arg(long)]
        into: Option<PathBuf>,
    },

    /// Extract code facts with Tree-sitter (ADR-0007) and store them as
    /// EXTRACTED claims in the project scope.
    Extract {
        /// Source file or directory to scan.
        path: PathBuf,
        /// ADR-0021 memory scope. Defaults to `proj/<repo name>`.
        #[arg(long)]
        scope: Option<String>,
        /// Print fact counts per predicate without opening the store.
        #[arg(long)]
        dry_run: bool,
        /// Only scan files of this language (rust, python, typescript, ...).
        #[arg(long)]
        lang: Option<String>,
    },

    /// Reclassify legacy prose/scalar objects and prune unreferenced object entities.
    MigrateLiterals {
        /// Preview counts without appending migration records.
        #[arg(long)]
        dry_run: bool,
    },

    /// Rebuild db.sqlite from the JSONL log (ADR-0019 §4).
    Replay {
        /// Allow overwriting an existing populated db.sqlite.
        #[arg(long)]
        force: bool,
        /// Quarantine invalid log lines and continue instead of aborting
        /// (disaster recovery; strict by default).
        #[arg(long)]
        lenient: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Vacuum and replay are operational tools that must run without an open
    // Store: vacuum needs the exclusive lock, replay rebuilds db.sqlite from
    // logs. Dispatch them before opening the store.
    match &cli.command {
        Command::Vacuum { analyze, into } => {
            let report = vacuum(&cli.memory_dir, into.as_deref(), *analyze)?;
            println!(
                "vacuum: pages {}->{}, freelist {}->{}",
                report.pages_before,
                report.pages_after,
                report.freelist_before,
                report.freelist_after
            );
            if let Some(path) = report.vacuumed_into {
                println!("into: {}", path.display());
            }
            return Ok(());
        }
        Command::Replay { force, lenient } => {
            let mode = if *lenient {
                ReplayMode::Lenient
            } else {
                ReplayMode::Strict
            };
            let report = replay_with_mode(&cli.memory_dir, *force, mode)?;
            println!(
                "replay: claims={} hyperedges={} events={} observations={} lifecycle={} files={} quarantined={}",
                report.claims,
                report.hyperedges,
                report.events,
                report.observations,
                report.lifecycle,
                report.files_walked,
                report.quarantined.len()
            );
            for quarantined in &report.quarantined {
                eprintln!(
                    "quarantined: {}:{}: {}",
                    quarantined.path, quarantined.line, quarantined.error
                );
            }
            return Ok(());
        }
        // Extract opens the store only for a real run, inside
        // `ingest_extracted`, so `--dry-run` stays free of store calls.
        Command::Extract {
            path,
            scope,
            dry_run,
            lang,
        } => {
            return run_extract(
                &cli.memory_dir,
                path,
                scope.as_deref(),
                *dry_run,
                lang.as_deref(),
            );
        }
        _ => {}
    }

    let store = Store::open(&cli.memory_dir)?;

    match cli.command {
        Command::MigrateLiterals { dry_run } => {
            let report = if dry_run {
                store.preview_literal_migration()?
            } else {
                store.migrate_literals()?
            };
            println!(
                "{{\"claims_reclassified\":{},\"entities_removed\":{}}}",
                report.claims_reclassified, report.entities_removed
            );
        }
        Command::Status => {
            println!("aver store: ok");
        }

        Command::Remember {
            subject,
            predicate,
            object,
            source,
            scope,
            validity,
        } => {
            let write = ClaimWrite::new(&subject, &predicate, &object, &source)
                .scope(scope.as_deref().unwrap_or("global"));
            let claim_id = store.add_claim(validity.apply(write))?;
            println!("claim_id={claim_id}");
        }

        Command::Reverify {
            claim_id,
            evidence,
            valid_until,
            scope,
            agent_id,
            agent_kind,
        } => {
            let mut write = ReverifyWrite::new(claim_id, evidence)
                .agent(agent_id, agent_kind.parse()?)
                .scope(scope.as_deref().unwrap_or("global"));
            if let Some(valid_until) = valid_until {
                write = write.valid_until(valid_until);
            }
            let verified_at = store.reverify_claim(write)?;
            println!("claim_id={claim_id} verified_at={verified_at}");
        }

        Command::Confirmations { claim_id } => {
            store.get_claim(claim_id)?;
            for verification in store.claim_verifications(claim_id)? {
                println!(
                    "verified_at={} agent_id={} agent_kind={} evidence={}",
                    verification.verified_at,
                    verification.agent_id,
                    verification.agent_kind.as_str(),
                    verification.evidence.join(",")
                );
            }
        }

        Command::Recall {
            query,
            scope,
            scope_walk,
            context,
        } => {
            let walk: ScopeWalk = match scope_walk.as_deref() {
                None => {
                    if scope.is_some() {
                        ScopeWalk::Ancestors
                    } else {
                        ScopeWalk::Any
                    }
                }
                Some(s) => s.parse()?,
            };
            let scope = scope.as_deref().unwrap_or("global");
            let mut recall = RecallQuery::new(&query).scope(scope, walk);
            recall.filters.context = context;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64);
            for claim in store.recall_text(recall)? {
                let confirmation = store.claim_confirmation(&claim, now)?;
                let mut line = format!(
                    "[{}] {} {} {}",
                    claim.scope, claim.subject, claim.predicate, claim.object
                );
                if let Some(context) = &claim.context {
                    line.push_str(&format!(" context={context}"));
                }
                if let Some(count) = confirmation.evidence_since_confirmation {
                    line.push_str(&format!(
                        " last_confirmed_at={} evidence_since_confirmation={count}",
                        confirmation.last_confirmed_at
                    ));
                }
                println!("{line}");
            }
        }

        Command::RecordEvent {
            session_id,
            kind,
            payload,
            source,
            scope,
        } => {
            let source = source.as_deref().unwrap_or("cli");
            let event_id = match scope.as_deref() {
                None => store.record_event(
                    EventWrite::new(&session_id, &kind, &payload, source)
                        .agent("cli", AgentKind::Human),
                )?,
                Some(scope) => store.record_event(
                    EventWrite::new(&session_id, &kind, &payload, source)
                        .agent("cli", AgentKind::Human)
                        .scope(scope),
                )?,
            };
            println!("event_id={event_id}");
        }

        Command::ShouldExtractMemories {
            session_id,
            event_threshold,
        } => {
            let result = store.should_extract_memories(&session_id, event_threshold)?;
            println!("should_extract={result}");
        }

        Command::Propose {
            event_id,
            subject,
            predicate,
            object,
            scope,
        } => {
            let candidate_id = match scope.as_deref() {
                None => store.propose_candidate_claim(CandidateWrite::new(
                    event_id, &subject, &predicate, &object,
                ))?,
                Some(scope) => store.propose_candidate_claim(
                    CandidateWrite::new(event_id, &subject, &predicate, &object).scope(scope),
                )?,
            };
            println!("candidate_id={candidate_id}");
        }

        Command::ListCandidates { session_id, status } => {
            let candidates =
                store.list_candidate_claims(session_id.as_deref(), status.as_deref())?;
            for candidate in candidates {
                println!(
                    "id={} status={} subject={} predicate={} object={}",
                    candidate.id,
                    candidate.status,
                    candidate.subject,
                    candidate.predicate,
                    candidate.object,
                );
            }
        }

        Command::Promote { candidate_id } => {
            let claim_id = store.promote_candidate_claim(candidate_id)?;
            println!("claim_id={claim_id}");
        }

        Command::Reject {
            candidate_id,
            reason,
        } => {
            store.reject_candidate_claim(candidate_id, &reason)?;
            println!("rejected");
        }

        Command::Expand {
            entity,
            hops,
            predicates,
            scope,
            scope_walk,
        } => {
            let predicate_refs: Option<Vec<String>> = predicates
                .as_ref()
                .map(|p| p.split(',').map(|s| s.trim().to_string()).collect());
            let pred_slice: Option<Vec<&str>> = predicate_refs
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect());
            let walk: ScopeWalk = match scope_walk.as_deref() {
                None => {
                    if scope.is_some() {
                        ScopeWalk::Ancestors
                    } else {
                        ScopeWalk::Any
                    }
                }
                Some(s) => s.parse()?,
            };
            let scope = scope.as_deref().unwrap_or("global");
            let graph =
                store.expand_with_scope(&entity, hops, pred_slice.as_deref(), scope, walk)?;
            println!("nodes={}", graph.nodes.join(","));
            for edge in graph.edges {
                println!(
                    "  [{}] {} {} {}",
                    edge.scope, edge.subject, edge.predicate, edge.object
                );
            }
        }

        Command::AddTriple {
            subject,
            predicate,
            object,
            source,
            confidence,
            scope,
            validity,
        } => {
            let write = ClaimWrite::new(&subject, &predicate, &object, &source)
                .confidence(confidence.unwrap_or(0.95))
                .scope(scope.as_deref().unwrap_or("global"));
            let id = store.add_claim(validity.apply(write))?;
            println!("triple_id={id}");
        }

        Command::Contradict { triple_id, reason } => {
            store.contradict(triple_id, &reason, None)?;
            println!("contradicted");
        }

        Command::Consolidate => {
            let report = store.consolidate_report()?;
            println!(
                "merged={} superseded={} decayed={}",
                report.merged, report.superseded, report.decayed
            );
        }

        Command::Backfill => {
            let (embedder, _) = aver_core::vector::embedding_from_env();
            let Some(embedder) = embedder else {
                anyhow::bail!(
                    "no embedding provider configured; set AVER_EMBEDDING_URL or configure AVER_EMBEDDING_PROVIDER=openai"
                );
            };
            let (embedded, indexed, total) = backfill_counts(&store, embedder.as_ref())?;
            println!("backfill: embedded={embedded} indexed={indexed} total={total}");
        }

        Command::Vacuum { .. } | Command::Replay { .. } | Command::Extract { .. } => {
            unreachable!("vacuum, replay and extract are dispatched before opening the store")
        }
    }

    store.close()?;
    Ok(())
}

fn backfill_counts(
    store: &Store,
    embedder: &dyn aver_core::vector::EmbeddingClient,
) -> anyhow::Result<(usize, usize, usize)> {
    let embedded = store.backfill_vector_embeddings(embedder)?;
    let (indexed, total) = store.vector_chunk_embedding_status()?;
    Ok((embedded, indexed, total))
}

#[cfg(test)]
mod backfill_tests {
    use super::*;
    use aver_core::vector::MockEmbeddingClient;

    #[test]
    fn backfill_command_fills_missing_vectors_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "aver",
            "--memory-dir",
            dir.path().to_str().unwrap(),
            "backfill",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Backfill));

        let store = Store::open(dir.path()).unwrap();
        let claim_id = store
            .add_claim(ClaimWrite::new("aver", "stores", "vectors", "test"))
            .unwrap();
        store
            .add_vector_chunk(claim_id, "aver stores vectors", "test-embedding")
            .unwrap();
        let embedder = MockEmbeddingClient::new(vec![1.0, 0.0, 0.0]);

        assert_eq!(backfill_counts(&store, &embedder).unwrap(), (1, 1, 1));
        assert_eq!(backfill_counts(&store, &embedder).unwrap(), (0, 1, 1));
    }
}

fn run_extract(
    memory_dir: &Path,
    path: &Path,
    scope: Option<&str>,
    dry_run: bool,
    lang: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(lang) = lang
        && !LANGUAGES.contains(&lang)
    {
        anyhow::bail!(
            "unknown --lang {lang}; expected one of {}",
            LANGUAGES.join(", ")
        );
    }
    let scope = match scope {
        Some(scope) => scope.to_string(),
        None => derive_project_scope(path)?,
    };
    let extraction = extract_tree(path, lang)?;
    for (file, err) in &extraction.failed {
        eprintln!("warning: could not extract {file}: {err}");
    }
    let unsupported = extraction
        .skipped
        .iter()
        .map(|(predicate, count)| format!("{predicate}={count}"))
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        eprintln!(
            "warning: skipped facts with predicates outside the core ontology: {}",
            unsupported.join(" ")
        );
    }

    let (ingested, privacy) = if dry_run {
        for (predicate, count) in extraction.facts_by_predicate() {
            println!("{predicate}={count}");
        }
        (0, 0)
    } else {
        ingest_extracted(memory_dir, &extraction, &scope)?
    };

    let mut skipped = extraction
        .skipped
        .iter()
        .map(|(predicate, count)| (predicate.as_str(), *count))
        .collect::<Vec<_>>();
    skipped.push(("duplicate", extraction.duplicates));
    skipped.push(("privacy", privacy));
    let skipped = skipped
        .iter()
        .map(|(kind, count)| format!("{kind}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "extract{}: scope={scope} files_scanned={} files_failed={} facts_extracted={} facts_ingested={ingested} skipped={skipped}",
        if dry_run { " (dry run)" } else { "" },
        extraction.files_scanned(),
        extraction.failed.len(),
        extraction.extracted,
    );
    Ok(())
}

/// Every store call `aver extract` makes lives here. Returns
/// `(ingested, privacy_rejected)`.
///
/// Phase 1 step 4 (one write API behind `ClaimWrite`) replaces this per-fact
/// loop with the batch `ingest_extractor_facts` once that takes a scope;
/// today it hardcodes `global`. Until then each fact is its own write, so a
/// privacy rejection skips one fact rather than failing a batch, and
/// `ExtractorFact::provenance` is not passed: core derives EXTRACTED
/// (confidence 0.90) from `AgentKind::DeterministicParser`. The privacy gate
/// runs in core on every write.
fn ingest_extracted(
    memory_dir: &Path,
    extraction: &TreeExtraction,
    scope: &str,
) -> anyhow::Result<(usize, usize)> {
    let store = Store::open(memory_dir)?;
    let mut ingested = 0;
    let mut privacy = 0;
    for file in &extraction.files {
        for fact in &file.facts {
            match store.add_claim(
                ClaimWrite::from_agent(
                    EXTRACT_AGENT_ID,
                    AgentKind::DeterministicParser,
                    &fact.subject,
                    &fact.predicate,
                    &fact.object,
                    &file.path,
                )
                .scope(scope),
            ) {
                Ok(_) => ingested += 1,
                Err(aver_core::Error::Privacy(_)) => privacy += 1,
                Err(err) => return Err(err.into()),
            }
        }
    }
    // No automatic `consolidate_report()` here: distinct code facts such as
    // `File:x defines A` and `File:x defines B` are multi-valued and must not
    // be retired merely because they share a subject and predicate.
    store.close()?;
    Ok((ingested, privacy))
}

/// ADR-0021 `proj/<slug>` for the project containing `path`, with the
/// human-readable slug ADR-0025 recommends: the repository name from
/// `remote.origin.url`, else the git toplevel directory name, else the
/// directory name of `path`. Characters outside `[A-Za-z0-9_-]` become `-`.
fn derive_project_scope(path: &Path) -> anyhow::Result<String> {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    };
    let dir = dir.canonicalize()?;
    let name = match git_output(&dir, &["rev-parse", "--show-toplevel"]) {
        Some(toplevel) => git_output(&dir, &["config", "--get", "remote.origin.url"])
            .and_then(|url| repo_name_from_url(&url))
            .or_else(|| dir_name(Path::new(&toplevel))),
        None => dir_name(&dir),
    };
    let slug = name.as_deref().map(slugify).unwrap_or_default();
    if slug.is_empty() {
        anyhow::bail!(
            "cannot derive a project scope for {}; pass --scope",
            path.display()
        );
    }
    Ok(format!("proj/{slug}"))
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Process::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_string())
}

fn dir_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// `git@github.com:o/aver.git`, `https://host/o/aver/` -> `aver`.
fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.rsplit(['/', ':'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_from_common_remote_forms() {
        for (url, expected) in [
            ("git@github.com:5queezer/aver.git", "aver"),
            ("https://github.com/5queezer/aver.git", "aver"),
            ("https://github.com/5queezer/aver/", "aver"),
            ("ssh://git@host:2222/team/my.repo", "my.repo"),
            ("/srv/git/local-repo.git\n", "local-repo"),
        ] {
            assert_eq!(repo_name_from_url(url).as_deref(), Some(expected), "{url}");
        }
        assert_eq!(repo_name_from_url(""), None);
    }

    #[test]
    fn slugify_keeps_the_scope_charset() {
        assert_eq!(slugify("my.repo"), "my-repo");
        assert_eq!(slugify("aver_core-2"), "aver_core-2");
        assert_eq!(slugify("..."), "");
    }
}
