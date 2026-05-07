use memory_server::config::ServerConfig;

#[test]
fn server_config_reads_aml_environment() {
    unsafe {
        std::env::set_var("AML_HOST", "127.0.0.1");
        std::env::set_var("AML_PORT", "3317");
        std::env::set_var("AML_BASE_URL", "http://127.0.0.1:3317");
        std::env::set_var("AML_MEMORY_DIR", "/tmp/aml-memory-test");
        std::env::set_var("AML_AUTH_DB_PATH", "/tmp/aml-auth-test.db");
    }

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3317);
    assert_eq!(config.base_url, "http://127.0.0.1:3317");
    assert_eq!(config.memory_dir, "/tmp/aml-memory-test");
    assert_eq!(config.auth_db_path, "/tmp/aml-auth-test.db");
}
