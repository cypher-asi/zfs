#![forbid(unsafe_code)]
//! JSON-RPC 2.0 HTTP server for The Grid.
//!
//! Provides external (non-P2P) clients access to sector operations via a
//! simple HTTP POST endpoint at `/rpc`. The server uses axum and speaks
//! JSON-RPC 2.0 over the existing `SectorRequest`/`SectorResponse` types.

mod config;
mod dispatch;
mod error;
mod server;

pub use config::RpcConfig;
pub use error::RpcError;
pub use server::RpcServer;

use grid_core::{SectorRequest, SectorResponse};

/// Trait for dispatching sector requests, implemented by the Zode's
/// `SectorRequestHandler` to avoid a circular crate dependency.
pub trait SectorDispatch: Send + Sync + 'static {
    fn dispatch(&self, req: &SectorRequest) -> SectorResponse;
}
