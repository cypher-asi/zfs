use std::net::SocketAddr;

/// Configuration for the JSON-RPC HTTP server.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// Enable the RPC server. Default: `false`.
    pub enabled: bool,
    /// Bind address. Default: `127.0.0.1:4690`.
    pub bind_addr: SocketAddr,
    /// Optional API key. `None` = open access.
    pub api_key: Option<String>,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_addr: ([127, 0, 0, 1], 4690).into(),
            api_key: None,
        }
    }
}
