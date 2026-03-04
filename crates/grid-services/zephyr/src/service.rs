use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use dashmap::DashMap;
use grid_core::ProgramId;
use grid_programs_zephyr::{
    ValidatorInfo, ZephyrConsensusDescriptor, ZephyrConsensusMessage, ZephyrGlobalDescriptor,
    ZephyrGlobalMessage, ZephyrSpendDescriptor, ZephyrValidatorDescriptor, ZephyrZoneDescriptor,
    ZephyrZoneMessage,
};
use grid_service::{
    ConfigField, ConfigFieldType, OwnedProgram, RouteInfo, Service, ServiceContext,
    ServiceDescriptor, ServiceError, ServiceGossipHandler,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::committee::my_assigned_zones;
use crate::config::ZephyrConfig;
use crate::epoch::EpochManager;
use crate::gossip::ZephyrGossipHandler;
use crate::shared_mempool::SharedMempool;

/// Summary of a finalized block for the metrics / dashboard feed.
pub(crate) struct BlockSummary {
    pub(crate) zone_id: u32,
    pub(crate) block_hash_hex: String,
    pub(crate) height: u64,
    pub(crate) tx_nullifiers: Vec<String>,
}

pub(crate) const MAX_RECENT_BLOCKS: usize = 100;

/// Live metrics snapshot shared between the consensus task and HTTP handlers.
pub(crate) struct ZephyrRuntime {
    pub zone_heads: HashMap<u32, [u8; 32]>,
    pub current_epoch: u64,
    pub epoch_progress_pct: f32,
    pub certificates_produced: u64,
    pub spends_processed: u64,
    pub mempool_sizes: HashMap<u32, usize>,
    pub assigned_zones: Vec<u32>,
    pub(crate) zone_heights: HashMap<u32, u64>,
    pub(crate) recent_blocks: VecDeque<BlockSummary>,
    pub(crate) blocks_produced: u64,
    pub(crate) zone_consecutive_timeouts: HashMap<u32, u32>,
    pub(crate) zone_last_advance: HashMap<u32, std::time::Instant>,
}

/// Shared state handed to HTTP route handlers.
pub(crate) struct ZephyrState {
    pub(crate) config: ZephyrConfig,
    pub(crate) global_program_id: ProgramId,
    pub(crate) zone_program_ids: Vec<ProgramId>,
    pub(crate) runtime: Arc<parking_lot::RwLock<ZephyrRuntime>>,
}

/// The Zephyr currency service.
///
/// Implements zone-scoped BFT consensus for a note-based currency on GRID.
/// Lifecycle:
/// - `on_start`: subscribes to global + assigned zone topics, initializes
///   epoch manager, spawns consensus tasks
/// - `on_stop`: cancels all tasks via the shutdown token, unsubscribes topics
pub struct ZephyrService {
    descriptor: ServiceDescriptor,
    config: ZephyrConfig,
    global_program_id: ProgramId,
    zone_program_ids: Vec<ProgramId>,
    consensus_program_ids: Vec<ProgramId>,
    runtime: Arc<parking_lot::RwLock<ZephyrRuntime>>,
    gossip_handler: Arc<ZephyrGossipHandler>,
    consensus_proposal_rx:
        std::sync::Mutex<Option<mpsc::Receiver<(String, ZephyrConsensusMessage)>>>,
    consensus_vote_rx: std::sync::Mutex<Option<mpsc::Receiver<(String, ZephyrConsensusMessage)>>>,
    zone_rx: std::sync::Mutex<Option<mpsc::Receiver<(String, ZephyrZoneMessage)>>>,
    global_rx: std::sync::Mutex<Option<mpsc::Receiver<ZephyrGlobalMessage>>>,
}

impl ZephyrService {
    pub fn new(config: ZephyrConfig) -> Result<Self, ServiceError> {
        let global_pid = ZephyrGlobalDescriptor::new()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        let mut zone_pids = Vec::with_capacity(config.total_zones as usize);
        let mut consensus_pids = Vec::with_capacity(config.total_zones as usize);
        for zone_id in 0..config.total_zones {
            let pid = ZephyrZoneDescriptor::new(zone_id)
                .program_id()
                .map_err(|e| ServiceError::Descriptor(e.to_string()))?;
            zone_pids.push(pid);
            let cpid = ZephyrConsensusDescriptor::new(zone_id)
                .program_id()
                .map_err(|e| ServiceError::Descriptor(e.to_string()))?;
            consensus_pids.push(cpid);
        }

        let spend_pid = ZephyrSpendDescriptor::new()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;
        let validator_pid = ZephyrValidatorDescriptor::new()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        let mut owned_programs = vec![
            OwnedProgram {
                name: "zephyr/global".into(),
                version: "1".into(),
                program_id: global_pid,
            },
            OwnedProgram {
                name: "zephyr/spend".into(),
                version: "1".into(),
                program_id: spend_pid,
            },
            OwnedProgram {
                name: "zephyr/validators".into(),
                version: "1".into(),
                program_id: validator_pid,
            },
        ];
        for (i, pid) in zone_pids.iter().enumerate() {
            owned_programs.push(OwnedProgram {
                name: format!("zephyr/zone-{i}"),
                version: "1".into(),
                program_id: *pid,
            });
        }
        for (i, cpid) in consensus_pids.iter().enumerate() {
            owned_programs.push(OwnedProgram {
                name: format!("zephyr/zone_consensus-{i}"),
                version: "1".into(),
                program_id: *cpid,
            });
        }

        let global_topic = grid_core::program_topic(&global_pid);
        let (proposal_tx, consensus_proposal_rx) = mpsc::channel(2048);
        let (vote_tx, consensus_vote_rx) = mpsc::channel(16_384);
        let (zone_tx, zone_rx) = mpsc::channel(65_536);
        let (global_tx, global_rx) = mpsc::channel(4096);
        let gossip_handler = Arc::new(ZephyrGossipHandler::new(
            global_topic,
            proposal_tx,
            vote_tx,
            zone_tx,
            global_tx,
        ));

        Ok(Self {
            descriptor: ServiceDescriptor {
                name: "ZEPHYR".into(),
                version: "0.1.0".into(),
                required_programs: vec![],
                owned_programs,
                summary: "Note-based currency with zone-scoped consensus.".into(),
            },
            config,
            global_program_id: global_pid,
            zone_program_ids: zone_pids,
            consensus_program_ids: consensus_pids,
            runtime: Arc::new(parking_lot::RwLock::new(ZephyrRuntime {
                zone_heads: HashMap::new(),
                current_epoch: 0,
                epoch_progress_pct: 0.0,
                certificates_produced: 0,
                spends_processed: 0,
                mempool_sizes: HashMap::new(),
                assigned_zones: Vec::new(),
                zone_heights: HashMap::new(),
                recent_blocks: VecDeque::new(),
                blocks_produced: 0,
                zone_consecutive_timeouts: HashMap::new(),
                zone_last_advance: HashMap::new(),
            })),
            gossip_handler,
            consensus_proposal_rx: std::sync::Mutex::new(Some(consensus_proposal_rx)),
            consensus_vote_rx: std::sync::Mutex::new(Some(consensus_vote_rx)),
            zone_rx: std::sync::Mutex::new(Some(zone_rx)),
            global_rx: std::sync::Mutex::new(Some(global_rx)),
        })
    }

    pub fn config(&self) -> &ZephyrConfig {
        &self.config
    }

    pub fn global_program_id(&self) -> &ProgramId {
        &self.global_program_id
    }

    pub fn zone_program_ids(&self) -> &[ProgramId] {
        &self.zone_program_ids
    }

    fn global_topic(&self) -> String {
        grid_core::program_topic(&self.global_program_id)
    }

    fn zone_topic(&self, zone_id: u32) -> String {
        grid_core::program_topic(&self.zone_program_ids[zone_id as usize])
    }

    fn consensus_topic(&self, zone_id: u32) -> String {
        grid_core::program_topic(&self.consensus_program_ids[zone_id as usize])
    }
}

#[async_trait]
impl Service for ZephyrService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn routes(&self, _ctx: &ServiceContext) -> Router {
        let state = Arc::new(ZephyrState {
            config: self.config.clone(),
            global_program_id: self.global_program_id,
            zone_program_ids: self.zone_program_ids.clone(),
            runtime: Arc::clone(&self.runtime),
        });

        Router::new()
            .route("/status", get(status_handler))
            .route("/zone/{id}/head", get(zone_head_handler))
            .route("/epoch/current", get(epoch_handler))
            .route("/health", get(health_handler))
            .with_state(state)
    }

    async fn on_start(&self, ctx: &ServiceContext) -> Result<(), ServiceError> {
        let global_topic = self.global_topic();
        ctx.subscribe_topic(&global_topic)?;
        info!(%global_topic, "subscribed to global topic");

        let mut validators = self.config.validators.clone();

        if self.config.self_validate && validators.is_empty() {
            if let Some(id) = ctx.identity() {
                let pk_bytes = id.public_key();
                let mut vid = [0u8; 32];
                let copy_len = pk_bytes.len().min(32);
                vid[..copy_len].copy_from_slice(&pk_bytes[..copy_len]);

                let mut pubkey = [0u8; 32];
                pubkey[..copy_len].copy_from_slice(&pk_bytes[..copy_len]);

                validators.push(ValidatorInfo {
                    validator_id: vid,
                    pubkey,
                    p2p_endpoint: id.zode_id().to_string(),
                });
                info!("self_validate enabled; running as solo validator");
            } else {
                warn!("self_validate enabled but no node identity available");
            }
        }

        if validators.is_empty() {
            warn!("no validators configured; Zephyr running in observer mode");
            return Ok(());
        }

        let node_identity = match ctx.identity_arc() {
            Some(id) => id,
            None => {
                warn!("no node identity; Zephyr running in observer mode");
                return Ok(());
            }
        };

        let my_validator_id = {
            let pk_bytes = node_identity.public_key();
            let mut vid = [0u8; 32];
            let copy_len = pk_bytes.len().min(32);
            vid[..copy_len].copy_from_slice(&pk_bytes[..copy_len]);
            vid
        };

        let epoch_mgr = EpochManager::new(
            0,
            self.config.epoch_duration_ms,
            self.config.initial_randomness,
            validators.clone(),
            self.config.total_zones,
            self.config.committee_size,
        );

        let assigned_zones = my_assigned_zones(
            &my_validator_id,
            epoch_mgr.randomness_seed(),
            &validators,
            self.config.total_zones,
            self.config.committee_size,
        );

        // Register ALL zone + consensus topics in the gossip handler so it can
        // decode messages for any zone, but only subscribe to GossipSub topics
        // for zones this node is actually assigned to. Epoch transitions
        // dynamically subscribe/unsubscribe as assignments change.
        let mut topic_to_zone: HashMap<String, u32> = HashMap::new();
        let mut consensus_topic_to_zone: HashMap<String, u32> = HashMap::new();
        for zone_id in 0..self.config.total_zones {
            let topic = self.zone_topic(zone_id);
            self.gossip_handler.add_zone_topic(topic.clone());
            topic_to_zone.insert(topic.clone(), zone_id);

            let ctopic = self.consensus_topic(zone_id);
            self.gossip_handler.add_consensus_topic(ctopic.clone());
            consensus_topic_to_zone.insert(ctopic.clone(), zone_id);

            if assigned_zones.contains(&zone_id) {
                ctx.subscribe_topic(&topic)?;
                ctx.subscribe_topic(&ctopic)?;
                info!(zone_id, %topic, %ctopic, "subscribed to zone + consensus topics (assigned)");
            } else {
                debug!(zone_id, %topic, %ctopic, "registered zone + consensus topics (not assigned)");
            }
        }

        // Take channel receivers (one-time)
        let consensus_proposal_rx = self
            .consensus_proposal_rx
            .lock()
            .map_err(|e| ServiceError::Other(format!("lock poisoned: {e}")))?
            .take()
            .ok_or_else(|| ServiceError::Other("consensus_proposal_rx already taken".into()))?;

        let consensus_vote_rx = self
            .consensus_vote_rx
            .lock()
            .map_err(|e| ServiceError::Other(format!("lock poisoned: {e}")))?
            .take()
            .ok_or_else(|| ServiceError::Other("consensus_vote_rx already taken".into()))?;

        let zone_rx = self
            .zone_rx
            .lock()
            .map_err(|e| ServiceError::Other(format!("lock poisoned: {e}")))?
            .take()
            .ok_or_else(|| ServiceError::Other("zone_rx already taken".into()))?;

        let global_rx = self
            .global_rx
            .lock()
            .map_err(|e| ServiceError::Other(format!("lock poisoned: {e}")))?
            .take()
            .ok_or_else(|| ServiceError::Other("global_rx already taken".into()))?;

        // Update runtime with initial state
        {
            let mut rt = self.runtime.write();
            rt.assigned_zones = assigned_zones.clone();
            rt.current_epoch = 0;
        }

        // Clone what we need for the spawned tasks
        let runtime = Arc::clone(&self.runtime);
        let config = self.config.clone();
        let shutdown = ctx.shutdown.clone();
        let publish_tx = ctx
            .publish_sender()
            .ok_or_else(|| ServiceError::NotInitialized("publish channel not set".into()))?;
        let topic_tx = ctx
            .topic_sender()
            .ok_or_else(|| ServiceError::NotInitialized("topic channel not set".into()))?;
        let global_topic_for_task = self.global_topic();

        // Shared mempool between ingest and consensus tasks
        let mempool = SharedMempool::new();
        for zone_id in 0..self.config.total_zones {
            mempool.add_zone(zone_id, 65_536);
        }

        // Spawn the ingest task (spend submissions only)
        // Proof verifier is None when skip_proof_verification is set.
        let ingest_verifier: Option<Arc<crate::proof::SpendProofVerifier>> = None;
        tokio::spawn(ingest_loop(
            zone_rx,
            topic_to_zone.clone(),
            mempool.clone(),
            ingest_verifier,
            shutdown.clone(),
        ));

        // Per-zone heads: lock-free concurrent map replaces the old shared Mutex<ZoneHead>
        let zone_head_store: Arc<DashMap<u32, [u8; 32]>> = Arc::new(DashMap::new());
        let epoch_mgr = Arc::new(tokio::sync::Mutex::new(epoch_mgr));

        // Per-zone channels and tasks
        let mut zone_proposal_txs = HashMap::new();
        let mut zone_vote_txs = HashMap::new();
        let mut zone_global_txs = HashMap::new();

        for zone_id in 0..self.config.total_zones {
            let (prop_tx, prop_rx) = mpsc::channel(2048);
            let (vote_tx, vote_rx) = mpsc::channel(16_384);
            let (glob_tx, glob_rx) = mpsc::channel(4096);
            zone_proposal_txs.insert(zone_id, prop_tx);
            zone_vote_txs.insert(zone_id, vote_tx);
            zone_global_txs.insert(zone_id, glob_tx);

            let is_assigned = assigned_zones.contains(&zone_id);
            let zt = self.zone_topic(zone_id);
            let ct = self.consensus_topic(zone_id);

            let nullifier_set =
                crate::storage::NullifierSet::in_memory(zone_id);

            let mut zone_state = crate::zone_task::ZoneTaskState {
                zone_id,
                engine: None,
                pending_certs: crate::consensus::PendingCertBuffer::new(
                    config.max_pending_certs,
                ),
                block_tx_cache: HashMap::new(),
                block_nullifiers: HashMap::new(),
                deferred_cleanups: HashMap::new(),
                last_buffered_proposal: None,
                last_known_epoch: 0,
                validators: validators.clone(),
                my_validator_id,
                identity: Arc::clone(&node_identity),
                config: config.clone(),
                publish_tx: publish_tx.clone(),
                topic_tx: topic_tx.clone(),
                zone_topic: zt,
                consensus_topic: ct,
                global_topic: global_topic_for_task.clone(),
                zone_head_store: zone_head_store.clone(),
                mempool: mempool.clone(),
                runtime: runtime.clone(),
                epoch_mgr: epoch_mgr.clone(),
                nullifier_set,
                proof_verifier: None,
            };

            if is_assigned {
                let em = epoch_mgr.lock().await;
                let committee = crate::committee::sample_committee(
                    em.randomness_seed(),
                    zone_id,
                    &validators,
                    config.committee_size,
                );
                let prev_head = zone_head_store
                    .get(&zone_id)
                    .map(|v| *v)
                    .unwrap_or([0u8; 32]);
                zone_state.engine = Some(crate::consensus::ZoneConsensus::new(
                    zone_id,
                    0,
                    committee,
                    my_validator_id,
                    prev_head,
                    config.clone(),
                    zone_id as usize,
                ));
                drop(em);
            }

            let sd = shutdown.clone();
            tokio::spawn(async move {
                zone_state.run(prop_rx, vote_rx, glob_rx, sd).await;
            });
        }

        // Spawn the dispatcher (fan-out from shared channels to per-zone channels)
        tokio::spawn(consensus_dispatcher(
            consensus_proposal_rx,
            consensus_vote_rx,
            global_rx,
            consensus_topic_to_zone,
            zone_proposal_txs,
            zone_vote_txs,
            zone_global_txs,
            shutdown.clone(),
        ));

        info!(
            zones = self.config.total_zones,
            committee_size = self.config.committee_size,
            "Zephyr service started with per-zone consensus tasks"
        );

        Ok(())
    }

    async fn on_stop(&self) -> Result<(), ServiceError> {
        info!("Zephyr service stopped");
        Ok(())
    }

    fn route_info(&self) -> Vec<RouteInfo> {
        vec![
            RouteInfo {
                method: "GET",
                path: "/status",
                description: "Overall Zephyr status (epoch, zones, validator count)",
            },
            RouteInfo {
                method: "GET",
                path: "/zone/:id/head",
                description: "Current zone head hash",
            },
            RouteInfo {
                method: "GET",
                path: "/epoch/current",
                description: "Current epoch info",
            },
            RouteInfo {
                method: "GET",
                path: "/health",
                description: "Health check",
            },
        ]
    }

    fn config_schema(&self) -> Vec<ConfigField> {
        vec![ConfigField {
            key: "self_validate",
            label: "Participate as validator",
            description: "Run this node as a solo validator using its own identity",
            field_type: ConfigFieldType::Bool { default: false },
        }]
    }

    fn current_config(&self) -> serde_json::Value {
        serde_json::json!({
            "self_validate": self.config.self_validate,
        })
    }

    fn gossip_handler(&self) -> Option<Arc<dyn ServiceGossipHandler>> {
        Some(Arc::clone(&self.gossip_handler) as _)
    }

    fn metrics(&self) -> serde_json::Value {
        let rt = self.runtime.read();
        serde_json::json!({
            "zone_heads": rt.zone_heads.iter()
                .map(|(k, v)| (k.to_string(), hex::encode(&v[..8])))
                .collect::<HashMap<_, _>>(),
            "current_epoch": rt.current_epoch,
            "epoch_progress_pct": rt.epoch_progress_pct,
            "certificates_produced": rt.certificates_produced,
            "spends_processed": rt.spends_processed,
            "mempool_sizes": rt.mempool_sizes,
            "assigned_zones": rt.assigned_zones,
            "blocks_produced": rt.blocks_produced,
            "zone_heights": rt.zone_heights.iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<HashMap<_, _>>(),
            "recent_blocks": rt.recent_blocks.iter().map(|b| {
                serde_json::json!({
                    "zone_id": b.zone_id,
                    "block_hash": &b.block_hash_hex,
                    "height": b.height,
                    "tx_nullifiers": &b.tx_nullifiers,
                })
            }).collect::<Vec<_>>(),
            "zone_consecutive_timeouts": rt.zone_consecutive_timeouts,
            "zone_stall_durations_ms": rt.zone_last_advance.iter()
                .map(|(k, v)| (k.to_string(), v.elapsed().as_millis() as u64))
                .collect::<HashMap<_, _>>(),
        })
    }
}

