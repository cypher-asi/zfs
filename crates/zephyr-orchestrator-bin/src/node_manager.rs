use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use grid_net::{Keypair, Multiaddr};
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

/// Build ZodeConfigs and launch all nodes for the given preset.
///
/// Node 0 starts first; we capture its listen address (including peer ID)
/// from the `LogEvent::Started` broadcast so subsequent nodes can bootstrap.
pub(crate) fn launch_network(
    preset: &NetworkPreset,
    rt: &Runtime,
    shared: &Arc<Mutex<AppState>>,
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
            let pk_bytes = kp.public().encode_protobuf();
            let mut vid = [0u8; 32];
            let len = pk_bytes.len().min(32);
            vid[..len].copy_from_slice(&pk_bytes[..len]);
            let mut pubkey = [0u8; 32];
            pubkey[..len].copy_from_slice(&pk_bytes[..len]);
            ValidatorInfo {
                validator_id: vid,
                pubkey,
                p2p_endpoint: String::new(),
            }
        })
        .collect();

    let zephyr_config = grid_services_zephyr::ZephyrConfig {
        total_zones,
        committee_size,
        epoch_duration_ms: 120_000,
        round_interval_ms: 500,
        quorum_threshold: ((2 * committee_size) / 3) + 1,
        max_batch_size: 64,
        initial_randomness: [0u8; 32],
        validators: validators.clone(),
        self_validate: false,
    };

    let zephyr_json = serde_json::to_value(&zephyr_config).expect("ZephyrConfig always serializes");

    let base_dir = std::env::temp_dir().join("zephyr-orchestrator");
    let _ = std::fs::create_dir_all(&base_dir);

    let mut managed_nodes = Vec::with_capacity(validator_count);
    let mut bootstrap_addr: Option<String> = None;

    for i in 0..validator_count {
        let kp = keypairs[i].clone();
        let vid = validators[i].validator_id;
        let zephyr_json = zephyr_json.clone();
        let base_dir = base_dir.clone();
        let boot = bootstrap_addr.clone();

        let result = rt.block_on(async {
            let data_dir = base_dir.join(format!("node-{i}"));
            let _ = std::fs::create_dir_all(&data_dir);

            let listen_addr: Multiaddr = "/ip4/127.0.0.1/udp/0/quic-v1"
                .parse()
                .expect("well-known constant multiaddr");

            let bootstrap_peers = match boot {
                Some(ref addr) => addr
                    .parse::<Multiaddr>()
                    .map(|ma| vec![ma])
                    .unwrap_or_default(),
                None => Vec::new(),
            };

            let net_config = grid_net::NetworkConfig {
                listen_addr,
                keypair: Some(kp),
                bootstrap_peers,
                ..Default::default()
            };

            let storage_config = grid_storage::StorageConfig::new(data_dir);

            let mut service_configs = HashMap::new();
            service_configs.insert("ZEPHYR".to_string(), zephyr_json);

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
                Ok(z) => Ok(Arc::new(z)),
                Err(e) => {
                    error!(node = i, error = %e, "failed to start node");
                    Err(e)
                }
            }
        });

        let Ok(zode) = result else {
            continue;
        };

        if i == 0 {
            let addr = rt.block_on(capture_listen_addr(&zode));
            if let Some(a) = addr {
                info!(addr = %a, "node 0 listen address captured for bootstrap");
                bootstrap_addr = Some(a);
            } else {
                warn!(
                    "could not capture node 0 listen address; subsequent nodes will not bootstrap"
                );
            }
        }

        managed_nodes.push(ManagedNode {
            zode,
            validator_id: vid,
            node_id: i,
        });
    }

    let shared_init = Arc::clone(shared);
    let node_count = managed_nodes.len();
    rt.block_on(async {
        let mut state = shared_init.lock().await;
        state.network = NetworkSnapshot {
            total_zones,
            ..Default::default()
        };
        state.nodes = (0..node_count).map(NodeState::new).collect();
    });

    managed_nodes
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
                            if let Some(zones) = zephyr.get("assigned_zones").and_then(|v| v.as_array()) {
                                ns.assigned_zones = zones.iter().filter_map(|z| z.as_u64().map(|n| n as u32)).collect();
                            }
                            if let Some(mp) = zephyr.get("mempool_sizes").and_then(|v| v.as_object()) {
                                ns.mempool_sizes = mp.iter()
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
                            if let Some(epoch) = zephyr.get("current_epoch").and_then(|v| v.as_u64()) {
                                state.network.current_epoch = epoch;
                            }
                            if let Some(pct) = zephyr.get("epoch_progress_pct").and_then(|v| v.as_f64()) {
                                state.network.epoch_progress_pct = pct as f32;
                            }
                            if let Some(certs) = zephyr.get("certificates_produced").and_then(|v| v.as_u64()) {
                                state.network.certificates_produced = certs;
                            }
                            if let Some(spends) = zephyr.get("spends_processed").and_then(|v| v.as_u64()) {
                                state.network.spends_processed = spends;
                            }
                            if let Some(heads) = zephyr.get("zone_heads").and_then(|v| v.as_object()) {
                                for (k, v) in heads {
                                    if let (Ok(zone_id), Some(hex_str)) = (k.parse::<u32>(), v.as_str()) {
                                        if let Ok(bytes) = hex::decode(hex_str) {
                                            let mut head = [0u8; 32];
                                            let len = bytes.len().min(32);
                                            head[..len].copy_from_slice(&bytes[..len]);
                                            state.network.zone_heads.insert(zone_id, head);
                                        }
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

/// Spawn a log listener for each node that pushes events into shared state.
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
                while let Ok(event) = rx.recv().await {
                    let (line, level) = classify_event(&event);
                    let entry = AggregatedLogEntry {
                        node_id,
                        line,
                        level,
                        timestamp: std::time::Instant::now(),
                    };
                    let mut state = shared.lock().await;
                    if state.log_entries.len() > 10_000 {
                        state.log_entries.drain(0..5_000);
                    }
                    state.log_entries.push(entry);
                }
            })
        })
        .collect()
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

/// Gracefully shut down all nodes.
pub(crate) fn shutdown_all(nodes: &[ManagedNode], rt: &Runtime) {
    for mn in nodes {
        let zode = Arc::clone(&mn.zode);
        rt.block_on(async move {
            zode.shutdown().await;
        });
    }
}
