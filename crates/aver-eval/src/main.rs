use std::{env, process};

fn main() {
    if let Err(err) = run() {
        eprintln!("aver-eval: {err}");
        process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let fixture_path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: aver-eval <fixture.json>"))?;
    let fixture_json = std::fs::read_to_string(fixture_path)?;
    let fixture = aver_eval::load_fixture(&fixture_json)?;
    let metrics = aver_eval::run_fixture(&fixture)?;
    println!("{}", serde_json::to_string_pretty(&metrics)?);
    Ok(())
}
