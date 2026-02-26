#![forbid(unsafe_code)]
//! ZFS client SDK — identity, connect, encrypt, sign, sector operations.
//!
//! Wraps `zero-neural`, `zfs-net`, `zfs-crypto`, and `zfs-programs`
//! into a unified client API. Does **not** use RocksDB.
//!
//! # Quick start
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), zfs_sdk::SdkError> {
//! use zfs_sdk::{SdkConfig, Client};
//!
//! let client = Client::connect(&SdkConfig::default()).await?;
//! // ... generate keys, encrypt, sector_store, sector_fetch ...
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod helpers;
mod identity;
pub mod sector;

pub use client::{Client, SdkConfig};
pub use error::SdkError;
pub use helpers::{zchat_descriptor, zchat_descriptor_v2, zid_descriptor, zid_descriptor_v2};
pub use identity::{
    derive_machine_keypair_from_shares, generate_identity, sign_with_shares, verify_shares,
    IdentityBundle, IdentityInfo,
};
pub use sector::{
    sector_append, sector_append_with_proof, sector_decrypt, sector_decrypt_and_verify,
    sector_decrypt_poseidon, sector_encrypt, sector_encrypt_and_prove, sector_log_length,
    sector_read_log, SignatureStatus,
};

// Re-export frequently used types so callers don't need extra deps.
pub use zero_neural::{
    HybridSignature, IdentitySigningKey, IdentityVerifyingKey, MachineKeyCapabilities,
    MachineKeyPair, MachinePublicKey, ShamirShare,
};
pub use zfs_core::{
    CborType, Cid, FieldDef, FieldSchema, ProgramId, ProofSystem, SectorId, ShapeProof,
};
pub use zfs_crypto::{
    decrypt_sector, encrypt_sector, pad_to_bucket, poseidon_decrypt_sector,
    poseidon_encrypt_sector, poseidon_hash, unpad_from_bucket, SectorKey,
};
pub use zfs_programs::{program_topic, ZChatDescriptor, ZChatMessage, ZidDescriptor, ZidMessage};
pub use zfs_proof_groth16::Groth16ShapeProver;
