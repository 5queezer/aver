use aver_server::mcp::AverMcpService;
use rmcp::ServerHandler;

#[test]
fn mcp_service_advertises_tools_capability() {
    let dir = tempfile::tempdir().unwrap();
    let service = AverMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();

    let info = service.get_info();

    assert_eq!(info.server_info.name, "aver");
    let instructions = info.instructions.unwrap();
    for tool_name in AverMcpService::TOOLS.iter().map(|tool| tool.name) {
        assert!(
            instructions.contains(tool_name),
            "MCP instructions should advertise {tool_name}"
        );
    }
}
