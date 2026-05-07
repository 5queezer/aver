use aver_server::mcp::AverMcpService;
use rmcp::ServerHandler;

#[test]
fn mcp_service_advertises_tools_capability() {
    let dir = tempfile::tempdir().unwrap();
    let service = AverMcpService::open(dir.path(), "http://localhost:3317".to_string()).unwrap();

    let info = service.get_info();

    assert_eq!(info.server_info.name, "aver");
    let instructions = info.instructions.unwrap();
    assert!(instructions.contains("remember_claim"));
    assert!(instructions.contains("record_event"));
    assert!(instructions.contains("promote_candidate_claim"));
}
