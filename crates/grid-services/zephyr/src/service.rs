use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_zephyr::{
    Block, FinalityCertificate, Nullifier, ValidatorInfo, ZephyrConsensusDescriptor,
    ZephyrConsensusMessage, ZephyrGlobalDescriptor, ZephyrGlobalMessage, ZephyrSpendDescriptor,
    ZephyrValidatorDescriptor, ZephyrZoneDescriptor, ZephyrZoneMessage,
};
use grid_service::{
    ConfigField, ConfigFieldType, OwnedProgram, RouteInfo, Service, ServiceContext,
    ServiceDescriptor, ServiceError, ServiceGossipHandler,
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::committee::{my_assigned_zones, sample_committee};
use crate::config::ZephyrConfig;
use crate::consensus::{ConsensusAction, ZoneConsensus};
use crate::epoch::EpochManager;
use crate::gossip::ZephyrGossipHandler;
use crate::shared_mempool::SharedMempool;
use crate::storage::zone_head::ZoneHead;

/// Summary of a finalized block for the metrics / dashboard feed.
struct BlockSummary {
    zone_id: u32,
    block_hash_hex: String,
    height: u64,
    tx_nullifiers: Vec<String>,
}

const MAX_RECENT_BLOCKS: usize = 100;
const MAX_BLOCK_TX_CACHE: usize = 200;

/// Live metrics snapshot shared between the consensus task and HTTP handlers.
pub(crate) struct ZephyrRuntime {
    pub zone_heads: HashMap<u32, [u8; 32]>,
    pub current_epoch: u64,
    pub epoch_progress_pct: f32,
    pub certificates_produced: u64,
    pub spends_processed: u64,
    pub mempool_sizes: HashMap<u32, usize>,
    pub assigned_zones: Vec<u32>,
    zone_heights: HashMap<u32, u64>,
    recent_blocks: VecDeque<BlockSummary>,
    blocks_produced: u64,
    zone_consecutive_timeouts: HashMap<u32, u32>,
    zone_last_advance: HashMap<u32, std::time::Instant>,
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
    consensus_rx: std::sync::Mutex<Option<mpsc::Receiver<(String, ZephyrConsensusMessage)>>>,
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
        let (consensus_tx, consensus_rx) = mpsc::channel(4096);
        let (zone_tx, zone_rx) = mpsc::channel(65_536);
        let (global_tx, global_rx) = mpsc::channel(1024);
        let gossip_handler = Arc::new(ZephyrGossipHandler::new(
            global_topic,
            consensus_tx,
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
            consensus_rx: std::sync::Mutex::new(Some(consensus_rx)),
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

/// HMAC-SHA256 signing using the validator ID as key (sufficient for local testbed).
fn hmac_sign(validator_id: &[u8; 32], data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(validator_id);
    hasher.update(data);
    hasher.finalize().to_vec()
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

        let my_validator_id = match ctx.identity() {
            Some(id) => {
                let mut vid = [0u8; 32];
                let pk_bytes = id.public_key();
                let copy_len = pk_bytes.len().min(32);
                vid[..copy_len].copy_from_slice(&pk_bytes[..copy_len]);
                vid
            }
            None => {
                warn!("no node identity; Zephyr running in observer mode");
                return Ok(());
            }
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

        // Subscribe to ALL zone + consensus topics up-front so that epoch
        // transitions (which may reassign zones) don't require dynamic topic
        // management. Zone topics carry spends; consensus topics carry
        // proposals/votes/rejects on a separate GossipSub channel.
        let mut topic_to_zone: HashMap<String, u32> = HashMap::new();
        let mut consensus_topic_to_zone: HashMap<String, u32> = HashMap::new();
        for zone_id in 0..self.config.total_zones {
            let topic = self.zone_topic(zone_id);
            self.gossip_handler.add_zone_topic(topic.clone());
            ctx.subscribe_topic(&topic)?;
            topic_to_zone.insert(topic.clone(), zone_id);

            let ctopic = self.consensus_topic(zone_id);
            self.gossip_handler.add_consensus_topic(ctopic.clone());
            ctx.subscribe_topic(&ctopic)?;
            consensus_topic_to_zone.insert(ctopic.clone(), zone_id);

            info!(zone_id, %topic, %ctopic, "subscribed to zone + consensus topics");
        }

        // Take channel receivers (one-time)
        let consensus_rx = self
            .consensus_rx
            .lock()
            .map_err(|e| ServiceError::Other(format!("lock poisoned: {e}")))?
            .take()
            .ok_or_else(|| ServiceError::Other("consensus_rx already taken".into()))?;

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
        let global_topic_for_task = self.global_topic();

        // Shared mempool between ingest and consensus tasks
        let mempool = SharedMempool::new();
        for zone_id in 0..self.config.total_zones {
            mempool.add_zone(zone_id, 65_536);
        }

        // Spawn the ingest task (spend submissions only)
        tokio::spawn(ingest_loop(
            zone_rx,
            topic_to_zone.clone(),
            mempool.clone(),
            shutdown.clone(),
        ));

        // Shared cross-zone state protected by async mutexes
        let zone_head_store = Arc::new(tokio::sync::Mutex::new(ZoneHead::new()));
        let epoch_mgr = Arc::new(tokio::sync::Mutex::new(epoch_mgr));

        // Per-zone channels and tasks
        let mut zone_consensus_txs = HashMap::new();
        let mut zone_global_txs = HashMap::new();

        for zone_id in 0..self.config.total_zones {
            let (cons_tx, cons_rx) = mpsc::channel(4096);
            let (glob_tx, glob_rx) = mpsc::channel(1024);
            zone_consensus_txs.insert(zone_id, cons_tx);
            zone_global_txs.insert(zone_id, glob_tx);

            let is_assigned = assigned_zones.contains(&zone_id);
            let zt = self.zone_topic(zone_id);
            let ct = self.consensus_topic(zone_id);

            tokio::spawn(zone_consensus_task(
                zone_id,
                is_assigned,
                my_validator_id,
                validators.clone(),
                config.clone(),
                runtime.clone(),
                cons_rx,
                glob_rx,
                publish_tx.clone(),
                zt,
                ct,
                global_topic_for_task.clone(),
                shutdown.clone(),
                epoch_mgr.clone(),
                mempool.clone(),
                zone_head_store.clone(),
            ));
        }

        // Spawn the dispatcher (fan-out from shared channels to per-zone channels)
        tokio::spawn(consensus_dispatcher(
            consensus_rx,
            global_rx,
            consensus_topic_to_zone,
            zone_consensus_txs,
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
async fn ingest_loop(
    mut zone_rx: mpsc::Receiver<(String, ZephyrZoneMessage)>,
    topic_to_zone: HashMap<String, u32>,
    mempool: SharedMempool,
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
                    mempool.insert_batch(zone_id, txs);
                }
            }
        }
    }
}

/// Lightweight fan-out task: reads from the shared consensus and global
/// channels and routes each message to the correct zone task's per-zone
/// channel based on topic → zone_id mapping (consensus) or cert.zone_id
/// (global certificates).
async fn consensus_dispatcher(
    mut consensus_rx: mpsc::Receiver<(String, ZephyrConsensusMessage)>,
    mut global_rx: mpsc::Receiver<ZephyrGlobalMessage>,
    consensus_topic_to_zone: HashMap<String, u32>,
    zone_consensus_txs: HashMap<u32, mpsc::Sender<ZephyrConsensusMessage>>,
    zone_global_txs: HashMap<u32, mpsc::Sender<ZephyrGlobalMessage>>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                debug!("consensus dispatcher shutting down");
                break;
            }

            msg = consensus_rx.recv() => {
                let Some((topic, cmsg)) = msg else { break };
                let Some(&zone_id) = consensus_topic_to_zone.get(&topic) else {
                    warn!(%topic, "dispatcher: unknown consensus topic");
                    continue;
                };
                if let Some(tx) = zone_consensus_txs.get(&zone_id) {
                    let msg_type = match &cmsg {
                        ZephyrConsensusMessage::Proposal(_) => "Proposal",
                        ZephyrConsensusMessage::Vote(_) => "Vote",
                        ZephyrConsensusMessage::Reject(_) => "Reject",
                    };
                    if let Err(_) = tx.try_send(cmsg) {
                        warn!(
                            zone_id,
                            msg_type,
                            capacity = tx.capacity(),
                            "zone consensus channel full, dropping message"
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

/// Independent consensus event loop for a single zone.
///
/// Each zone task owns its own round timer, consensus engine, and local
/// caches (`block_tx_cache`, `block_nullifiers`, `deferred_cleanups`).
/// Shared cross-zone state (`ZoneHead`, `EpochManager`) is protected by
/// async mutexes with near-zero contention since each zone task only
/// writes to its own zone_id entry.
#[allow(clippy::too_many_arguments)]
async fn zone_consensus_task(
    zone_id: u32,
    initially_assigned: bool,
    my_validator_id: [u8; 32],
    validators: Vec<ValidatorInfo>,
    config: ZephyrConfig,
    runtime: Arc<parking_lot::RwLock<ZephyrRuntime>>,
    mut consensus_rx: mpsc::Receiver<ZephyrConsensusMessage>,
    mut global_rx: mpsc::Receiver<ZephyrGlobalMessage>,
    publish_tx: mpsc::Sender<(String, Vec<u8>)>,
    _zone_topic: String,
    consensus_topic: String,
    global_topic: String,
    shutdown: tokio_util::sync::CancellationToken,
    epoch_mgr: Arc<tokio::sync::Mutex<EpochManager>>,
    mempool: SharedMempool,
    zone_head_store: Arc<tokio::sync::Mutex<ZoneHead>>,
) {
    let mut engine: Option<ZoneConsensus> = None;
    let mut block_tx_cache: HashMap<[u8; 32], (u32, Vec<String>)> = HashMap::new();
    let mut block_nullifiers: HashMap<[u8; 32], (u32, Vec<Nullifier>)> = HashMap::new();
    let mut deferred_cleanups: HashMap<[u8; 32], u32> = HashMap::new();
    let mut pending_certs: Vec<FinalityCertificate> = Vec::new();
    let mut last_buffered_proposal: Option<Block> = None;
    let mut last_known_epoch: u64 = 0;

    if initially_assigned {
        let em = epoch_mgr.lock().await;
        let committee = sample_committee(
            em.randomness_seed(),
            zone_id,
            &validators,
            config.committee_size,
        );
        let prev_head = zone_head_store.lock().await.get_or_genesis(zone_id);
        engine = Some(ZoneConsensus::new(
            zone_id,
            0,
            committee,
            my_validator_id,
            prev_head,
            config.clone(),
        ));
    }

    let round_interval = std::time::Duration::from_millis(config.round_interval_ms);
    let mut round_timer = tokio::time::interval(round_interval);
    round_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let epoch_start = tokio::time::Instant::now();

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                debug!(zone_id, "zone consensus task shutting down");
                break;
            }

            msg = consensus_rx.recv() => {
                let Some(first) = msg else { break };

                let mut batch = Vec::with_capacity(129);
                batch.push(first);
                while batch.len() < 128 {
                    match consensus_rx.try_recv() {
                        Ok(m) => batch.push(m),
                        Err(_) => break,
                    }
                }

                for cmsg in batch {
                    match cmsg {
                        ZephyrConsensusMessage::Proposal(proposal) => {
                            info!(
                                zone_id,
                                proposer = %hex::encode(&proposal.header.proposer_id[..8]),
                                block_hash = %hex::encode(&proposal.block_hash[..8]),
                                tx_count = proposal.transactions.len(),
                                height = proposal.header.height,
                                parent_hash = %hex::encode(&proposal.header.parent_hash[..8]),
                                "received proposal from network"
                            );
                            cache_block_txs(
                                &mut block_tx_cache,
                                &mut block_nullifiers,
                                zone_id,
                                &proposal,
                            );
                            if let Some(ref mut eng) = engine {
                                let vid = my_validator_id;
                                if let Some(action) =
                                    eng.vote_on_proposal(&proposal, |data| hmac_sign(&vid, data))
                                {
                                    eng.reset_timeout();
                                    if eng.take_fork_recovery_used() {
                                        let dropped = pending_certs.len();
                                        pending_certs.clear();
                                        if dropped > 0 {
                                            info!(zone_id, dropped, "cleared pending certs after fork recovery (proposal)");
                                        }
                                    }
                                    publish_action(
                                        &action,
                                        &consensus_topic,
                                        &global_topic,
                                        &publish_tx,
                                        &block_tx_cache,
                                    );
                                    if let ConsensusAction::BroadcastVote(vote) = action {
                                        if let Some(cert_action) = eng.receive_vote(vote) {
                                            if let ConsensusAction::BroadcastCertificate(
                                                ref cert,
                                            ) = cert_action
                                            {
                                                {
                                                    let mut zhs =
                                                        zone_head_store.lock().await;
                                                    apply_certificate_locally(
                                                        cert,
                                                        &mut zhs,
                                                        &mut block_tx_cache,
                                                        &runtime,
                                                    );
                                                }
                                                cleanup_mempool_after_cert(
                                                    cert,
                                                    &mempool,
                                                    &mut block_nullifiers,
                                                    &mut deferred_cleanups,
                                                );
                                            }
                                            publish_action(
                                                &cert_action,
                                                &consensus_topic,
                                                &global_topic,
                                                &publish_tx,
                                                &block_tx_cache,
                                            );
                                        }
                                    }
                                } else if proposal.header.parent_hash != *eng.parent_hash()
                                    && proposal.header.epoch == eng.epoch()
                                    && proposal.header.height >= eng.height()
                                {
                                    debug!(
                                        zone_id,
                                        proposal_parent = %hex::encode(&proposal.header.parent_hash[..8]),
                                        local_parent = %eng.parent_hash_hex(),
                                        height = proposal.header.height,
                                        "buffering proposal for retry after cert"
                                    );
                                    last_buffered_proposal = Some(proposal);
                                }
                            }
                        }
                        ZephyrConsensusMessage::Vote(vote) => {
                            let mut cert_produced = false;
                            if let Some(ref mut eng) = engine {
                                if let Some(action) = eng.receive_vote(vote) {
                                    if let ConsensusAction::BroadcastCertificate(ref cert) =
                                        action
                                    {
                                        {
                                            let mut zhs = zone_head_store.lock().await;
                                            apply_certificate_locally(
                                                cert,
                                                &mut zhs,
                                                &mut block_tx_cache,
                                                &runtime,
                                            );
                                        }
                                        cleanup_mempool_after_cert(
                                            cert,
                                            &mempool,
                                            &mut block_nullifiers,
                                            &mut deferred_cleanups,
                                        );
                                        cert_produced = true;
                                    }
                                    publish_action(
                                        &action,
                                        &consensus_topic,
                                        &global_topic,
                                        &publish_tx,
                                        &block_tx_cache,
                                    );
                                }
                            }
                            // Proposal deferred to round timer tick to let cert propagate first.
                            let _ = cert_produced;
                        }
                        ZephyrConsensusMessage::Reject(_) => {}
                    }
                }

                let resolved: Vec<[u8; 32]> = deferred_cleanups
                    .keys()
                    .filter(|h| block_nullifiers.contains_key(*h))
                    .copied()
                    .collect();
                for hash in resolved {
                    if deferred_cleanups.remove(&hash).is_some() {
                        if let Some((_, nullifiers)) = block_nullifiers.remove(&hash) {
                            mempool.remove_nullifiers(zone_id, &nullifiers);
                        }
                    }
                }
            }

            msg = global_rx.recv() => {
                let Some(first) = msg else { break };

                let mut global_batch = Vec::with_capacity(33);
                global_batch.push(first);
                while global_batch.len() < 32 {
                    match global_rx.try_recv() {
                        Ok(m) => global_batch.push(m),
                        Err(_) => break,
                    }
                }

                for gmsg in global_batch {
                    match gmsg {
                        ZephyrGlobalMessage::Certificate { cert, tx_nullifiers } => {
                            if !tx_nullifiers.is_empty() {
                                block_tx_cache
                                    .entry(cert.block_hash)
                                    .or_insert_with(|| (cert.zone_id, tx_nullifiers));
                            }

                            let mut round_advanced = false;
                            if let Some(ref mut eng) = engine {
                                if eng.apply_certificate(&cert) {
                                    let fork_recovery_fired = eng.take_fork_recovery_used();
                                    {
                                        let mut zhs = zone_head_store.lock().await;
                                        apply_certificate_locally(
                                            &cert,
                                            &mut zhs,
                                            &mut block_tx_cache,
                                            &runtime,
                                        );
                                    }
                                    cleanup_mempool_after_cert(
                                        &cert,
                                        &mempool,
                                        &mut block_nullifiers,
                                        &mut deferred_cleanups,
                                    );
                                    debug!(zone_id, "applied certificate from global topic");
                                    round_advanced = true;

                                    if fork_recovery_fired {
                                        let dropped = pending_certs.len();
                                        pending_certs.clear();
                                        if dropped > 0 {
                                            info!(zone_id, dropped, "cleared pending certs after fork recovery (cert)");
                                        }
                                    }

                                    // Drain buffered certs that may now be applicable,
                                    // and discard any that are now stale.
                                    while !pending_certs.is_empty() {
                                        let before = pending_certs.len();
                                        let mut still_pending = Vec::new();
                                        for pc in pending_certs.drain(..) {
                                            if pc.block_hash == *eng.parent_hash() {
                                                debug!(
                                                    zone_id,
                                                    cert_block = %hex::encode(&pc.block_hash[..8]),
                                                    "purging stale buffered certificate"
                                                );
                                                continue;
                                            }
                                            if pc.height <= eng.height() {
                                                debug!(
                                                    zone_id,
                                                    cert_height = pc.height,
                                                    local_height = eng.height(),
                                                    cert_block = %hex::encode(&pc.block_hash[..8]),
                                                    "purging height-stale buffered certificate"
                                                );
                                                continue;
                                            }
                                            if eng.apply_certificate(&pc) {
                                                if eng.take_fork_recovery_used() {
                                                    let dropped = still_pending.len();
                                                    still_pending.clear();
                                                    if dropped > 0 {
                                                        info!(zone_id, dropped, "cleared pending certs after fork recovery mid-drain");
                                                    }
                                                    break;
                                                }
                                                {
                                                    let mut zhs =
                                                        zone_head_store.lock().await;
                                                    apply_certificate_locally(
                                                        &pc,
                                                        &mut zhs,
                                                        &mut block_tx_cache,
                                                        &runtime,
                                                    );
                                                }
                                                cleanup_mempool_after_cert(
                                                    &pc,
                                                    &mempool,
                                                    &mut block_nullifiers,
                                                    &mut deferred_cleanups,
                                                );
                                                debug!(
                                                    zone_id,
                                                    "applied buffered certificate"
                                                );
                                            } else {
                                                still_pending.push(pc);
                                            }
                                        }
                                        pending_certs = still_pending;
                                        if pending_certs.len() >= before {
                                            break;
                                        }
                                    }

                                    let purge_at = config.max_pending_certs * 3 / 4;
                                    if pending_certs.len() > purge_at {
                                        let drop_count =
                                            pending_certs.len() - purge_at;
                                        debug!(
                                            zone_id,
                                            dropped = drop_count,
                                            remaining = purge_at,
                                            "purging oldest buffered certs"
                                        );
                                        pending_certs.drain(..drop_count);
                                    }
                                } else if cert.block_hash == *eng.parent_hash() {
                                    debug!(
                                        zone_id,
                                        cert_block = %hex::encode(&cert.block_hash[..8]),
                                        "ignoring stale certificate (already applied)"
                                    );
                                } else if pending_certs.iter().any(|pc| pc.block_hash == cert.block_hash) {
                                    debug!(
                                        zone_id,
                                        cert_block = %hex::encode(&cert.block_hash[..8]),
                                        "ignoring duplicate buffered certificate"
                                    );
                                } else if pending_certs.len() < config.max_pending_certs {
                                    debug!(
                                        zone_id,
                                        cert_block = %hex::encode(&cert.block_hash[..8]),
                                        cert_parent = %hex::encode(&cert.parent_hash[..8]),
                                        local_parent = %eng.parent_hash_hex(),
                                        buffered = pending_certs.len() + 1,
                                        "buffering out-of-order certificate"
                                    );
                                    pending_certs.push(cert.clone());
                                } else {
                                    warn!(
                                        zone_id,
                                        cert_block = %hex::encode(&cert.block_hash[..8]),
                                        cert_parent = %hex::encode(&cert.parent_hash[..8]),
                                        local_parent = %eng.parent_hash_hex(),
                                        buffered = pending_certs.len(),
                                        "pending cert buffer full, dropping certificate"
                                    );
                                }
                            } else {
                                {
                                    let mut zhs = zone_head_store.lock().await;
                                    apply_certificate_locally(
                                        &cert,
                                        &mut zhs,
                                        &mut block_tx_cache,
                                        &runtime,
                                    );
                                }
                                cleanup_mempool_after_cert(
                                    &cert,
                                    &mempool,
                                    &mut block_nullifiers,
                                    &mut deferred_cleanups,
                                );
                            }

                            // Proposal deferred to round timer tick to let cert propagate first.
                            let _ = round_advanced;

                            if round_advanced {
                                if let Some(ref mut eng) = engine {
                                    retry_buffered_proposal(
                                        &mut last_buffered_proposal,
                                        eng,
                                        zone_id,
                                        my_validator_id,
                                        &mut pending_certs,
                                        &consensus_topic,
                                        &global_topic,
                                        &publish_tx,
                                        &mut block_tx_cache,
                                        &mut block_nullifiers,
                                        &mut deferred_cleanups,
                                        &zone_head_store,
                                        &mempool,
                                        &runtime,
                                    )
                                    .await;
                                }
                            }
                        }
                        ZephyrGlobalMessage::EpochAnnounce(ann) => {
                            debug!(zone_id, epoch = ann.epoch, "received epoch announcement");
                        }
                    }
                }
            }

            _ = round_timer.tick() => {
                let elapsed = epoch_start.elapsed();
                let epoch_elapsed = elapsed.as_millis() as u64 % config.epoch_duration_ms;
                let progress = epoch_elapsed as f32 / config.epoch_duration_ms as f32;
                let expected_epoch = elapsed.as_millis() as u64 / config.epoch_duration_ms;

                // --- Epoch boundary check ---
                // Only acquire the epoch lock when a transition may have
                // occurred (~once per 120s). On every other tick the lock is
                // skipped entirely, saving ~1-2ms of contention per tick.
                if expected_epoch > last_known_epoch {
                    let mut em = epoch_mgr.lock().await;
                    while em.current_epoch() < expected_epoch {
                        em.advance_epoch(&my_validator_id);
                        info!(
                            zone_id,
                            new_epoch = em.current_epoch(),
                            "epoch advanced"
                        );
                    }

                    let current_epoch = em.current_epoch();
                    if current_epoch > last_known_epoch {
                        last_known_epoch = current_epoch;
                        let assigned = em.zones_for_validator(&my_validator_id);
                        let is_assigned = assigned.contains(&zone_id);
                        let was_assigned = engine.is_some();

                        {
                            let mut rt = runtime.write();
                            rt.assigned_zones = assigned;
                        }

                        if is_assigned {
                            let committee = sample_committee(
                                em.randomness_seed(),
                                zone_id,
                                &validators,
                                config.committee_size,
                            );
                            drop(em);
                            if let Some(ref mut eng) = engine {
                                if eng.consecutive_timeouts() >= 2 {
                                    warn!(
                                        zone_id,
                                        consecutive_timeouts = eng.consecutive_timeouts(),
                                        height = eng.height(),
                                        parent_hash = %eng.parent_hash_hex(),
                                        "zone stalled at epoch boundary, enabling fork recovery"
                                    );
                                    if eng.enable_fork_recovery() {
                                        let dropped = pending_certs.len();
                                        pending_certs.clear();
                                        if dropped > 0 {
                                            info!(zone_id, dropped, "cleared pending certs for fork recovery");
                                        }
                                    }
                                }
                                eng.advance_to_epoch(current_epoch, committee);
                                pending_certs.retain(|c| c.epoch + 1 >= current_epoch);
                            } else {
                                let prev_head =
                                    zone_head_store.lock().await.get_or_genesis(zone_id);
                                engine = Some(ZoneConsensus::new(
                                    zone_id,
                                    current_epoch,
                                    committee,
                                    my_validator_id,
                                    prev_head,
                                    config.clone(),
                                ));
                                mempool.add_zone(zone_id, 65_536);
                                pending_certs.retain(|c| c.epoch + 1 >= current_epoch);
                            }
                        } else {
                            drop(em);
                            if was_assigned {
                                engine = None;
                                mempool.remove_zone(zone_id);
                                pending_certs.clear();
                            }
                        }
                    }
                }

                {
                    let mut rt = runtime.write();
                    rt.epoch_progress_pct = progress;
                    rt.current_epoch = last_known_epoch;
                }

                if let Some(ref mut eng) = engine {
                    eng.tick();
                    if eng.is_round_timed_out(config.round_timeout_ticks) {
                        let effective_timeout = config.round_timeout_ticks
                            * (1 + eng.consecutive_timeouts().min(3));
                        let (vote_blocks, max_votes) = eng.vote_summary();
                        warn!(
                            zone_id,
                            round = eng.round(),
                            height = eng.height(),
                            consecutive_timeouts = eng.consecutive_timeouts(),
                            effective_timeout_ticks = effective_timeout,
                            is_leader = eng.is_leader(),
                            leader = %hex::encode(&eng.leader_id()[..8]),
                            parent_hash = %eng.parent_hash_hex(),
                            proposal_seen = eng.proposal_seen(),
                            has_pending_proposal = eng.has_pending_proposal(),
                            votes_for_pending = eng.vote_count_for_pending(),
                            vote_blocks,
                            max_votes,
                            committee_size = eng.committee_size(),
                            quorum = config.quorum_threshold,
                            pending_certs = pending_certs.len(),
                            "round timed out without quorum, rotating leader"
                        );
                        let abandoned_txs = eng.timeout_round();
                        if !abandoned_txs.is_empty() {
                            mempool.reinsert_batch(zone_id, abandoned_txs);
                        }
                        if eng.consecutive_timeouts() >= 2 || pending_certs.len() >= 8 {
                            if eng.enable_fork_recovery() {
                                warn!(
                                    zone_id,
                                    consecutive_timeouts = eng.consecutive_timeouts(),
                                    pending_certs = pending_certs.len(),
                                    height = eng.height(),
                                    parent_hash = %eng.parent_hash_hex(),
                                    mempool = mempool.len(zone_id),
                                    "zone stalled, enabling fork recovery"
                                );
                                let dropped = pending_certs.len();
                                pending_certs.clear();
                                if dropped > 0 {
                                    info!(zone_id, dropped, "cleared pending certs for fork recovery");
                                }
                            }
                        }
                        {
                            let mut rt = runtime.write();
                            rt.zone_consecutive_timeouts.insert(zone_id, eng.consecutive_timeouts());
                        }
                    }

                    // Periodic drain of buffered certs (~every 1s at default 100ms tick).
                    // Breaks the deadlock where a node can't drain because no cert is
                    // applied, and no cert is applied because the drain never runs.
                    if eng.ticks_in_round() % 10 == 0 && !pending_certs.is_empty() {
                        let mut drained_any = true;
                        while drained_any && !pending_certs.is_empty() {
                            drained_any = false;
                            let mut still_pending = Vec::new();
                            for pc in pending_certs.drain(..) {
                                if pc.block_hash == *eng.parent_hash() {
                                    continue;
                                }
                                if pc.height <= eng.height() {
                                    debug!(
                                        zone_id,
                                        cert_height = pc.height,
                                        local_height = eng.height(),
                                        cert_block = %hex::encode(&pc.block_hash[..8]),
                                        "periodic drain: purging height-stale certificate"
                                    );
                                    continue;
                                }
                                if eng.apply_certificate(&pc) {
                                    if eng.take_fork_recovery_used() {
                                        let dropped = still_pending.len();
                                        still_pending.clear();
                                        if dropped > 0 {
                                            info!(zone_id, dropped, "cleared pending certs after fork recovery in periodic drain");
                                        }
                                        break;
                                    }
                                    {
                                        let mut zhs = zone_head_store.lock().await;
                                        apply_certificate_locally(
                                            &pc,
                                            &mut zhs,
                                            &mut block_tx_cache,
                                            &runtime,
                                        );
                                    }
                                    cleanup_mempool_after_cert(
                                        &pc,
                                        &mempool,
                                        &mut block_nullifiers,
                                        &mut deferred_cleanups,
                                    );
                                    debug!(zone_id, "periodic drain: applied buffered certificate");
                                    drained_any = true;
                                } else {
                                    still_pending.push(pc);
                                }
                            }
                            pending_certs = still_pending;
                        }

                        retry_buffered_proposal(
                            &mut last_buffered_proposal,
                            eng,
                            zone_id,
                            my_validator_id,
                            &mut pending_certs,
                            &consensus_topic,
                            &global_topic,
                            &publish_tx,
                            &mut block_tx_cache,
                            &mut block_nullifiers,
                            &mut deferred_cleanups,
                            &zone_head_store,
                            &mempool,
                            &runtime,
                        )
                        .await;
                    }

                    // Periodic health summary during stalls (every ~10s at default 100ms tick)
                    if eng.consecutive_timeouts() > 0 && eng.ticks_in_round() % 100 == 0 {
                        let (vote_blocks, max_votes) = eng.vote_summary();
                        let distinct_parents: std::collections::HashSet<[u8; 32]> = pending_certs.iter().map(|c| c.parent_hash).collect();
                        info!(
                            zone_id,
                            round = eng.round(),
                            height = eng.height(),
                            epoch = eng.epoch(),
                            consecutive_timeouts = eng.consecutive_timeouts(),
                            consecutive_successes = eng.consecutive_successes(),
                            ticks_in_round = eng.ticks_in_round(),
                            is_leader = eng.is_leader(),
                            leader = %hex::encode(&eng.leader_id()[..8]),
                            proposal_seen = eng.proposal_seen(),
                            has_pending_proposal = eng.has_pending_proposal(),
                            votes_for_pending = eng.vote_count_for_pending(),
                            vote_blocks,
                            max_votes,
                            parent_hash = %eng.parent_hash_hex(),
                            pending_certs = pending_certs.len(),
                            pending_cert_distinct_parents = distinct_parents.len(),
                            mempool_len = mempool.len(zone_id),
                            "zone stall health check"
                        );
                    }

                    try_propose_for_zone(
                        zone_id,
                        eng,
                        &mempool,
                        my_validator_id,
                        &config,
                        &mut block_tx_cache,
                        &mut block_nullifiers,
                        &consensus_topic,
                        &global_topic,
                        &publish_tx,
                        &zone_head_store,
                        &runtime,
                        &mut deferred_cleanups,
                    )
                    .await;
                }

                let zone_len = mempool.len(zone_id);
                {
                    let mut rt = runtime.write();
                    rt.mempool_sizes.insert(zone_id, zone_len);
                }
            }
        }
    }
}

fn cache_block_txs(
    cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    nullifier_cache: &mut HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    zone_id: u32,
    block: &grid_programs_zephyr::Block,
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

fn cleanup_mempool_after_cert(
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

fn apply_certificate_locally(
    cert: &FinalityCertificate,
    zone_head_store: &mut ZoneHead,
    block_tx_cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    runtime: &Arc<parking_lot::RwLock<ZephyrRuntime>>,
) {
    zone_head_store.set(cert.zone_id, cert.block_hash);
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

    rt.recent_blocks.push_back(BlockSummary {
        zone_id: cert.zone_id,
        block_hash_hex: hex::encode(&cert.block_hash[..8]),
        height: block_height,
        tx_nullifiers,
    });
    rt.blocks_produced += 1;
    if rt.recent_blocks.len() > MAX_RECENT_BLOCKS {
        rt.recent_blocks.pop_front();
    }
    rt.zone_consecutive_timeouts.insert(cert.zone_id, 0);
    rt.zone_last_advance.insert(cert.zone_id, std::time::Instant::now());
}

#[allow(clippy::too_many_arguments)]
async fn retry_buffered_proposal(
    last_buffered_proposal: &mut Option<Block>,
    eng: &mut ZoneConsensus,
    zone_id: u32,
    my_validator_id: [u8; 32],
    pending_certs: &mut Vec<FinalityCertificate>,
    consensus_topic: &str,
    global_topic: &str,
    publish_tx: &mpsc::Sender<(String, Vec<u8>)>,
    block_tx_cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    block_nullifiers: &mut HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    deferred_cleanups: &mut HashMap<[u8; 32], u32>,
    zone_head_store: &Arc<tokio::sync::Mutex<ZoneHead>>,
    mempool: &SharedMempool,
    runtime: &Arc<parking_lot::RwLock<ZephyrRuntime>>,
) {
    let proposal = match last_buffered_proposal.take() {
        Some(p) => p,
        None => return,
    };

    if proposal.header.height < eng.height() {
        debug!(
            zone_id,
            proposal_height = proposal.header.height,
            local_height = eng.height(),
            "discarding stale buffered proposal"
        );
        return;
    }

    if proposal.header.parent_hash != *eng.parent_hash() {
        *last_buffered_proposal = Some(proposal);
        return;
    }

    debug!(
        zone_id,
        block_hash = %hex::encode(&proposal.block_hash[..8]),
        height = proposal.header.height,
        "retrying buffered proposal after cert catch-up"
    );

    let vid = my_validator_id;
    if let Some(action) = eng.vote_on_proposal(&proposal, |data| hmac_sign(&vid, data)) {
        eng.reset_timeout();
        if eng.take_fork_recovery_used() {
            let dropped = pending_certs.len();
            pending_certs.clear();
            if dropped > 0 {
                info!(zone_id, dropped, "cleared pending certs after fork recovery (retry)");
            }
        }
        publish_action(&action, consensus_topic, global_topic, publish_tx, block_tx_cache);
        if let ConsensusAction::BroadcastVote(vote) = action {
            if let Some(cert_action) = eng.receive_vote(vote) {
                if let ConsensusAction::BroadcastCertificate(ref cert) = cert_action {
                    {
                        let mut zhs = zone_head_store.lock().await;
                        apply_certificate_locally(cert, &mut zhs, block_tx_cache, runtime);
                    }
                    cleanup_mempool_after_cert(cert, mempool, block_nullifiers, deferred_cleanups);
                }
                publish_action(&cert_action, consensus_topic, global_topic, publish_tx, block_tx_cache);
            }
        }
    }
}

fn publish_action(
    action: &ConsensusAction,
    consensus_topic: &str,
    global_topic: &str,
    publish_tx: &mpsc::Sender<(String, Vec<u8>)>,
    block_tx_cache: &HashMap<[u8; 32], (u32, Vec<String>)>,
) {
    let (topic, data) = match action {
        ConsensusAction::BroadcastProposal(p) => {
            let msg = ZephyrConsensusMessage::Proposal(p.clone());
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode proposal");
                    return;
                }
            };
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
            let msg = ZephyrGlobalMessage::Certificate {
                cert: c.clone(),
                tx_nullifiers,
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

    if let Err(e) = publish_tx.try_send((topic, data)) {
        warn!(error = %e, "publish channel full or closed, consensus message delayed");
    }
}

/// Attempt to propose a block for a single zone where this node is leader.
///
/// Called on every round timer tick. Proposals are paced to the timer to give
/// certificates time to propagate before the next proposal goes out.
#[allow(clippy::too_many_arguments)]
async fn try_propose_for_zone(
    zone_id: u32,
    engine: &mut ZoneConsensus,
    mempool: &SharedMempool,
    my_validator_id: [u8; 32],
    config: &ZephyrConfig,
    block_tx_cache: &mut HashMap<[u8; 32], (u32, Vec<String>)>,
    block_nullifiers: &mut HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    consensus_topic: &str,
    global_topic: &str,
    publish_tx: &mpsc::Sender<(String, Vec<u8>)>,
    zone_head_store: &Arc<tokio::sync::Mutex<ZoneHead>>,
    runtime: &Arc<parking_lot::RwLock<ZephyrRuntime>>,
    deferred_cleanups: &mut HashMap<[u8; 32], u32>,
) {
    if !engine.is_leader() {
        return;
    }

    let is_rebroadcast = engine.has_pending_proposal();
    let spends = if is_rebroadcast {
        vec![]
    } else {
        mempool.drain_proposal(zone_id, config.max_block_size)
    };
    let tx_count = spends.len();
    let vid = my_validator_id;
    if let Some(action) = engine.propose(spends, |data| hmac_sign(&vid, data)) {
        if let ConsensusAction::BroadcastProposal(ref block) = action {
            if is_rebroadcast {
                debug!(
                    zone_id,
                    round = engine.round(),
                    block_hash = %hex::encode(&block.block_hash[..8]),
                    "rebroadcasting proposal"
                );
            } else {
                info!(
                    zone_id,
                    height = engine.height(),
                    round = engine.round(),
                    tx_count,
                    block_hash = %hex::encode(&block.block_hash[..8]),
                    "proposed new block"
                );
                cache_block_txs(block_tx_cache, block_nullifiers, zone_id, block);
            }

            publish_action(&action, consensus_topic, global_topic, publish_tx, block_tx_cache);

            if !is_rebroadcast {
                let vid2 = my_validator_id;
                if let Some(vote_action) =
                    engine.vote_on_proposal(block, |data| hmac_sign(&vid2, data))
                {
                    publish_action(
                        &vote_action,
                        consensus_topic,
                        global_topic,
                        publish_tx,
                        block_tx_cache,
                    );
                    if let ConsensusAction::BroadcastVote(vote) = vote_action {
                        if let Some(cert_action) = engine.receive_vote(vote) {
                            if let ConsensusAction::BroadcastCertificate(ref cert) = cert_action {
                                {
                                    let mut zhs = zone_head_store.lock().await;
                                    apply_certificate_locally(
                                        cert,
                                        &mut zhs,
                                        block_tx_cache,
                                        runtime,
                                    );
                                }
                                cleanup_mempool_after_cert(
                                    cert,
                                    mempool,
                                    block_nullifiers,
                                    deferred_cleanups,
                                );
                            }
                            publish_action(
                                &cert_action,
                                consensus_topic,
                                global_topic,
                                publish_tx,
                                block_tx_cache,
                            );
                        }
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

    #[test]
    fn hmac_sign_is_deterministic() {
        let vid = [0xAB; 32];
        let data = b"test-data";
        let s1 = hmac_sign(&vid, data);
        let s2 = hmac_sign(&vid, data);
        assert_eq!(s1, s2);
    }
}
