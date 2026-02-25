use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;
use zfs_core::SectorId;

const HKDF_SALT: &[u8] = b"zfs:sector:v1";
const DERIVE_KEY_INFO: &[u8] = b"zfs:sector:derive-key:v1";

/// Derive a deterministic [`SectorId`] from a shared secret and caller-provided info.
///
/// Uses two-step HKDF-SHA256:
///   1. Extract a 32-byte derivation key from `shared_secret` with `DERIVE_KEY_INFO`.
///   2. Expand the derivation key with the caller's `info` to produce the sector ID.
///
/// The intermediate derivation key is zeroized after use.
pub fn derive_sector_id(
    shared_secret: &[u8; 32],
    info: &[u8],
) -> Result<SectorId, crate::CryptoError> {
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), shared_secret);
    let mut derivation_key = [0u8; 32];
    hk.expand(DERIVE_KEY_INFO, &mut derivation_key)
        .map_err(|_| crate::CryptoError::HkdfExpandFailed)?;

    let hk2 = Hkdf::<Sha256>::new(Some(HKDF_SALT), &derivation_key);
    derivation_key.zeroize();

    let mut sector_id_bytes = [0u8; 32];
    hk2.expand(info, &mut sector_id_bytes)
        .map_err(|_| crate::CryptoError::HkdfExpandFailed)?;

    Ok(SectorId::from_bytes(sector_id_bytes.to_vec()))
}
