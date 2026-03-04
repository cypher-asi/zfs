use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use grid_net::{Keypair, Multiaddr, ZodeId};
use grid_programs_zephyr::ValidatorInfo;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use zode::{LogEvent, Zode};

use crate::state::{
    AggregatedLogEntry, AppState, LogLevel, NetworkPreset, NetworkSnapshot, NodeState,
};

pub(crate) struct ManagedNode {
    pub zode: Arc<Zode>,
    #[allow(dead_code)]
    pub validator_id: [u8; 32],
    pub node_id: usize,
}

/// Launch all nodes for the given preset asynchronously.
///
/// Starts the first node to obtain a bootstrap address, then spawns all
/// remaining nodes concurrently via [`tokio::task::JoinSet`].  After
/// every node is up, a concurrent full-mesh dial ensures direct
/// connectivity between every pair.
pub(crate) async fn launch_network(
    preset: NetworkPreset,
    max_block_size: usize,
    round_interval_ms: u64,
    shared: Arc<Mutex<AppState>>,
) -> Vec<ManagedNode> {
    let validator_count = preset.validators();
    let total_zones = preset.zones();
    let committee_size = preset.committee_size();

    let keypairs: Vec<Keypair> = (0..validator_count)
        .map(|_| Keypair::generate_ed25519())
        .collect();

    let validators: Vec<ValidatorInfo> = keypairs
        .iter()
        .map(|kp: &Keypair| {
            let ed_pk = kp.public().try_into_ed25519().expect("keypair is ed25519");
            let vid: [u8; 32] = ed_pk.to_bytes();
            ValidatorInfo {
                validator_id: vid,
                pubkey: vid,
                p2p_endpoint: String::new(),
            }
        })
        .collect();

    let zephyr_config = grid_services_zephyr::ZephyrConfig {
        total_zones,
        committee_size,
        epoch_duration_ms: 120_000,
        round_interval_ms,
        quorum_threshold: ((2 * committee_size) / 3) + 1,
        max_block_size,
        round_timeout_ticks: 50,
        max_pending_certs: 512,
        initial_randomness: [0u8; 32],
        validators: validators.clone(),
        self_validate: false,
        skip_proof_verification: true,
    };

    let zephyr_json =
        serde_json::to_value(&zephyr_config).expect("ZephyrConfig always serializes");

    let base_dir = std::env::temp_dir().join("zephyr-orchestrator");
    let _ = std::fs::remove_dir_all(&base_dir);
    let _ = std::fs::create_dir_all(&base_dir);

    let mut managed_nodes: Vec<ManagedNode> = Vec::with_capacity(validator_count);
    let mut addr_by_node: HashMap<usize, (String, ZodeId)> = HashMap::new();

    // ── Phase 1: start the first node to get a bootstrap address ──────
    if let Some((zode, addr)) = start_node(0, &keypairs[0], &zephyr_json, &base_dir, &[]).await {
        let peer_id = keypairs[0].public().to_peer_id();
        if let Some(ref a) = addr {
            let full_addr = format!("{a}/p2p/{peer_id}");
            info!(node = 0, addr = %full_addr, "node listen address captured");
            addr_by_node.insert(0, (full_addr, peer_id));
        } else {
            warn!(node = 0, "could not capture listen address");
        }
        managed_nodes.push(ManagedNode {
            zode,
            validator_id: validators[0].validator_id,
            node_id: 0,
        });
    }

    // ── Phase 2: start remaining nodes concurrently ───────────────────
    if validator_count > 1 {
        let bootstrap: Vec<String> = addr_by_node
            .values()
            .map(|(addr, _)| addr.clone())
            .collect();
        let mut set = tokio::task::JoinSet::new();

        for i in 1..validator_count {
            let kp = keypairs[i].clone();
            let vid = validators[i].validator_id;
            let zj = zephyr_json.clone();
            let bd = base_dir.clone();
            let boot = bootstrap.clone();

            set.spawn(async move {
                start_node(i, &kp, &zj, &bd, &boot)
                    .await
                    .map(|(zode, addr)| (i, vid, zode, addr))
            });
        }

        let mut batch: Vec<(usize, [u8; 32], Arc<Zode>, Option<String>)> = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(Some(tuple)) = res {
                batch.push(tuple);
            }
        }
        batch.sort_by_key(|(i, ..)| *i);

        for (i, vid, zode, addr) in batch {
            let peer_id = keypairs[i].public().to_peer_id();
            if let Some(ref a) = addr {
                let full_addr = format!("{a}/p2p/{peer_id}");
                info!(node = i, addr = %full_addr, "node listen address captured");
                addr_by_node.insert(i, (full_addr, peer_id));
            } else {
                warn!(node = i, "could not capture listen address");
            }
            managed_nodes.push(ManagedNode {
                zode,
                validator_id: vid,
                node_id: i,
            });
        }
    }

    // ── Phase 3: concurrent full-mesh dial ────────────────────────────
    {
        let local_peer_ids: HashMap<usize, ZodeId> = managed_nodes
            .iter()
            .map(|mn| (mn.node_id, keypairs[mn.node_id].public().to_peer_id()))
            .collect();

        let mut dial_set = tokio::task::JoinSet::new();
        for mn in &managed_nodes {
            let zode = Arc::clone(&mn.zode);
            let src = mn.node_id;
            let src_peer_id = local_peer_ids[&src];
            let targets: Vec<Multiaddr> = addr_by_node
                .iter()
                .filter(|(_, (_, pid))| *pid != src_peer_id)
                .filter_map(|(_, (a, _))| a.parse::<Multiaddr>().ok())
                .collect();

            dial_set.spawn(async move {
                for addr in targets {
                    let mut net = zode.network().lock().await;
                    if let Err(e) = net.dial(addr) {
                        warn!(from = src, error = %e, "full-mesh dial failed");
                    }
                }
            });
        }
        while dial_set.join_next().await.is_some() {}
    }

    // ── Phase 4: seed shared state ────────────────────────────────────
    {
        let node_count = managed_nodes.len();
        let mut state = shared.lock().await;
        state.network = NetworkSnapshot {
            total_zones,
            ..Default::default()
        };
        state.nodes = (0..node_count).map(NodeState::new).collect();
    }

    managed_nodes
}