/// Spend ingestion task -- runs concurrently with the consensus loop.
///
/// Receives spend submissions from gossip and inserts them into the shared
/// mempool. This decouples high-volume transaction ingestion from
/// latency-sensitive consensus round-trips.
///
/// When a `SpendProofVerifier` is provided, each spend proof is verified
/// before mempool insertion; invalid spends are silently dropped.
async fn ingest_loop(
    mut zone_rx: mpsc::Receiver<(String, ZephyrZoneMessage)>,
    topic_to_zone: HashMap<String, u32>,
    mempool: SharedMempool,
    proof_verifier: Option<Arc<crate::proof::SpendProofVerifier>>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    loop {
        tokio::select! {
            biased;

            _ = shutdown.cancelled() => {
                debug!("ingest loop shutting down");
                break;
            }

            msg = zone_rx.recv() => {
                let Some(first) = msg else { break };

                let mut batch = Vec::with_capacity(1025);
                batch.push(first);
                while batch.len() < 1024 {
                    match zone_rx.try_recv() {
                        Ok(m) => batch.push(m),
                        Err(_) => break,
                    }
                }

                let mut zone_buckets: HashMap<u32, Vec<grid_programs_zephyr::SpendTransaction>> =
                    HashMap::new();
                for (topic, msg) in batch {
                    let Some(&zone_id) = topic_to_zone.get(&topic) else {
                        continue;
                    };
                    match msg {
                        ZephyrZoneMessage::SubmitSpend(tx) => {
                            zone_buckets.entry(zone_id).or_default().push(tx);
                        }
                        ZephyrZoneMessage::SubmitSpendBatch(txs) => {
                            zone_buckets.entry(zone_id).or_default().extend(txs);
                        }
                    }
                }
                for (zone_id, txs) in zone_buckets {
                    let verified = if let Some(ref verifier) = proof_verifier {
                        txs.into_iter()
                            .filter(|tx| verify_spend_proof(verifier, tx))
                            .collect()
                    } else {
                        txs
                    };
                    mempool.insert_batch(zone_id, verified);
                }
            }
        }
    }
}

