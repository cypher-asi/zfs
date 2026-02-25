#![forbid(unsafe_code)]
//! ZFS Zode — ties together storage, network, and programs.
//!
//! The Zode is the storage node: it runs libp2p + QUIC via `zfs-net`,
//! subscribes to program topics via GossipSub, persists sectors
//! in RocksDB via `zfs-storage`, and enforces local storage policy.
//!
//! # Usage
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), zfs_zode::ZodeError> {
//! use zfs_zode::{Zode, ZodeConfig};
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
pub use error::ZodeError;
pub use metrics::{MetricsSnapshot, ZodeMetrics};
pub use types::{LogEvent, LogLevel, ZodeStatus};
pub use zode::Zode;
