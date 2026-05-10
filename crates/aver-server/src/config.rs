#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub base_url: String,
    pub memory_dir: String,
    pub auth_db_path: String,
    pub cors_origins: Vec<String>,
    /// Optional reverse-proxy header name used for Profile C public deployments.
    ///
    /// If present, non-loopback OAuth authorize requests may be authenticated
    /// by this header in `AVER_TRUSTED_AUTH_HEADER` (for example,
    /// `X-Forwarded-User`).
    pub trusted_auth_header: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let host = std::env::var("AVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("AVER_PORT")
            .unwrap_or_else(|_| "3317".to_string())
            .parse()?;
        let base_url =
            std::env::var("AVER_BASE_URL").unwrap_or_else(|_| format!("http://{host}:{port}"));
        let memory_dir = std::env::var("AVER_MEMORY_DIR").unwrap_or_else(|_| ".aver".to_string());
        let auth_db_path =
            std::env::var("AVER_AUTH_DB_PATH").unwrap_or_else(|_| format!("{memory_dir}/auth.db"));
        let cors_origins = std::env::var("AVER_CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(ToString::to_string)
            .collect();
        let trusted_auth_header = std::env::var("AVER_TRUSTED_AUTH_HEADER")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());

        Ok(Self {
            host,
            port,
            base_url,
            memory_dir,
            auth_db_path,
            cors_origins,
            trusted_auth_header,
        })
    }
}