/// Verify a single spend transaction's Groth16 proof.
fn verify_spend_proof(
    verifier: &crate::proof::SpendProofVerifier,
    tx: &grid_programs_zephyr::SpendTransaction,
) -> bool {
    let signals: Result<Vec<ark_bn254::Fr>, _> = tx
        .public_signals
        .iter()
        .map(|b| {
            use ark_serialize::CanonicalDeserialize;
            ark_bn254::Fr::deserialize_compressed(&b[..]).map_err(|_| ())
        })
        .collect();
    let Ok(signals) = signals else {
        return false;
    };
    verifier.verify(&tx.proof, &signals).is_ok()
}

/// Lightweight fan-out task: reads from the shared consensus and global
/// channels and routes each message to the correct zone task's per-zone
/// channel based on topic â†’ zone_id mapping (consensus) or cert.zone_id
/// (global certificates).
async fn consensus_dispatcher(
    mut proposal_rx: mpsc::Receiver<(String, ZephyrConsensusMessage)>,
    mut vote_rx: mpsc::Receiver<(String, ZephyrConsensusMessage)>,
    mut global_rx: mpsc::Receiver<ZephyrGlobalMessage>,
    consensus_topic_to_zone: HashMap<String, u32>,
    zone_proposal_txs: HashMap<u32, mpsc::Sender<ZephyrConsensusMessage>>,
    zone_vote_txs: HashMap<u32, mpsc::Sender<ZephyrConsensusMessage>>,
    zone_global_txs: HashMap<u32, mpsc::Sender<ZephyrGlobalMessage>>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    loop {
        tokio::select! {
            biased;

            _ = shutdown.cancelled() => {
                debug!("consensus dispatcher shutting down");
                break;
            }

            msg = proposal_rx.recv() => {
                let Some((topic, cmsg)) = msg else { break };
                let Some(&zone_id) = consensus_topic_to_zone.get(&topic) else {
                    warn!(%topic, "dispatcher: unknown consensus topic");
                    continue;
                };
                if let Some(tx) = zone_proposal_txs.get(&zone_id) {
                    if tx.try_send(cmsg).is_err() {
                        warn!(
                            zone_id,
                            capacity = tx.capacity(),
                            "zone proposal channel full, dropping proposal"
                        );
                    }
                }
            }

            msg = vote_rx.recv() => {
                let Some((topic, cmsg)) = msg else { break };
                let Some(&zone_id) = consensus_topic_to_zone.get(&topic) else {
                    warn!(%topic, "dispatcher: unknown consensus topic");
                    continue;
                };
                if let Some(tx) = zone_vote_txs.get(&zone_id) {
                    if tx.try_send(cmsg).is_err() {
                        warn!(
                            zone_id,
                            capacity = tx.capacity(),
                            "zone vote channel full, dropping vote"
                        );
                    }
                }
            }

            msg = global_rx.recv() => {
                let Some(gmsg) = msg else { break };
                match &gmsg {
                    ZephyrGlobalMessage::Certificate { cert, .. } => {
                        let zone_id = cert.zone_id;
                        if let Some(tx) = zone_global_txs.get(&zone_id) {
                            if let Err(e) = tx.try_send(gmsg) {
                                warn!(
                                    zone_id,
                                    capacity = tx.capacity(),
                                    "zone global channel full, dropping certificate: {e}"
                                );
                            }
                        }
                    }
                    ZephyrGlobalMessage::EpochAnnounce(ann) => {
                        debug!(epoch = ann.epoch, "received epoch announcement");
                    }
                }
            }
        }
    }
}

