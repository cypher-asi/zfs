use std::sync::Arc;
use std::time::Duration;

use grid_programs_zephyr::{
    NoteCommitment, NoteOutput, Nullifier, SpendTransaction, ZephyrZoneMessage,
};
use rand::Rng;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use zode::Zode;

use crate::state::AppState;

/// Spawn a traffic generator that publishes random spend transactions to nodes.
///
/// The generator checks `shared.auto_traffic` every tick and adjusts its
/// rate from `shared.traffic_rate` (transactions per second).
pub(crate) fn spawn_traffic_generator(
    nodes: &[Arc<Zode>],
    zone_program_ids: &[grid_core::ProgramId],
    total_zones: u32,
    shared: Arc<Mutex<AppState>>,
    rt: &tokio::runtime::Runtime,
) -> tokio::task::JoinHandle<()> {
    let nodes: Vec<Arc<Zode>> = nodes.to_vec();
    let zone_topics: Vec<String> = zone_program_ids
        .iter()
        .map(grid_core::program_topic)
        .collect();

    rt.spawn(async move {
        let mut seq: u64 = 0;
        loop {
            let (enabled, rate) = {
                let state = shared.lock().await;
                (state.auto_traffic, state.traffic_rate)
            };

            if !enabled || rate <= 0.0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }

            let interval = Duration::from_secs_f64(1.0 / rate.min(100.0) as f64);
            tokio::time::sleep(interval).await;

            if nodes.is_empty() || total_zones == 0 {
                continue;
            }

            let tx = make_random_spend(&mut seq);
            let zone_id = grid_services_zephyr::routing::zone_for_nullifier(
                &tx.nullifier,
                total_zones,
            ) as usize;

            let msg = ZephyrZoneMessage::SubmitSpend(tx);
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode spend");
                    continue;
                }
            };

            let topic = &zone_topics[zone_id % zone_topics.len()];
            let node_idx = seq as usize % nodes.len();
            nodes[node_idx].publish(topic.clone(), data);

            // Track the submission
            {
                let mut state = shared.lock().await;
                state.traffic_stats.total_submitted += 1;
                let recent = crate::state::RecentTransaction {
                    nullifier_hex: hex::encode(&msg_nullifier_bytes(seq.wrapping_sub(1))[..8]),
                    zone_id: zone_id as u32,
                    timestamp: std::time::Instant::now(),
                };
                state.traffic_stats.recent.push_back(recent);
                if state.traffic_stats.recent.len() > 50 {
                    state.traffic_stats.recent.pop_front();
                }
            }

            debug!(seq, zone_id, "traffic gen: submitted spend");
            seq += 1;
        }
    })
}

fn make_random_spend(seq: &mut u64) -> SpendTransaction {
    let mut rng = rand::thread_rng();
    let mut nullifier_bytes = [0u8; 32];
    nullifier_bytes[..8].copy_from_slice(&seq.to_le_bytes());
    rng.fill(&mut nullifier_bytes[8..]);

    let mut commitment_bytes = [0u8; 32];
    rng.fill(&mut commitment_bytes[..]);

    let mut output_commitment = [0u8; 32];
    rng.fill(&mut output_commitment[..]);

    SpendTransaction {
        input_commitment: NoteCommitment(commitment_bytes),
        nullifier: Nullifier(nullifier_bytes),
        outputs: vec![NoteOutput {
            commitment: NoteCommitment(output_commitment),
            encrypted_data: vec![0u8; 32],
        }],
        proof: vec![0u8; 64],
        public_signals: vec![],
    }
}

fn msg_nullifier_bytes(seq: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&seq.to_le_bytes());
    b
}
