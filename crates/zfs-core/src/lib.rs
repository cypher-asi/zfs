#![forbid(unsafe_code)]

mod cbor;
mod cid;
mod error;
mod program_descriptor;
mod program_id;
mod sector_id;
mod sector_protocol;
mod serde_helpers;
mod util;

pub use cbor::{decode_canonical, encode_canonical};
pub use cid::Cid;
pub use error::{ErrorCode, SectorStoreError, ZfsError};
pub use program_descriptor::ProgramDescriptor;
pub use program_id::ProgramId;
pub use sector_id::SectorId;
pub use util::format_bytes;
pub use sector_protocol::{
    GossipSector, SectorBatchFetchRequest, SectorBatchFetchResponse, SectorBatchStoreEntry,
    SectorBatchStoreRequest, SectorBatchStoreResponse, SectorFetchRequest, SectorFetchResponse,
    SectorRequest, SectorResponse, SectorStoreRequest, SectorStoreResponse, SectorStoreResult,
    MAX_BATCH_ENTRIES, MAX_BATCH_PAYLOAD_BYTES,
};
