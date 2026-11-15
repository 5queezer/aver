//! T101 — v0.8 starts with a checked eval harness descriptor.

#[test]
fn eval_harness_declares_memory_agent_bench() {
    let descriptor = include_str!("../../../eval/memory_agent_bench.json");
    let json: serde_json::Value = serde_json::from_str(descriptor).unwrap();

    assert_eq!(json["name"], "MemoryAgentBench");
    assert_eq!(json["primary_metric"], "mean_recall_at_k");
    assert_eq!(json["version"], 2);
}
