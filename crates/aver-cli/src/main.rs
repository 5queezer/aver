use std::path::PathBuf;

use aver_core::{AgentKind, ObservationRelevance, ScopeWalk, Store, replay, vacuum};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "aver", version, about = "Aver local-first claim memory CLI")]
struct Cli {
    /// Directory containing db.sqlite and log.jsonl.
    #[arg(long, default_value = ".aver")]
    memory_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
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

    /// Record a source-backed episodic observation.
    RecordObservation {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        relevance: String,
        /// Comma-separated event IDs.
        #[arg(long)]
        source_event_ids: String,
        #[arg(long)]
        derivation: String,
        #[arg(long)]
        scope: Option<String>,
    },

    /// Recall an observation and its supporting events by observation ID.
    RecallObservation {
        #[arg(long)]
        id: String,
    },

    /// Report coverage for a session's episodic events.
    ObservationCoverage {
        #[arg(long)]
        session_id: String,
    },

    /// Run a deterministic catch-up pass over uncovered session events.
    CatchUp {
        #[arg(long)]
        session_id: String,
    },

    /// Assemble a compaction summary from current session observations.
    CompactionSummary {
        #[arg(long)]
        session_id: String,
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

    /// Detect deterministic weighted graph communities.
    Communities,

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

    /// Compact the SQLite database (ADR-0019 §2).
    Vacuum {
        /// Run ANALYZE after vacuum to refresh planner stats.
        #[arg(long)]
        analyze: bool,
        /// Vacuum into a copy at PATH instead of in-place.
        #[arg(long)]
        into: Option<PathBuf>,
    },

    /// Rebuild db.sqlite from the JSONL log (ADR-0019 §4).
    Replay {
        /// Allow overwriting an existing populated db.sqlite.
        #[arg(long)]
        force: bool,
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
        Command::Replay { force } => {
            let report = replay(&cli.memory_dir, *force)?;
            println!(
                "replay: claims={} hyperedges={} events={} observations={} files={}",
                report.claims,
                report.hyperedges,
                report.events,
                report.observations,
                report.files_walked
            );
            return Ok(());
        }
        _ => {}
    }

    let store = Store::open(&cli.memory_dir)?;

    match cli.command {
        Command::Status => {
            println!("aver store: ok");
        }

        Command::Remember {
            subject,
            predicate,
            object,
            source,
            scope,
        } => {
            let claim_id = match scope.as_deref() {
                None => store.add_claim(&subject, &predicate, &object, &source)?,
                Some(scope) => {
                    store.add_claim_with_scope(&subject, &predicate, &object, &source, scope)?
                }
            };
            println!("claim_id={claim_id}");
        }

        Command::Recall {
            query,
            scope,
            scope_walk,
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
            for claim in store.recall_text_with_scope(&query, scope, walk)? {
                println!(
                    "[{}] {} {} {}",
                    claim.scope, claim.subject, claim.predicate, claim.object
                );
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
                None => store.record_event_from_agent(
                    "cli",
                    AgentKind::Human,
                    &session_id,
                    &kind,
                    &payload,
                    source,
                )?,
                Some(scope) => store.record_event_from_agent_with_scope(
                    "cli",
                    AgentKind::Human,
                    &session_id,
                    &kind,
                    &payload,
                    source,
                    scope,
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
                None => store.propose_candidate_claim(event_id, &subject, &predicate, &object)?,
                Some(scope) => store.propose_candidate_claim_with_scope(
                    event_id, &subject, &predicate, &object, scope,
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

        Command::RecordObservation {
            session_id,
            content,
            relevance,
            source_event_ids,
            derivation,
            scope,
        } => {
            let relevance: ObservationRelevance = relevance.parse()?;
            let ids: Vec<i64> = source_event_ids
                .split(',')
                .map(|s| s.trim().parse::<i64>())
                .collect::<Result<_, _>>()
                .map_err(|err| anyhow::anyhow!("invalid event id: {err}"))?;
            let id = match scope.as_deref() {
                None => {
                    store.record_observation(&session_id, &content, relevance, &ids, &derivation)?
                }
                Some(scope) => store.record_observation_with_scope(
                    &session_id,
                    &content,
                    relevance,
                    &ids,
                    &derivation,
                    scope,
                )?,
            };
            println!("observation_id={id}");
        }

        Command::RecallObservation { id } => {
            let recall = store.recall_observation(&id)?;
            println!("observation={}", recall.observation.content);
            for event in recall.events {
                println!("  event_id={} kind={}", event.id, event.kind);
            }
        }

        Command::ObservationCoverage { session_id } => {
            let coverage = store.observation_coverage(&session_id)?;
            println!("event_ids={:?}", coverage.event_ids);
            println!("covered_event_ids={:?}", coverage.covered_event_ids);
            println!("uncovered_event_ids={:?}", coverage.uncovered_event_ids);
        }

        Command::CatchUp { session_id } => {
            let coverage = store.observation_coverage(&session_id)?;
            if coverage.uncovered_event_ids.is_empty() {
                println!("catch_up=complete");
            } else {
                let events = store.list_events_for_session(&session_id)?;
                let uncovered_events = events
                    .into_iter()
                    .filter(|event| coverage.uncovered_event_ids.contains(&event.id))
                    .collect::<Vec<_>>();
                let payloads = uncovered_events
                    .iter()
                    .map(|event| format!("{}:{}", event.id, event.payload))
                    .collect::<Vec<_>>()
                    .join(" | ");
                let content = format!(
                    "Catch-up observation for session {session_id}: uncovered event ids {:?}; payloads: {payloads}",
                    coverage.uncovered_event_ids
                );
                let observation_id = store.record_observation(
                    &session_id,
                    &content,
                    ObservationRelevance::Medium,
                    &coverage.uncovered_event_ids,
                    "cli-catch-up",
                )?;
                println!("observation_id={observation_id}");
            }
        }

        Command::CompactionSummary { session_id } => {
            let summary = store.assemble_compaction_summary(&session_id)?;
            print!("{summary}");
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

        Command::Communities => {
            for community in store.detect_communities()? {
                println!(
                    "{} score={:.3} members={} bridges={}",
                    community.id,
                    community.score,
                    community.members.join(","),
                    community.bridge_nodes.join(",")
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
        } => {
            let id = match scope.as_deref() {
                None => store.add_claim_with_confidence(
                    &subject,
                    &predicate,
                    &object,
                    &source,
                    confidence.unwrap_or(0.95),
                )?,
                Some(scope) => store.add_claim_with_confidence_and_scope(
                    &subject,
                    &predicate,
                    &object,
                    &source,
                    confidence.unwrap_or(0.95),
                    scope,
                )?,
            };
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

        Command::Vacuum { .. } | Command::Replay { .. } => {
            unreachable!("vacuum and replay are dispatched before opening the store")
        }
    }

    store.close()?;
    Ok(())
}
