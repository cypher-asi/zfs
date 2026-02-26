use zfs_core::{
    FieldSchema, ProgramId, SectorAppendRequest, SectorAppendResponse, SectorId,
    SectorLogLengthRequest, SectorLogLengthResponse, SectorReadLogRequest,
    SectorReadLogResponse, SectorRequest, SectorResponse, ShapeProof,
};
use zfs_crypto::{pad_to_bucket, unpad_from_bucket, SectorKey};
use zfs_proof_groth16::Groth16ShapeProver;

use crate::client::{Client, PendingRequest};
use crate::error::SdkError;

pub use zfs_crypto::{derive_sector_id, CryptoError};

/// Signature verification result for decrypted messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureStatus {
    Verified,
    Failed,
    UnknownSender,
    NoSignature,
}

/// Encrypt plaintext for sector storage: pad → build AAD → encrypt.
/// Uses the legacy XChaCha20-Poly1305 path (for proof_system=None).
pub fn sector_encrypt(
    plaintext: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<Vec<u8>, SdkError> {
    let padded = pad_to_bucket(plaintext);
    let aad = build_sector_aad(program_id, sector_id);
    let ciphertext =
        zfs_crypto::encrypt_sector(&padded, sector_key, &aad).map_err(SdkError::Crypto)?;
    Ok(ciphertext)
}

/// Decrypt ciphertext from sector storage: decrypt → unpad.
/// Uses the legacy XChaCha20-Poly1305 path (for proof_system=None).
pub fn sector_decrypt(
    ciphertext: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<Vec<u8>, SdkError> {
    let aad = build_sector_aad(program_id, sector_id);
    let padded =
        zfs_crypto::decrypt_sector(ciphertext, sector_key, &aad).map_err(SdkError::Crypto)?;
    let plaintext = unpad_from_bucket(&padded).map_err(SdkError::Crypto)?;
    Ok(plaintext)
}

/// Encrypt with Poseidon sponge AND generate a Groth16 shape proof.
///
/// `plaintext_cbor` should be a signed message already encoded to canonical CBOR.
/// Returns `(ciphertext, ShapeProof)`.
pub fn sector_encrypt_and_prove(
    plaintext_cbor: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    prover: &Groth16ShapeProver,
    schema: &FieldSchema,
) -> Result<(Vec<u8>, ShapeProof), SdkError> {
    let padded = pad_to_bucket(plaintext_cbor);
    prover
        .encrypt_and_prove(&padded, sector_key, program_id, sector_id, schema)
        .map_err(SdkError::Proof)
}

/// Decrypt Poseidon sponge ciphertext.
pub fn sector_decrypt_poseidon(
    ciphertext: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<Vec<u8>, SdkError> {
    let aad = build_sector_aad(program_id, sector_id);
    let padded =
        zfs_crypto::poseidon_decrypt_sector(ciphertext, sector_key, &aad).map_err(SdkError::Crypto)?;
    let plaintext = unpad_from_bucket(&padded).map_err(SdkError::Crypto)?;
    Ok(plaintext)
}

/// Decrypt Poseidon ciphertext and verify the sender's hybrid signature.
///
/// `resolve_key` maps a sender DID to their `IdentityVerifyingKey`.
pub fn sector_decrypt_and_verify<F>(
    ciphertext: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    resolve_key: F,
) -> Result<(Vec<u8>, SignatureStatus), SdkError>
where
    F: FnOnce(&str) -> Option<zero_neural::IdentityVerifyingKey>,
{
    let plaintext = sector_decrypt_poseidon(ciphertext, sector_key, program_id, sector_id)?;
    let msg: zfs_programs::ZChatMessage =
        zfs_core::decode_canonical(&plaintext).map_err(SdkError::Core)?;

    if msg.signature.is_empty() {
        return Ok((plaintext, SignatureStatus::NoSignature));
    }

    let status = match resolve_key(&msg.sender_did) {
        Some(vk) => {
            let signable = msg.signable_bytes().map_err(SdkError::Core)?;
            let sig = zero_neural::HybridSignature::from_bytes(&msg.signature)
                .map_err(|e| SdkError::Other(format!("bad signature bytes: {e}")))?;
            match vk.verify(&signable, &sig) {
                Ok(()) => SignatureStatus::Verified,
                Err(_) => SignatureStatus::Failed,
            }
        }
        None => SignatureStatus::UnknownSender,
    };

    Ok((plaintext, status))
}

/// Append an entry to a sector log via a connected Zode.
pub async fn sector_append(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
    entry: &[u8],
) -> Result<SectorAppendResponse, SdkError> {
    sector_append_with_proof(client, program_id, sector_id, entry, None).await
}

/// Append an entry to a sector log with an optional shape proof.
pub async fn sector_append_with_proof(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
    entry: &[u8],
    shape_proof: Option<ShapeProof>,
) -> Result<SectorAppendResponse, SdkError> {
    let request = SectorRequest::Append(SectorAppendRequest {
        program_id: *program_id,
        sector_id: sector_id.clone(),
        entry: entry.to_vec(),
        shape_proof,
    });

    let response = send_sector_request(client, &request).await?;
    match response {
        SectorResponse::Append(r) => Ok(r),
        _ => Err(SdkError::Other("unexpected sector response variant".into())),
    }
}

/// Read log entries from a sector via a connected Zode.
pub async fn sector_read_log(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
    from_index: u64,
    max_entries: u32,
) -> Result<SectorReadLogResponse, SdkError> {
    let request = SectorRequest::ReadLog(SectorReadLogRequest {
        program_id: *program_id,
        sector_id: sector_id.clone(),
        from_index,
        max_entries,
    });

    let response = send_sector_request(client, &request).await?;
    match response {
        SectorResponse::ReadLog(r) => Ok(r),
        _ => Err(SdkError::Other("unexpected sector response variant".into())),
    }
}

/// Query the length of a sector log via a connected Zode.
pub async fn sector_log_length(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<SectorLogLengthResponse, SdkError> {
    let request = SectorRequest::LogLength(SectorLogLengthRequest {
        program_id: *program_id,
        sector_id: sector_id.clone(),
    });

    let response = send_sector_request(client, &request).await?;
    match response {
        SectorResponse::LogLength(r) => Ok(r),
        _ => Err(SdkError::Other("unexpected sector response variant".into())),
    }
}

async fn send_sector_request(
    client: &Client,
    request: &SectorRequest,
) -> Result<SectorResponse, SdkError> {
    let peers = client.connected_peers().await;
    if peers.is_empty() {
        return Err(SdkError::NoPeers);
    }

    let peer = peers[0];
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request_id = {
        let mut net = client.network.lock().await;
        net.send_sector_request(&peer, request.clone())
    };
    {
        let mut pending = client.pending.lock().await;
        pending.insert(request_id, PendingRequest::Sector(tx));
    }

    match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(_)) => Err(SdkError::Other("sector response channel dropped".into())),
        Err(_) => Err(SdkError::Timeout(
            "sector request timed out after 30s".into(),
        )),
    }
}

fn build_sector_aad(program_id: &ProgramId, sector_id: &SectorId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(program_id.as_bytes().len() + sector_id.as_bytes().len());
    aad.extend_from_slice(program_id.as_bytes());
    aad.extend_from_slice(sector_id.as_bytes());
    aad
}
