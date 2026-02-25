use rocksdb::{ColumnFamilyDescriptor, Options, DB};

use crate::config::CompressionType;
use crate::error::StorageError;
use crate::StorageConfig;

const CF_METADATA: &str = "metadata";
pub(crate) const CF_SECTORS: &str = "sectors";

/// RocksDB-backed storage implementation.
///
/// Provides sector storage via column families: `sectors` and `metadata`.
pub struct RocksStorage {
    db: DB,
    #[allow(dead_code)]
    config: StorageConfig,
}

impl RocksStorage {
    /// Open (or create) a RocksDB database with the given config.
    pub fn open(config: StorageConfig) -> Result<Self, StorageError> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        if let Some(max_files) = config.max_open_files {
            db_opts.set_max_open_files(max_files);
        }

        let compression = match config.compression {
            CompressionType::None => rocksdb::DBCompressionType::None,
            CompressionType::Lz4 => rocksdb::DBCompressionType::Lz4,
            CompressionType::Zstd => rocksdb::DBCompressionType::Zstd,
        };

        let cf_names = [CF_METADATA, CF_SECTORS];
        let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
            .iter()
            .map(|name| {
                let mut cf_opts = Options::default();
                cf_opts.set_compression_type(compression);
                ColumnFamilyDescriptor::new(*name, cf_opts)
            })
            .collect();

        let db = DB::open_cf_descriptors(&db_opts, &config.path, cf_descriptors)?;
        Ok(Self { db, config })
    }

    pub(crate) fn cf_handle(&self, name: &str) -> Result<&rocksdb::ColumnFamily, StorageError> {
        self.db
            .cf_handle(name)
            .ok_or_else(|| StorageError::CfNotFound(name.to_owned()))
    }

    pub(crate) fn db(&self) -> &DB {
        &self.db
    }
}
