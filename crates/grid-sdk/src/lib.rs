#![forbid(unsafe_code)]
//! Grid client SDK — identity, connect, encrypt, sign, sector operations.
//!
//! Wraps `zid`, `grid-net`, `grid-crypto`, and program crates
//! into a unified client API. Does **not** use RocksDB.
//!
//! # Quick start
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), grid_sdk::SdkError> {
//! use grid_sdk::{SdkConfig, Client};
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
pub use helpers::{interlink_descriptor, interlink_descriptor_v2, zid_descriptor, zid_descriptor_v2};
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
pub use zid::{
    HybridSignature, IdentitySigningKey, IdentityVerifyingKey, MachineKeyCapabilities,
    MachineKeyPair, MachinePublicKey, ShamirShare,
};
pub use grid_core::{
    CborType, Cid, FieldDef, FieldSchema, ProgramId, ProofSystem, SectorId, ShapeProof,
};
pub use grid_crypto::{
    decrypt_sector, encrypt_sector, pad_to_bucket, poseidon_decrypt_sector,
    poseidon_encrypt_sector, poseidon_hash, unpad_from_bucket, SectorKey,
};
pub use grid_core::program_topic;
pub use grid_programs_zid::{ZidDescriptor, ZidMessage};
pub use grid_programs_interlink::{InterlinkDescriptor, ZMessage};
pub use grid_proof_groth16::Groth16ShapeProver;
