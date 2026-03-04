use std::collections::HashMap;
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

const TICK_MS: u64 = 50;
const CONFIG_REFRESH_TICKS: u64 = 4;
const MEMPOOL_CAP: f64 = 65_536.0;
const BACKPRESSURE_LOW: f64 = 0.75;
const BACKPRESSURE_HIGH: f64 = 0.90;

/// Spawn a traffic generator that publishes random spend transactions to nodes.
///
/// Uses batched sends: every TICK_MS we compute how many txs to emit for that
/// tick and publish them all at once, avoiding per-tx sleeps and mutex locks.
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
        let mut cached_enabled = false;
        let mut cached_rate: f32 = 0.0;
        let mut ticks_since_refresh: u64 = CONFIG_REFRESH_TICKS;
        let mut fractional_carry: f64 = 0.0;
        let mut backpressure_scale: f64 = 1.0;

        let tick = Duration::from_millis(TICK_MS);

        loop {
            tokio::time::sleep(tick).await;

            ticks_since_refresh += 1;
            if ticks_since_refresh >= CONFIG_REFRESH_TICKS {
                let state = shared.lock().await;
                cached_enabled = state.auto_traffic;
                cached_rate = state.traffic_rate;

                let max_occupancy_pct = state
                    .nodes
                    .iter()
                    .flat_map(|ns| ns.mempool_sizes.values())
                    .map(|&sz| sz as f64 / MEMPOOL_CAP)
                    .fold(0.0_f64, f64::max);

                backpressure_scale = if max_occupancy_pct >= BACKPRESSURE_HIGH {
                    0.0
                } else if max_occupancy_pct > BACKPRESSURE_LOW {
                    let range = BACKPRESSURE_HIGH - BACKPRESSURE_LOW;
                    ((BACKPRESSURE_HIGH - max_occupancy_pct) / range).clamp(0.0, 1.0)
                } else {
                    1.0
                };

                ticks_since_refresh = 0;
            }

            if !cached_enabled || cached_rate <= 0.0 || nodes.is_empty() || total_zones == 0 {
                fractional_carry = 0.0;
                continue;
            }

            let exact =
                cached_rate as f64 * backpressure_scale * (TICK_MS as f64 / 1000.0)
                    + fractional_carry;
            let batch_size = exact as u64;
            fractional_carry = exact - batch_size as f64;

            if batch_size == 0 {
                continue;
            }

            let mut zone_batches: HashMap<usize, Vec<SpendTransaction>> = HashMap::new();
            for _ in 0..batch_size {
                let tx = make_random_spend(&mut seq);
                let zone_id =
                    grid_services_zephyr::routing::zone_for_nullifier(&tx.nullifier, total_zones)
                        as usize;
                zone_batches.entry(zone_id).or_default().push(tx);
                seq += 1;
            }

            let mut submitted: u64 = 0;
            for (zone_id, txs) in zone_batches {
                let count = txs.len() as u64;
                let msg = ZephyrZoneMessage::SubmitSpendBatch(txs);
                let data = match grid_core::encode_canonical(&msg) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(error = %e, "failed to encode spend batch");
                        continue;
                    }
                };
                let topic = &zone_topics[zone_id % zone_topics.len()];
                let node_idx = (seq as usize + zone_id) % nodes.len();
                nodes[node_idx].publish(topic.clone(), data);
                submitted += count;
            }

            let bp_active = backpressure_scale < 1.0;
            if bp_active {
                debug!(
                    submitted,
                    rate = cached_rate,
                    scale = format!("{:.2}", backpressure_scale),
                    "traffic gen: backpressure active"
                );
            } else {
                debug!(submitted, rate = cached_rate, "traffic gen: batch sent");
            }

            {
                let mut state = shared.lock().await;
                state.traffic_stats.backpressure_active = bp_active;
                state.traffic_stats.total_submitted += submitted;
                let now = std::time::Instant::now();
                let start_seq = seq - submitted;
                let keep = submitted.min(50);
                for i in (submitted - keep)..submitted {
                    let s = start_seq + i;
                    let recent = crate::state::RecentTransaction {
                        nullifier_hex: hex::encode(&msg_nullifier_bytes(s)[..8]),
                        zone_id: (s as u32) % total_zones,
                        timestamp: now,
                    };
                    state.traffic_stats.recent.push_back(recent);
                }
                while state.traffic_stats.recent.len() > 50 {
                    state.traffic_stats.recent.pop_front();
                }
            }
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
