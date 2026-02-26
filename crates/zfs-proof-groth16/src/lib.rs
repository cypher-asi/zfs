//! Groth16 shape+encrypt proof system for ZFS.
//!
//! Provides:
//! - [`ShapeEncryptCircuit`]: arkworks R1CS circuit proving both CBOR shape
//!   conformance and correct Poseidon sponge encryption.
//! - [`Groth16ShapeProver`]: client-side encrypt + prove in one operation.
//! - [`Groth16ShapeVerifier`]: Zode-side verification via [`ProofVerifier`] trait.
//! - [`generate_keys_for_bucket`]: trusted setup key generation.

pub mod circuit;
pub mod error;
pub mod prover;
pub mod setup;
pub mod verifier;

pub use circuit::ShapeEncryptCircuit;
pub use error::Groth16Error;
pub use prover::Groth16ShapeProver;
pub use setup::{generate_keys_for_bucket, KEY_VERSION};
pub use verifier::Groth16ShapeVerifier;