/// Start a single node and capture its listen address.
async fn start_node(
    i: usize,
    kp: &Keypair,
    zephyr_json: &serde_json::Value,
    base_dir: &std::path::Path,
    boot_addrs: &[String],
) -> Option<(Arc<Zode>, Option<String>)> {
    let data_dir = base_dir.join(format!("node-{i}"));
    let _ = std::fs::create_dir_all(&data_dir);

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/udp/0/quic-v1"
        .parse()
        .expect("well-known constant multiaddr");

    let bootstrap_peers: Vec<Multiaddr> = boot_addrs
        .iter()
        .filter_map(|a| a.parse::<Multiaddr>().ok())
        .collect();

    let net_config = grid_net::NetworkConfig {
        listen_addr,
        keypair: Some(kp.clone()),
        bootstrap_peers,
        discovery: grid_net::DiscoveryConfig {
            allow_private_addresses: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let storage_config = grid_storage::StorageConfig::new(data_dir);

    let mut service_configs = HashMap::new();
    service_configs.insert("ZEPHYR".to_string(), zephyr_json.clone());

    let config = zode::ZodeConfig {
        storage: storage_config,
        default_programs: zode::DefaultProgramsConfig {
            zid: false,
            interlink: false,
        },
        topics: HashSet::new(),
        sector_limits: zode::SectorLimitsConfig::default(),
        sector_filter: zode::SectorFilter::default(),
        network: net_config,
        rpc: zode::RpcConfig {
            enabled: false,
            ..Default::default()
        },
        services: zode::ServiceRegistryConfig::default(),
        service_configs,
    };

    match Zode::start(config).await {
        Ok(z) => {
            let zode = Arc::new(z);
            let sync_addrs = zode.network().lock().await.listen_addresses();
            let addr = if let Some(first) = sync_addrs.into_iter().next() {
                Some(first.to_string())
            } else {
                capture_listen_addr(&zode).await
            };
            Some((zode, addr))
        }
        Err(e) => {
            error!(node = i, error = %e, "failed to start node");
            None
        }
    }
}

/// Wait up to 5 seconds for the `LogEvent::Started` event from a Zode
/// and return the listen address (which includes the peer ID).
async fn capture_listen_addr(zode: &Arc<Zode>) -> Option<String> {
    let mut rx = zode.subscribe_events();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(LogEvent::Started { listen_addr }) => return Some(listen_addr),
                    Ok(_) => continue,
                    Err(_) => return None,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                warn!("timed out waiting for listen address");
                return None;
            }
        }
    }
}

