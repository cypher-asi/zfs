use thiserror::Error;

/// Errors from the RPC server.
#[derive(Debug, Error)]
pub enum RpcError {
    /// Failed to bind the TCP listener.
    #[error("RPC bind error: {0}")]
    Bind(#[source] std::io::Error),
}
