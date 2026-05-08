use std::path::PathBuf;

use aver_core::{AgentKind, ObservationRelevance, Store};
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
    },
    /// Search active claims by keyword.
    Recall { query: String },

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
    },

    /// Recall an observation and its supporting events by observation ID.
    RecallObservation {
        #[arg(long)]
        id: String,
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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
        } => {
            let claim_id = store.add_claim(&subject, &predicate, &object, &source)?;
            println!("claim_id={claim_id}");
        }

        Command::Recall { query } => {
            for claim in store.recall_text(&query)? {
                println!("{} {} {}", claim.subject, claim.predicate, claim.object);
            }
        }

        Command::RecordEvent {
            session_id,
            kind,
            payload,
            source,
        } => {
            let source = source.as_deref().unwrap_or("cli");
            let event_id = store.record_event_from_agent(
                "cli",
                AgentKind::Human,
                &session_id,
                &kind,
                &payload,
                source,
            )?;
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
        } => {
            let candidate_id =
                store.propose_candidate_claim(event_id, &subject, &predicate, &object)?;
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
        } => {
            let relevance: ObservationRelevance = relevance.parse()?;
            let ids: Vec<i64> = source_event_ids
                .split(',')
                .map(|s| s.trim().parse::<i64>())
                .collect::<Result<_, _>>()
                .map_err(|err| anyhow::anyhow!("invalid event id: {err}"))?;
            let id =
                store.record_observation(&session_id, &content, relevance, &ids, &derivation)?;
            println!("observation_id={id}");
        }

        Command::RecallObservation { id } => {
            let recall = store.recall_observation(&id)?;
            println!("observation={}", recall.observation.content);
            for event in recall.events {
                println!("  event_id={} kind={}", event.id, event.kind);
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
        } => {
            let predicate_refs: Option<Vec<String>> = predicates
                .as_ref()
                .map(|p| p.split(',').map(|s| s.trim().to_string()).collect());
            let pred_slice: Option<Vec<&str>> = predicate_refs
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect());
            let graph = store.expand(&entity, hops, pred_slice.as_deref())?;
            println!("nodes={}", graph.nodes.join(","));
            for edge in graph.edges {
                println!("  {} {} {}", edge.subject, edge.predicate, edge.object);
            }
        }

        Command::AddTriple {
            subject,
            predicate,
            object,
            source,
            confidence,
        } => {
            let id = store.add_claim_with_confidence(
                &subject,
                &predicate,
                &object,
                &source,
                confidence.unwrap_or(0.95),
            )?;
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
    }

    Ok(())
}
