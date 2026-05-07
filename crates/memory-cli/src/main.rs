use std::path::PathBuf;

use clap::{Parser, Subcommand};
use memory_core::Store;

#[derive(Debug, Parser)]
#[command(name = "memory", about = "Local-first agent memory CLI")]
struct Cli {
    /// Directory containing db.sqlite and log.jsonl.
    #[arg(long, default_value = ".memory")]
    memory_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the store and report readiness.
    Status,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Status => {
            Store::open(&cli.memory_dir)?;
            println!("memory store: ok");
        }
    }

    Ok(())
}
