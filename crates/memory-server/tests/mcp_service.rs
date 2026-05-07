use memory_server::mcp::AmlMcpService;
use rmcp::ServerHandler;

#[test]
fn mcp_service_advertises_tools_capability() {
    let dir = tempfile::tempdir().unwrap();
    let service = AmlMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();

    let info = service.get_info();

    assert_eq!(info.server_info.name, "agent-memory-layer");
    assert!(info.instructions.unwrap().contains("remember_claim"));
}
