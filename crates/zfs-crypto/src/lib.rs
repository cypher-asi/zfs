#![forbid(unsafe_code)]

mod derive;
mod encrypt;
mod error;
mod padding;
mod sector_key;
mod wrap;

pub use derive::derive_sector_id;
pub use encrypt::{decrypt_sector, encrypt_sector};
pub use error::CryptoError;
pub use padding::{pad_to_bucket, unpad_from_bucket};
pub use sector_key::SectorKey;
pub use wrap::{unwrap_sector_key, wrap_sector_key, KeyEnvelopeEntry};

#[cfg(test)]
mod tests;
