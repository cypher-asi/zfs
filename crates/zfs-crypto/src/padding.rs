use crate::CryptoError;

/// Bucket sizes for content padding (2× progression):
/// 256 B, 512 B, 1 KB, 2 KB, 4 KB, 8 KB, 16 KB, 32 KB, 64 KB, 128 KB, 256 KB.
const BUCKET_SIZES: &[usize] = &[
    256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072, 262_144,
];

/// Pad content to the next bucket boundary.
///
/// Format: `[4-byte LE length][content][zero-fill to bucket size]`.
/// The largest bucket is 256 KB; content exceeding that is padded to
/// the next 256 KB multiple.
pub fn pad_to_bucket(content: &[u8]) -> Vec<u8> {
    let total_needed = 4 + content.len();
    let bucket = select_bucket(total_needed);
    let mut buf = Vec::with_capacity(bucket);
    buf.extend_from_slice(&(content.len() as u32).to_le_bytes());
    buf.extend_from_slice(content);
    buf.resize(bucket, 0);
    buf
}

/// Remove padding applied by [`pad_to_bucket`], returning the original content.
pub fn unpad_from_bucket(padded: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if padded.len() < 4 {
        return Err(CryptoError::PaddingError("padded data too short".into()));
    }
    let len = u32::from_le_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;
    let end = 4 + len;
    if end > padded.len() {
        return Err(CryptoError::PaddingError(format!(
            "declared length {len} exceeds padded buffer ({})",
            padded.len()
        )));
    }
    Ok(padded[4..end].to_vec())
}

fn select_bucket(total_needed: usize) -> usize {
    for &bucket in BUCKET_SIZES {
        if total_needed <= bucket {
            return bucket;
        }
    }
    let largest = *BUCKET_SIZES.last().expect("non-empty bucket list");
    total_needed.div_ceil(largest) * largest
}
