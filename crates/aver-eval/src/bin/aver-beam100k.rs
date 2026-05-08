use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Run BEAM 100K against Aver with local Ollama answer + judge")]
struct Args {
    #[arg(long)]
    dataset: Option<PathBuf>,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_base_url: String,
    #[arg(long, default_value = "nomic-embed-text")]
    embedding_model: String,
    #[arg(long, default_value = "gemma4")]
    generation_model: String,
    #[arg(long, default_value_t = 12)]
    top_k: usize,
    #[arg(long)]
    limit_conversations: Option<usize>,
    #[arg(long)]
    limit_questions: Option<usize>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let dataset_path = aver_eval::beam::resolve_dataset_path(args.dataset)?;
    let report = aver_eval::beam::run_beam100k(aver_eval::beam::BeamRunConfig {
        dataset_path,
        ollama_base_url: args.ollama_base_url,
        embedding_model: args.embedding_model,
        generation_model: args.generation_model,
        top_k: args.top_k,
        limit_conversations: args.limit_conversations,
        limit_questions: args.limit_questions,
        data_dir: args.data_dir,
    })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
