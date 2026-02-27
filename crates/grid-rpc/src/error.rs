use std::fmt;

/// Errors from the RPC server.
#[derive(Debug)]
pub enum RpcError {
    /// Failed to bind the TCP listener.
    Bind(std::io::Error),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(e) => write!(f, "RPC bind error: {e}"),
        }
    }
}

impl std::error::Error for RpcError {}
