use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Run BEAM 100K against Aver with Ollama or OpenAI answer + judge")]
struct Args {
    #[arg(long)]
    dataset: Option<PathBuf>,
    #[arg(long, default_value = "ollama")]
    provider: aver_eval::beam::BeamProvider,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_base_url: String,
    #[arg(long, default_value = "https://api.openai.com/v1")]
    openai_base_url: String,
    #[arg(long, default_value = "text-embedding-3-small")]
    embedding_model: String,
    #[arg(long, default_value = "gpt-4o-mini")]
    generation_model: String,
    #[arg(long, default_value_t = 12)]
    top_k: usize,
    #[arg(long)]
    retrieval_alpha: Option<f64>,
    #[arg(long)]
    limit_conversations: Option<usize>,
    #[arg(long)]
    limit_questions: Option<usize>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Rubric judge: `generative` (default) or `decision` (ADR-0024, ollama only)
    #[arg(long, default_value = "generative")]
    judge: aver_eval::beam::BeamJudgeMode,
    /// Memory target: `direct` (in-process store, default) or `mcp` (a
    /// running aver-server; ingestion and recall go through its MCP tools)
    #[arg(long, default_value = "direct")]
    target: String,
    /// aver-server base URL for `--target mcp` (`/mcp` is appended)
    #[arg(long, env = "AVER_MCP_URL")]
    mcp_url: Option<String>,
    /// Bearer token with claims:read and claims:write for `--target mcp`
    #[arg(long, env = "AVER_MCP_TOKEN", hide_env_values = true)]
    mcp_token: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let target = match args.target.as_str() {
        "direct" => aver_eval::beam::BeamTarget::Direct,
        "mcp" => aver_eval::beam::BeamTarget::Mcp {
            base_url: args
                .mcp_url
                .ok_or_else(|| anyhow::anyhow!("--target mcp requires --mcp-url"))?,
            token: args
                .mcp_token
                .ok_or_else(|| anyhow::anyhow!("--target mcp requires --mcp-token"))?,
        },
        other => anyhow::bail!("unknown BEAM target: {other} (expected direct or mcp)"),
    };
    let dataset_path = aver_eval::beam::resolve_dataset_path(args.dataset)?;
    let report = aver_eval::beam::run_beam100k(aver_eval::beam::BeamRunConfig {
        dataset_path,
        provider: args.provider,
        ollama_base_url: args.ollama_base_url,
        openai_base_url: args.openai_base_url,
        openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
        embedding_model: args.embedding_model,
        generation_model: args.generation_model,
        top_k: args.top_k,
        retrieval_alpha: args.retrieval_alpha,
        limit_conversations: args.limit_conversations,
        limit_questions: args.limit_questions,
        data_dir: args.data_dir,
        judge_mode: args.judge,
        decision_model: aver_core::decision::DecisionConfig::from_env().model,
        target,
    })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
