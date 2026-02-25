use zfs_core::{
    ProgramId, SectorFetchRequest, SectorFetchResponse, SectorId, SectorRequest, SectorResponse,
    SectorStoreRequest, SectorStoreResponse,
};
use zfs_crypto::{pad_to_bucket, unpad_from_bucket, SectorKey};

use crate::client::{Client, PendingRequest};
use crate::error::SdkError;

pub use zfs_crypto::{derive_sector_id, CryptoError};

/// Encrypt plaintext for sector storage: pad → build AAD → encrypt.
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

/// Store a sector via a connected Zode.
pub async fn sector_store(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
    payload: &[u8],
    overwrite: bool,
    expected_hash: Option<Vec<u8>>,
) -> Result<SectorStoreResponse, SdkError> {
    let request = SectorRequest::Store(SectorStoreRequest {
        program_id: *program_id,
        sector_id: sector_id.clone(),
        payload: payload.to_vec(),
        overwrite,
        expected_hash,
    });

    let response = send_sector_request(client, &request).await?;
    match response {
        SectorResponse::Store(r) => Ok(r),
        _ => Err(SdkError::Other("unexpected sector response variant".into())),
    }
}

/// Fetch a sector from a connected Zode.
pub async fn sector_fetch(
    client: &Client,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<SectorFetchResponse, SdkError> {
    let request = SectorRequest::Fetch(SectorFetchRequest {
        program_id: *program_id,
        sector_id: sector_id.clone(),
    });

    let response = send_sector_request(client, &request).await?;
    match response {
        SectorResponse::Fetch(r) => Ok(r),
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
