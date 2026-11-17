use std::{env, process};

use anyhow::Context as _;

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
    let mut ops_reports = Vec::new();
    for fixture_path in fixture_paths {
        let fixture_json = std::fs::read_to_string(&fixture_path)
            .with_context(|| format!("read fixture {fixture_path}"))?;
        // Ops fixtures declare their harness so they cannot be scored
        // silently by the keyword runner (their extra fields would parse).
        let is_ops = serde_json::from_str::<serde_json::Value>(&fixture_json)
            .ok()
            .and_then(|value| value.get("harness")?.as_str().map(str::to_owned))
            .as_deref()
            == Some("graphfirst");
        if is_ops {
            let fixture = aver_eval::graphfirst::load_ops_fixture(&fixture_json)?;
            ops_reports.push(serde_json::to_value(
                aver_eval::graphfirst::run_ops_fixture(&fixture)?,
            )?);
            continue;
        }
        let fixture = aver_eval::load_fixture(&fixture_json)?;
        metrics.push(aver_eval::run_fixture(&fixture)?);
    }

    let keyword = match metrics.len() {
        0 => None,
        1 => Some(serde_json::to_value(&metrics[0])?),
        _ => Some(serde_json::to_value(aver_eval::aggregate_metrics(metrics))?),
    };
    // One JSON document on stdout: a lone report as is, several under
    // "graphfirst" (reports) and "keyword" (metrics or their aggregate).
    let document = match (ops_reports.len(), keyword) {
        (0, None) => return Ok(()),
        (0, Some(keyword)) => keyword,
        (1, None) => ops_reports.remove(0),
        (_, keyword) => {
            let mut document = serde_json::json!({ "graphfirst": ops_reports });
            if let Some(keyword) = keyword {
                document["keyword"] = keyword;
            }
            document
        }
    };
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}
