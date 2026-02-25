use rand_core::RngCore;

use crate::error::CryptoError;

/// Opaque Shamir share with a 1-based index. Hex-serializable for UI display.
#[derive(Clone)]
pub struct ShamirShare {
    index: u8,
    data: Vec<u8>,
}

impl ShamirShare {
    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn to_hex(&self) -> String {
        let mut raw = vec![self.index];
        raw.extend_from_slice(&self.data);
        hex::encode(raw)
    }

    pub fn from_hex(h: &str) -> Result<Self, CryptoError> {
        let raw = hex::decode(h)
            .map_err(|e| CryptoError::ShamirRecoveryFailed(format!("hex decode: {e}")))?;
        if raw.is_empty() {
            return Err(CryptoError::ShamirRecoveryFailed("share is empty".into()));
        }
        Ok(Self {
            index: raw[0],
            data: raw[1..].to_vec(),
        })
    }
}

impl core::fmt::Debug for ShamirShare {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShamirShare")
            .field("index", &self.index)
            .field("len", &self.data.len())
            .finish()
    }
}

/// Split a 32-byte secret into `total` shares with `threshold` required for reconstruction.
pub(crate) fn split(
    secret: &[u8; 32],
    total: usize,
    threshold: usize,
    rng: &mut dyn RngCore,
) -> Result<Vec<ShamirShare>, CryptoError> {
    let raw_shares = shamir_vault::split(secret.as_slice(), total, threshold, rng)
        .map_err(|e| CryptoError::ShamirSplitFailed(format!("{e}")))?;

    Ok(raw_shares
        .into_iter()
        .enumerate()
        .map(|(i, data)| ShamirShare {
            index: (i + 1) as u8,
            data,
        })
        .collect())
}

/// Combine threshold-or-more shares back into a 32-byte secret.
pub(crate) fn combine(shares: &[ShamirShare]) -> Result<[u8; 32], CryptoError> {
    if shares.is_empty() {
        return Err(CryptoError::ShamirRecoveryFailed(
            "no shares provided".into(),
        ));
    }
    let raw: Vec<Vec<u8>> = shares.iter().map(|s| s.data.clone()).collect();
    let secret = shamir_vault::combine(&raw)
        .map_err(|e| CryptoError::ShamirRecoveryFailed(format!("{e}")))?;

    if secret.len() != 32 {
        return Err(CryptoError::ShamirRecoveryFailed(format!(
            "expected 32 bytes, got {}",
            secret.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&secret);
    Ok(out)
}
