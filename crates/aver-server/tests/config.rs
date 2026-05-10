use std::sync::Mutex;

use aver_server::config::ServerConfig;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const ENV_VARS: &[&str] = &[
    "AVER_HOST",
    "AVER_PORT",
    "AVER_BASE_URL",
    "AVER_MEMORY_DIR",
    "AVER_AUTH_DB_PATH",
    "AVER_CORS_ORIGINS",
    "AVER_LOCAL_AUTHORIZATION_TOKEN",
];

fn clear_env() {
    unsafe {
        for key in ENV_VARS {
            std::env::remove_var(key);
        }
    }
}

#[test]
fn server_config_reads_aver_environment() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_env();
    unsafe {
        std::env::set_var("AVER_HOST", "127.0.0.1");
        std::env::set_var("AVER_PORT", "3317");
        std::env::set_var("AVER_BASE_URL", "http://127.0.0.1:3317");
        std::env::set_var("AVER_MEMORY_DIR", "/tmp/aver-memory-test");
        std::env::set_var("AVER_AUTH_DB_PATH", "/tmp/aver-auth-test.db");
    }

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 3317);
    assert_eq!(config.base_url, "http://127.0.0.1:3317");
    assert_eq!(config.memory_dir, "/tmp/aver-memory-test");
    assert_eq!(config.auth_db_path, "/tmp/aver-auth-test.db");
    assert!(config.cors_origins.is_empty());

    clear_env();
}

#[test]
fn auth_db_path_defaults_under_memory_dir() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_env();
    unsafe {
        std::env::set_var("AVER_MEMORY_DIR", "/tmp/custom");
    }

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(config.memory_dir, "/tmp/custom");
    assert_eq!(config.auth_db_path, "/tmp/custom/auth.db");

    clear_env();
}

#[test]
fn auth_db_path_env_overrides_memory_dir_default() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_env();
    unsafe {
        std::env::set_var("AVER_MEMORY_DIR", "/tmp/custom");
        std::env::set_var("AVER_AUTH_DB_PATH", "/explicit/path.db");
    }

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(config.auth_db_path, "/explicit/path.db");

    clear_env();
}

#[test]
fn auth_db_path_defaults_under_default_memory_dir() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_env();

    let config = ServerConfig::from_env().unwrap();

    assert_eq!(config.memory_dir, ".aver");
    assert_eq!(config.auth_db_path, ".aver/auth.db");

    clear_env();
}
