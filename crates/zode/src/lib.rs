#![forbid(unsafe_code)]
//! Zode — ties together storage, network, and programs.
//!
//! The Zode is the storage node: it runs libp2p + QUIC via `grid-net`,
//! subscribes to program topics via GossipSub, persists sectors
//! in RocksDB via `grid-storage`, and enforces local storage policy.
//!
//! # Usage
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), zode::ZodeError> {
//! use zode::{Zode, ZodeConfig};
//!
//! let config = ZodeConfig::default();
//! let zode = Zode::start(config).await?;
//! let status = zode.status();
//! println!("peer count: {}", status.peer_count);
//! zode.shutdown().await;
//! # Ok(())
//! # }
//! ```

mod config;
mod error;
mod gossip;
mod metrics;
mod sector_handler;
mod types;
mod zode;

pub use config::{DefaultProgramsConfig, SectorFilter, SectorLimitsConfig, ZodeConfig};
pub use config::default_program_ids;
pub use grid_rpc::RpcConfig;
pub use error::ZodeError;
pub use metrics::{MetricsSnapshot, ZodeMetrics};
pub use sector_handler::SectorRequestHandler;
pub use types::{GossipAppendResult, GossipRejectReason, LogEvent, LogLevel, ZodeStatus};
pub use zode::Zode;
