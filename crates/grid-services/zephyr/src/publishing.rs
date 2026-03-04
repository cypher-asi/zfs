use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use grid_programs_zephyr::{
    Block, FinalityCertificate, Nullifier, ZephyrConsensusMessage, ZephyrGlobalMessage,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::consensus::ConsensusAction;
use crate::service::ZephyrRuntime;
use crate::shared_mempool::SharedMempool;

const MAX_BLOCK_TX_CACHE: usize = 200;

pub(crate) fn cache_block_txs(
    cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    nullifier_cache: &mut HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    zone_id: u32,
    block: &Block,
) {
    if cache.len() >= MAX_BLOCK_TX_CACHE {
        let keys: Vec<[u8; 32]> = cache.keys().take(MAX_BLOCK_TX_CACHE / 4).copied().collect();
        for k in &keys {
            cache.remove(k);
            nullifier_cache.remove(k);
        }
    }
    let full_nullifiers: Vec<Nullifier> = block
        .transactions
        .iter()
        .map(|tx| tx.nullifier.clone())
        .collect();
    let hex_nullifiers: Vec<String> = full_nullifiers
        .iter()
        .map(|n| hex::encode(&n.0[..8]))
        .collect();
    cache.insert(block.block_hash, (zone_id, hex_nullifiers));
    nullifier_cache.insert(block.block_hash, (zone_id, full_nullifiers));
}

pub(crate) fn cleanup_mempool_after_cert(
    cert: &FinalityCertificate,
    mempool: &SharedMempool,
    block_nullifiers: &mut HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    deferred_cleanups: &mut HashMap<[u8; 32], u32>,
) {
    if let Some((zone_id, nullifiers)) = block_nullifiers.remove(&cert.block_hash) {
        mempool.remove_nullifiers(zone_id, &nullifiers);
    } else {
        deferred_cleanups.insert(cert.block_hash, cert.zone_id);
    }
}

pub(crate) fn apply_certificate_locally(
    cert: &FinalityCertificate,
    zone_head_store: &DashMap<u32, [u8; 32]>,
    block_tx_cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    runtime: &Arc<parking_lot::RwLock<ZephyrRuntime>>,
) {
    zone_head_store.insert(cert.zone_id, cert.block_hash);
    let tx_nullifiers = block_tx_cache
        .get(&cert.block_hash)
        .map(|(_, n)| n.clone())
        .unwrap_or_default();
    let spend_count = tx_nullifiers.len() as u64;
    let mut rt = runtime.write();
    rt.zone_heads.insert(cert.zone_id, cert.block_hash);
    rt.certificates_produced += 1;
    rt.spends_processed += spend_count;

    let height = rt.zone_heights.entry(cert.zone_id).or_insert(0);
    *height += 1;
    let block_height = *height;

    info!(
        zone_id = cert.zone_id,
        height = block_height,
        spend_count,
        block_hash = %hex::encode(&cert.block_hash[..8]),
        "certificate applied, block finalized"
    );

    rt.recent_blocks.push_back(crate::service::BlockSummary {
        zone_id: cert.zone_id,
        block_hash_hex: hex::encode(&cert.block_hash[..8]),
        height: block_height,
        tx_nullifiers,
    });
    rt.blocks_produced += 1;
    if rt.recent_blocks.len() > crate::service::MAX_RECENT_BLOCKS {
        rt.recent_blocks.pop_front();
    }
    rt.zone_consecutive_timeouts.insert(cert.zone_id, 0);
    rt.zone_last_advance
        .insert(cert.zone_id, std::time::Instant::now());
}

pub(crate) fn publish_action(
    action: &ConsensusAction,
    consensus_topic: &str,
    global_topic: &str,
    publish_tx: &mpsc::Sender<(String, Vec<u8>)>,
    block_tx_cache: &HashMap<[u8; 32], (u32, Vec<String>)>,
    block_nullifiers: &HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
) {
    let (topic, data) = match action {
        ConsensusAction::BroadcastProposal(p) => {
            let encode_start = std::time::Instant::now();
            let msg = ZephyrConsensusMessage::Proposal(p.clone());
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode proposal");
                    return;
                }
            };
            // #region agent log
            {
                let encode_us = encode_start.elapsed().as_micros();
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("debug-6fcc0e.log") {
                    let _ = writeln!(f, r#"{{"sessionId":"6fcc0e","hypothesisId":"L","location":"publishing.rs:proposal_size","message":"proposal encoded","data":{{"zone_id":{},"tx_count":{},"encoded_bytes":{},"encode_us":{}}},"timestamp":{}}}"#,
                        p.header.zone_id, p.transactions.len(), data.len(), encode_us,
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
                }
            }
            // #endregion
            (consensus_topic.to_owned(), data)
        }
        ConsensusAction::BroadcastVote(v) => {
            let msg = ZephyrConsensusMessage::Vote(v.clone());
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode vote");
                    return;
                }
            };
            (consensus_topic.to_owned(), data)
        }
        ConsensusAction::BroadcastCertificate(c) => {
            let tx_nullifiers = block_tx_cache
                .get(&c.block_hash)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            let nullifiers = block_nullifiers
                .get(&c.block_hash)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            let msg = ZephyrGlobalMessage::Certificate {
                cert: c.clone(),
                tx_nullifiers,
                nullifiers,
            };
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode certificate");
                    return;
                }
            };
            (global_topic.to_owned(), data)
        }
    };

    let msg_type = match action {
        ConsensusAction::BroadcastProposal(_) => "proposal",
        ConsensusAction::BroadcastVote(_) => "vote",
        ConsensusAction::BroadcastCertificate(_) => "certificate",
    };
    if let Err(e) = publish_tx.try_send((topic, data)) {
        let is_closed = publish_tx.is_closed();
        // #region agent log
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("debug-6fcc0e.log") {
                let _ = writeln!(f, r#"{{"sessionId":"6fcc0e","hypothesisId":"E","location":"publishing.rs:publish_fail","message":"publish failed","data":{{"msg_type":"{}","is_closed":{},"capacity":{}}},"timestamp":{}}}"#,
                    msg_type, is_closed, publish_tx.capacity(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
            }
        }
        // #endregion
        warn!(error = %e, msg_type, "publish channel full, consensus message dropped");
    }
}
