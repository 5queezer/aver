use std::{env, process};

fn main() {
    if let Err(err) = run() {
        eprintln!("aver-eval: {err}");
        process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let fixture_paths: Vec<String> = env::args().skip(1).collect();
    if fixture_paths.is_empty() {
        return Err(anyhow::anyhow!(
            "usage: aver-eval <fixture.json> [fixture.json ...]"
        ));
    }

    let mut metrics = Vec::new();
    for fixture_path in fixture_paths {
        let fixture_json = std::fs::read_to_string(fixture_path)?;
        let fixture = aver_eval::load_fixture(&fixture_json)?;
        metrics.push(aver_eval::run_fixture(&fixture)?);
    }

    if metrics.len() == 1 {
        println!("{}", serde_json::to_string_pretty(&metrics[0])?);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&aver_eval::aggregate_metrics(metrics))?
        );
    }
    Ok(())
}