/// Spawn a polling task for each node that updates shared state.
///
/// In addition to the basic `ZodeStatus`, this reads Zephyr service metrics
/// (zone_heads, current_epoch, certificates, spends, mempool_sizes, etc.)
/// and aggregates them into `NetworkSnapshot` and `NodeState`.
pub(crate) fn spawn_status_pollers(
    nodes: &[ManagedNode],
    shared: Arc<Mutex<AppState>>,
    rt: &Runtime,
) -> Vec<tokio::task::JoinHandle<()>> {
    nodes
        .iter()
        .map(|mn| {
            let zode = Arc::clone(&mn.zode);
            let node_id = mn.node_id;
            let shared = Arc::clone(&shared);
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let status = zode.status();

                    let registry = zode.service_registry().read().await;
                    let metrics = registry.service_metrics();
                    drop(registry);

                    let mut state = shared.lock().await;
                    if let Some(ns) = state.nodes.get_mut(node_id) {
                        ns.zode_id = status.zode_id.clone();
                        ns.status = Some(status);
                        ns.last_update = std::time::Instant::now();

                        if let Some(zephyr) = metrics.get("ZEPHYR") {
                            if let Some(zones) =
                                zephyr.get("assigned_zones").and_then(|v| v.as_array())
                            {
                                ns.assigned_zones = zones
                                    .iter()
                                    .filter_map(|z| z.as_u64().map(|n| n as u32))
                                    .collect();
                            }
                            if let Some(mp) =
                                zephyr.get("mempool_sizes").and_then(|v| v.as_object())
                            {
                                ns.mempool_sizes = mp
                                    .iter()
                                    .filter_map(|(k, v)| {
                                        let zone_id = k.parse::<u32>().ok()?;
                                        let size = v.as_u64()? as usize;
                                        Some((zone_id, size))
                                    })
                                    .collect();
                            }
                        }
                    }

                    // Aggregate network-level metrics from the first responding node
                    // (all nodes should converge on the same values)
                    if node_id == 0 {
                        if let Some(zephyr) = metrics.get("ZEPHYR") {
                            if let Some(epoch) =
                                zephyr.get("current_epoch").and_then(|v| v.as_u64())
                            {
                                state.network.current_epoch = epoch;
                            }
                            if let Some(pct) =
                                zephyr.get("epoch_progress_pct").and_then(|v| v.as_f64())
                            {
                                state.network.epoch_progress_pct = pct as f32;
                            }
                            if let Some(certs) =
                                zephyr.get("certificates_produced").and_then(|v| v.as_u64())
                            {
                                state.network.certificates_produced = certs;
                            }
                            if let Some(spends) =
                                zephyr.get("spends_processed").and_then(|v| v.as_u64())
                            {
                                state.network.spends_processed = spends;
                                state.tps_sampler.record(spends);
                            }
                            if let Some(heads) =
                                zephyr.get("zone_heads").and_then(|v| v.as_object())
                            {
                                for (k, v) in heads {
                                    if let (Ok(zone_id), Some(hex_str)) =
                                        (k.parse::<u32>(), v.as_str())
                                    {
                                        if let Ok(bytes) = hex::decode(hex_str) {
                                            let mut head = [0u8; 32];
                                            let len = bytes.len().min(32);
                                            head[..len].copy_from_slice(&bytes[..len]);
                                            state.network.zone_heads.insert(zone_id, head);
                                        }
                                    }
                                }
                            }

                            if let Some(timeouts) =
                                zephyr.get("zone_consecutive_timeouts").and_then(|v| v.as_object())
                            {
                                for (k, v) in timeouts {
                                    if let (Ok(zone_id), Some(ct)) =
                                        (k.parse::<u32>(), v.as_u64())
                                    {
                                        state.network.zone_consecutive_timeouts.insert(zone_id, ct as u32);
                                    }
                                }
                            }
                            if let Some(stalls) =
                                zephyr.get("zone_stall_durations_ms").and_then(|v| v.as_object())
                            {
                                for (k, v) in stalls {
                                    if let (Ok(zone_id), Some(dur)) =
                                        (k.parse::<u32>(), v.as_u64())
                                    {
                                        state.network.zone_stall_durations_ms.insert(zone_id, dur);
                                    }
                                }
                            }

                            if let Some(heights) =
                                zephyr.get("zone_heights").and_then(|v| v.as_object())
                            {
                                for (k, v) in heights {
                                    if let (Ok(zone_id), Some(h)) =
                                        (k.parse::<u32>(), v.as_u64())
                                    {
                                        state.network.zone_heights.insert(zone_id, h);
                                    }
                                }
                            }

                            let total_produced = zephyr
                                .get("blocks_produced")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize;

                            if let Some(blocks) =
                                zephyr.get("recent_blocks").and_then(|v| v.as_array())
                            {
                                let new_blocks = total_produced.saturating_sub(state.blocks_seen);
                                if new_blocks > 0 {
                                    let take = new_blocks.min(blocks.len());
                                    for b in &blocks[blocks.len() - take..] {
                                        let zone_id =
                                            b.get("zone_id").and_then(|v| v.as_u64()).unwrap_or(0)
                                                as u32;
                                        let block_hash_hex = b
                                            .get("block_hash")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_owned();
                                        let height =
                                            b.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
                                        let all_nullifiers: Vec<String> = b
                                            .get("tx_nullifiers")
                                            .and_then(|v| v.as_array())
                                            .map(|arr| {
                                                arr.iter()
                                                    .filter_map(|v| {
                                                        v.as_str().map(|s| s.to_owned())
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let tx_count = all_nullifiers.len();
                                        let cap = crate::state::MAX_DISPLAY_NULLIFIERS;
                                        let tx_nullifiers = if all_nullifiers.len() > cap {
                                            all_nullifiers.into_iter().take(cap).collect()
                                        } else {
                                            all_nullifiers
                                        };
                                        state.recent_blocks.push_back(crate::state::RecentBlock {
                                            zone_id,
                                            block_hash_hex,
                                            height,
                                            timestamp: std::time::Instant::now(),
                                            tx_nullifiers,
                                            tx_count,
                                        });
                                    }
                                    state.blocks_seen = total_produced;
                                    while state.recent_blocks.len() > 50 {
                                        state.recent_blocks.pop_front();
                                    }
                                }
                            }
                        }
                    }

                    let total_peers: usize = state
                        .nodes
                        .iter()
                        .filter_map(|n| n.status.as_ref())
                        .map(|s| s.peer_count as usize)
                        .sum();
                    state.network.total_peers = total_peers / 2;
                }
            })
        })
        .collect()
}

const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const LOG_BATCH_CAP: usize = 64;

/// Spawn a log listener for each node that pushes events into shared state.
///
/// Events are buffered locally and flushed in batches (every 100 ms or 64
/// events) to reduce mutex contention at high TPS.
pub(crate) fn spawn_log_listeners(
    nodes: &[ManagedNode],
    shared: Arc<Mutex<AppState>>,
    rt: &Runtime,
) -> Vec<tokio::task::JoinHandle<()>> {
    nodes
        .iter()
        .map(|mn| {
            let mut rx = mn.zode.subscribe_events();
            let node_id = mn.node_id;
            let shared = Arc::clone(&shared);
            rt.spawn(async move {
                let mut buf: Vec<AggregatedLogEntry> = Vec::with_capacity(LOG_BATCH_CAP);
                let mut flush_deadline = tokio::time::Instant::now() + LOG_FLUSH_INTERVAL;

                loop {
                    tokio::select! {
                        result = rx.recv() => {
                            match result {
                                Ok(event) => {
                                    let (line, level) = classify_event(&event);
                                    buf.push(AggregatedLogEntry {
                                        node_id,
                                        line,
                                        level,
                                        timestamp: std::time::Instant::now(),
                                    });
                                    if buf.len() >= LOG_BATCH_CAP {
                                        flush_logs(&shared, &mut buf).await;
                                        flush_deadline = tokio::time::Instant::now() + LOG_FLUSH_INTERVAL;
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(_) => break,
                            }
                        }
                        _ = tokio::time::sleep_until(flush_deadline) => {
                            if !buf.is_empty() {
                                flush_logs(&shared, &mut buf).await;
                            }
                            flush_deadline = tokio::time::Instant::now() + LOG_FLUSH_INTERVAL;
                        }
                    }
                }
                if !buf.is_empty() {
                    flush_logs(&shared, &mut buf).await;
                }
            })
        })
        .collect()
}

async fn flush_logs(shared: &Arc<Mutex<AppState>>, buf: &mut Vec<AggregatedLogEntry>) {
    let mut state = shared.lock().await;
    if state.log_entries.len() > 10_000 {
        state.log_entries.drain(0..5_000);
    }
    state.log_entries.append(buf);
}

fn classify_event(event: &LogEvent) -> (String, LogLevel) {
    let line = event.to_string();
    let level = match event {
        LogEvent::PeerConnected(_) | LogEvent::PeerDiscovered(_) | LogEvent::Started { .. } => {
            LogLevel::Info
        }
        LogEvent::PeerDisconnected(_) => LogLevel::Warn,
        LogEvent::ConnectionFailed { .. } | LogEvent::RelayFailed { .. } => LogLevel::Error,
        LogEvent::ShuttingDown => LogLevel::Warn,
        _ => LogLevel::Debug,
    };
    (line, level)
}

/// Gracefully shut down all nodes concurrently.
pub(crate) fn shutdown_all(nodes: &[ManagedNode], rt: &Runtime) {
    rt.block_on(async {
        let mut set = tokio::task::JoinSet::new();
        for mn in nodes {
            let zode = Arc::clone(&mn.zode);
            set.spawn(async move {
                zode.shutdown().await;
            });
        }
        while set.join_next().await.is_some() {}
    });
}