// --- HTTP Handlers ---

async fn status_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    let rt = state.runtime.read();
    Json(serde_json::json!({
        "service": "ZEPHYR",
        "total_zones": state.config.total_zones,
        "committee_size": state.config.committee_size,
        "validator_count": state.config.validators.len(),
        "global_program_id": state.global_program_id.to_hex(),
        "current_epoch": rt.current_epoch,
        "certificates_produced": rt.certificates_produced,
        "spends_processed": rt.spends_processed,
    }))
}

async fn zone_head_handler(
    State(state): State<Arc<ZephyrState>>,
    Path(id): Path<u32>,
) -> impl IntoResponse {
    if id >= state.config.total_zones {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "zone not found" })),
        )
            .into_response();
    }
    let pid = &state.zone_program_ids[id as usize];
    let rt = state.runtime.read();
    let head = rt.zone_heads.get(&id).map(hex::encode);
    Json(serde_json::json!({
        "zone_id": id,
        "program_id": pid.to_hex(),
        "head": head,
    }))
    .into_response()
}

async fn epoch_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    let rt = state.runtime.read();
    Json(serde_json::json!({
        "epoch": rt.current_epoch,
        "epoch_duration_ms": state.config.epoch_duration_ms,
        "epoch_progress_pct": rt.epoch_progress_pct,
        "total_zones": state.config.total_zones,
        "committee_size": state.config.committee_size,
    }))
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_succeeds_with_default_config() {
        let svc = ZephyrService::new(ZephyrConfig::default()).unwrap();
        assert_eq!(svc.descriptor().name, "ZEPHYR");
        assert_eq!(svc.descriptor().version, "0.1.0");
    }

    #[test]
    fn zone_program_ids_match_zone_count() {
        let config = ZephyrConfig {
            total_zones: 4,
            ..ZephyrConfig::default()
        };
        let svc = ZephyrService::new(config).unwrap();
        assert_eq!(svc.zone_program_ids().len(), 4);
    }

    #[test]
    fn route_info_contains_expected_paths() {
        let svc = ZephyrService::new(ZephyrConfig::default()).unwrap();
        let routes = svc.route_info();
        assert_eq!(routes.len(), 4);
        assert!(routes.iter().any(|r| r.path == "/health"));
        assert!(routes.iter().any(|r| r.path == "/status"));
    }

    #[test]
    fn global_program_id_is_deterministic() {
        let svc1 = ZephyrService::new(ZephyrConfig::default()).unwrap();
        let svc2 = ZephyrService::new(ZephyrConfig::default()).unwrap();
        assert_eq!(svc1.global_program_id(), svc2.global_program_id());
    }

    #[test]
    fn global_topic_format() {
        let svc = ZephyrService::new(ZephyrConfig::default()).unwrap();
        let topic = svc.global_topic();
        assert!(topic.starts_with("prog/"));
        assert_eq!(topic.len(), 5 + 64);
    }

    #[test]
    fn zone_topics_are_distinct() {
        let config = ZephyrConfig {
            total_zones: 4,
            ..ZephyrConfig::default()
        };
        let svc = ZephyrService::new(config).unwrap();
        let topics: Vec<String> = (0..4).map(|z| svc.zone_topic(z)).collect();
        for (i, a) in topics.iter().enumerate() {
            for (j, b) in topics.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "zone {i} and {j} should have distinct topics");
                }
            }
        }
    }

    #[test]
    fn gossip_handler_is_some() {
        let svc = ZephyrService::new(ZephyrConfig::default()).unwrap();
        assert!(svc.gossip_handler().is_some());
    }

    #[test]
    fn metrics_returns_valid_json() {
        let svc = ZephyrService::new(ZephyrConfig::default()).unwrap();
        let m = svc.metrics();
        assert!(m.is_object());
        assert_eq!(m["current_epoch"], 0);
    }

}
