#![forbid(unsafe_code)]

mod config;
mod error;
mod rocks;
mod sector_rocks;
mod sector_traits;

pub use config::{CompressionType, StorageConfig};
pub use error::StorageError;
pub use rocks::RocksStorage;
pub use sector_traits::{SectorStorageStats, SectorStore};

#[cfg(test)]
mod tests;
