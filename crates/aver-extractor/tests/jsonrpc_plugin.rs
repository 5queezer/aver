use std::time::Duration;

use aver_extractor::{Error, JsonRpcPluginRunner, PluginRequest};

fn request() -> PluginRequest {
    PluginRequest {
        id: 1,
        method: "extract_prose".to_string(),
        text: "ADR-0013 permits JSON-RPC plugin processes.".to_string(),
    }
}

#[test]
fn jsonrpc_plugin_runner_parses_stdout_response_into_facts() {
    let runner = JsonRpcPluginRunner::new("/bin/sh").arg("-c").arg(
        "read request; printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"facts\":[{\"subject\":\"ADR-0013\",\"predicate\":\"permits\",\"object\":\"JSON-RPC plugins\"}]}}'",
    );

    let facts = runner.extract(request()).unwrap();

    assert_eq!(facts[0].subject, "ADR-0013");
    assert_eq!(facts[0].predicate, "permits");
    assert_eq!(facts[0].object, "JSON-RPC plugins");
}

#[test]
fn jsonrpc_plugin_runner_closes_stdin_so_plugins_can_read_to_eof() {
    let runner = JsonRpcPluginRunner::new("/bin/sh").arg("-c").arg(
        "cat > /dev/null; printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"facts\":[{\"subject\":\"ADR-0013\",\"predicate\":\"permits\",\"object\":\"JSON-RPC plugins\"}]}}'",
    );

    let facts = runner.extract(request()).unwrap();

    assert_eq!(facts.len(), 1);
}

#[test]
fn jsonrpc_plugin_runner_times_out_when_plugin_never_responds() {
    let runner = JsonRpcPluginRunner::new("/bin/sh")
        .arg("-c")
        .arg("sleep 5")
        .with_timeout(Duration::from_millis(100));

    let result = runner.extract(request());

    assert!(
        matches!(result, Err(Error::PluginTimeout(_))),
        "expected PluginTimeout, got {result:?}"
    );
}

#[test]
fn jsonrpc_plugin_runner_surfaces_jsonrpc_error_responses() {
    let runner = JsonRpcPluginRunner::new("/bin/sh").arg("-c").arg(
        "read request; printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"method not found\"}}'",
    );

    let result = runner.extract(request());

    assert!(
        matches!(result, Err(Error::PluginError { code: -32601, .. })),
        "expected PluginError, got {result:?}"
    );
}

#[test]
fn jsonrpc_plugin_runner_rejects_mismatched_response_ids() {
    let runner = JsonRpcPluginRunner::new("/bin/sh").arg("-c").arg(
        "read request; printf '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"facts\":[{\"subject\":\"ADR-0013\",\"predicate\":\"permits\",\"object\":\"JSON-RPC plugins\"}]}}'",
    );

    let result = runner.extract(request());

    assert!(
        matches!(
            result,
            Err(Error::PluginIdMismatch {
                expected: 1,
                actual: Some(2),
            })
        ),
        "expected PluginIdMismatch, got {result:?}"
    );
}

#[test]
fn jsonrpc_plugin_runner_rejects_responses_without_result_or_error() {
    let runner = JsonRpcPluginRunner::new("/bin/sh")
        .arg("-c")
        .arg("read request; printf '{\"jsonrpc\":\"2.0\",\"id\":1}'");

    let result = runner.extract(request());

    assert!(
        matches!(result, Err(Error::PluginMissingResult)),
        "expected PluginMissingResult, got {result:?}"
    );
}
