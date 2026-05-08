use aver_extractor::{JsonRpcPluginRunner, PluginRequest};

#[test]
fn jsonrpc_plugin_runner_parses_stdout_response_into_facts() {
    let runner = JsonRpcPluginRunner::new("/bin/sh").arg("-c").arg(
        "read request; printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"facts\":[{\"subject\":\"ADR-0013\",\"predicate\":\"permits\",\"object\":\"JSON-RPC plugins\"}]}}'",
    );

    let facts = runner
        .extract(PluginRequest {
            id: 1,
            method: "extract_prose".to_string(),
            text: "ADR-0013 permits JSON-RPC plugin processes.".to_string(),
        })
        .unwrap();

    assert_eq!(facts[0].subject, "ADR-0013");
    assert_eq!(facts[0].predicate, "permits");
    assert_eq!(facts[0].object, "JSON-RPC plugins");
}
