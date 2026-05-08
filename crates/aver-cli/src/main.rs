use std::path::PathBuf;

use aver_core::Store;
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Status => {
            Store::open(&cli.memory_dir)?;
            println!("aver store: ok");
        }
        Command::Remember {
            subject,
            predicate,
            object,
            source,
        } => {
            let store = Store::open(&cli.memory_dir)?;
            let claim_id = store.add_claim(&subject, &predicate, &object, &source)?;
            println!("claim_id={claim_id}");
        }
        Command::Recall { query } => {
            let store = Store::open(&cli.memory_dir)?;
            for claim in store.recall_text(&query)? {
                println!("{} {} {}", claim.subject, claim.predicate, claim.object);
            }
        }
    }

    Ok(())
}
