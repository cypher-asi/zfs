use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_zephyr::{
    FinalityCertificate, ValidatorInfo, ZephyrGlobalDescriptor, ZephyrGlobalMessage,
    ZephyrSpendDescriptor, ZephyrValidatorDescriptor, ZephyrZoneDescriptor, ZephyrZoneMessage,
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
use crate::mempool::Mempool;
use crate::storage::zone_head::ZoneHead;

/// Live metrics snapshot shared between the consensus task and HTTP handlers.
pub(crate) struct ZephyrRuntime {
    pub zone_heads: HashMap<u32, [u8; 32]>,
    pub current_epoch: u64,
    pub epoch_progress_pct: f32,
    pub certificates_produced: u64,
    pub spends_processed: u64,
    pub mempool_sizes: HashMap<u32, usize>,
    pub assigned_zones: Vec<u32>,
}

/// Shared state handed to HTTP route handlers.
pub(crate) struct ZephyrState {
    pub(crate) config: ZephyrConfig,
    pub(crate) global_program_id: ProgramId,
    pub(crate) zone_program_ids: Vec<ProgramId>,
    pub(crate) runtime: Arc<std::sync::RwLock<ZephyrRuntime>>,
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
    runtime: Arc<std::sync::RwLock<ZephyrRuntime>>,
    gossip_handler: Arc<ZephyrGossipHandler>,
    zone_rx: std::sync::Mutex<Option<mpsc::Receiver<(String, ZephyrZoneMessage)>>>,
    global_rx: std::sync::Mutex<Option<mpsc::Receiver<ZephyrGlobalMessage>>>,
}

impl ZephyrService {
    pub fn new(config: ZephyrConfig) -> Result<Self, ServiceError> {
        let global_pid = ZephyrGlobalDescriptor::new()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        let mut zone_pids = Vec::with_capacity(config.total_zones as usize);
        for zone_id in 0..config.total_zones {
            let pid = ZephyrZoneDescriptor::new(zone_id)
                .program_id()
                .map_err(|e| ServiceError::Descriptor(e.to_string()))?;
            zone_pids.push(pid);
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

        let global_topic = grid_core::program_topic(&global_pid);
        let (zone_tx, zone_rx) = mpsc::channel(4096);
        let (global_tx, global_rx) = mpsc::channel(1024);
        let gossip_handler = Arc::new(ZephyrGossipHandler::new(global_topic, zone_tx, global_tx));

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
            runtime: Arc::new(std::sync::RwLock::new(ZephyrRuntime {
                zone_heads: HashMap::new(),
                current_epoch: 0,
                epoch_progress_pct: 0.0,
                certificates_produced: 0,
                spends_processed: 0,
                mempool_sizes: HashMap::new(),
                assigned_zones: Vec::new(),
            })),
            gossip_handler,
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

        for &zone_id in &assigned_zones {
            let topic = self.zone_topic(zone_id);
            self.gossip_handler.add_zone_topic(topic.clone());
            ctx.subscribe_topic(&topic)?;
            info!(zone_id, %topic, "subscribed to zone topic");
        }

        // Take channel receivers (one-time)
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

        // Build topic-to-zone-id map
        let mut topic_to_zone: HashMap<String, u32> = HashMap::new();
        for &zone_id in &assigned_zones {
            topic_to_zone.insert(self.zone_topic(zone_id), zone_id);
        }

        // Update runtime with initial state
        if let Ok(mut rt) = self.runtime.write() {
            rt.assigned_zones = assigned_zones.clone();
            rt.current_epoch = 0;
        }

        // Clone what we need for the spawned task
        let runtime = Arc::clone(&self.runtime);
        let config = self.config.clone();
        let shutdown = ctx.shutdown.clone();
        let publish_tx = ctx
            .publish_sender()
            .ok_or_else(|| ServiceError::NotInitialized("publish channel not set".into()))?;
        let global_topic_for_task = self.global_topic();

        // Spawn the consensus task
        tokio::spawn(consensus_loop(
            my_validator_id,
            validators,
            assigned_zones,
            topic_to_zone,
            config,
            runtime.clone(),
            zone_rx,
            global_rx,
            publish_tx.clone(),
            global_topic_for_task.clone(),
            shutdown.clone(),
            epoch_mgr,
        ));

        info!(
            zones = self.config.total_zones,
            committee_size = self.config.committee_size,
            "Zephyr service started with consensus wiring"
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
        let rt = self.runtime.read().unwrap_or_else(|e| e.into_inner());
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
        })
    }
}

/// The main consensus event loop, spawned as an async task.
#[allow(clippy::too_many_arguments)]
async fn consensus_loop(
    my_validator_id: [u8; 32],
    validators: Vec<ValidatorInfo>,
    assigned_zones: Vec<u32>,
    topic_to_zone: HashMap<String, u32>,
    config: ZephyrConfig,
    runtime: Arc<std::sync::RwLock<ZephyrRuntime>>,
    mut zone_rx: mpsc::Receiver<(String, ZephyrZoneMessage)>,
    mut global_rx: mpsc::Receiver<ZephyrGlobalMessage>,
    publish_tx: mpsc::Sender<(String, Vec<u8>)>,
    global_topic: String,
    shutdown: tokio_util::sync::CancellationToken,
    mut epoch_mgr: EpochManager,
) {
    let mut mempools: HashMap<u32, Mempool> = HashMap::new();
    let mut consensus_engines: HashMap<u32, ZoneConsensus> = HashMap::new();
    let mut zone_head_store = ZoneHead::new();

    // Build zone topic lookup (zone_id -> topic string)
    let mut zone_to_topic: HashMap<u32, String> = HashMap::new();
    for (topic, &zone_id) in &topic_to_zone {
        zone_to_topic.insert(zone_id, topic.clone());
    }

    for &zone_id in &assigned_zones {
        mempools.insert(zone_id, Mempool::new(zone_id, 4096));

        let committee = sample_committee(
            epoch_mgr.randomness_seed(),
            zone_id,
            &validators,
            config.committee_size,
        );
        let prev_head = zone_head_store.get_or_genesis(zone_id);
        consensus_engines.insert(
            zone_id,
            ZoneConsensus::new(zone_id, 0, committee, my_validator_id, prev_head, config.clone()),
        );
    }

    let round_interval = std::time::Duration::from_millis(config.round_interval_ms);
    let mut round_timer = tokio::time::interval(round_interval);
    round_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let epoch_start = tokio::time::Instant::now();

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("consensus loop shutting down");
                break;
            }

            _ = round_timer.tick() => {
                // Update epoch progress
                let elapsed = epoch_start.elapsed();
                let epoch_elapsed = elapsed.as_millis() as u64 % config.epoch_duration_ms;
                let progress = epoch_elapsed as f32 / config.epoch_duration_ms as f32;

                // Check for epoch boundary
                let expected_epoch = elapsed.as_millis() as u64 / config.epoch_duration_ms;
                let current_epoch = epoch_mgr.current_epoch();

                if expected_epoch > current_epoch {
                    let transition = epoch_mgr.advance_epoch(&my_validator_id);
                    info!(
                        new_epoch = transition.new_epoch,
                        gained = transition.gained_zones.len(),
                        lost = transition.lost_zones.len(),
                        "epoch advanced"
                    );
                }

                if let Ok(mut rt) = runtime.write() {
                    rt.epoch_progress_pct = progress;
                    rt.current_epoch = epoch_mgr.current_epoch();
                }

                // Leader proposes for assigned zones
                for &zone_id in &assigned_zones {
                    let Some(engine) = consensus_engines.get(&zone_id) else {
                        continue;
                    };
                    if !engine.is_leader() {
                        continue;
                    }
                    let Some(mp) = mempools.get_mut(&zone_id) else {
                        continue;
                    };
                    let spends = mp.drain(config.max_batch_size);
                    let vid = my_validator_id;
                    if let Some(action) = engine.propose(spends, |data| hmac_sign(&vid, data)) {
                        publish_action(&action, &zone_to_topic, zone_id, &global_topic, &publish_tx).await;
                    }
                }

                // Update mempool sizes in runtime
                if let Ok(mut rt) = runtime.write() {
                    for (&zone_id, mp) in &mempools {
                        rt.mempool_sizes.insert(zone_id, mp.len());
                    }
                }
            }

            msg = zone_rx.recv() => {
                let Some((topic, msg)) = msg else { break };
                let Some(&zone_id) = topic_to_zone.get(&topic) else {
                    warn!(%topic, "received message for unknown zone topic");
                    continue;
                };

                match msg {
                    ZephyrZoneMessage::SubmitSpend(tx) => {
                        if let Some(mp) = mempools.get_mut(&zone_id) {
                            if mp.insert(tx) {
                                debug!(zone_id, "spend inserted into mempool");
                            }
                        }
                    }
                    ZephyrZoneMessage::Proposal(proposal) => {
                        let Some(engine) = consensus_engines.get(&zone_id) else {
                            continue;
                        };
                        let vid = my_validator_id;
                        if let Some(action) = engine.vote_on_proposal(&proposal, |data| hmac_sign(&vid, data)) {
                            publish_action(&action, &zone_to_topic, zone_id, &global_topic, &publish_tx).await;
                        }
                    }
                    ZephyrZoneMessage::Vote(vote) => {
                        let Some(engine) = consensus_engines.get_mut(&zone_id) else {
                            continue;
                        };
                        if let Some(action) = engine.receive_vote(vote) {
                            if let ConsensusAction::BroadcastCertificate(ref cert) = action {
                                apply_certificate_locally(
                                    cert,
                                    &mut zone_head_store,
                                    &mempools,
                                    &runtime,
                                );
                            }
                            publish_action(&action, &zone_to_topic, zone_id, &global_topic, &publish_tx).await;
                        }
                    }
                    ZephyrZoneMessage::Reject(_) => {
                        debug!(zone_id, "received reject message (ignored)");
                    }
                }
            }

            msg = global_rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    ZephyrGlobalMessage::Certificate(cert) => {
                        let cz = cert.zone_id;
                        if let Some(engine) = consensus_engines.get_mut(&cz) {
                            if engine.apply_certificate(&cert) {
                                apply_certificate_locally(
                                    &cert,
                                    &mut zone_head_store,
                                    &mempools,
                                    &runtime,
                                );
                                debug!(zone_id = cz, "applied certificate from global topic");
                            }
                        } else {
                            // Certificate for a zone we don't handle — still track the head
                            zone_head_store.set(cz, cert.new_zone_head);
                            if let Ok(mut rt) = runtime.write() {
                                rt.zone_heads.insert(cz, cert.new_zone_head);
                                rt.certificates_produced += 1;
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

fn apply_certificate_locally(
    cert: &FinalityCertificate,
    zone_head_store: &mut ZoneHead,
    _mempools: &HashMap<u32, Mempool>,
    runtime: &Arc<std::sync::RwLock<ZephyrRuntime>>,
) {
    zone_head_store.set(cert.zone_id, cert.new_zone_head);
    let spend_count = cert.signatures.len() as u64;
    if let Ok(mut rt) = runtime.write() {
        rt.zone_heads.insert(cert.zone_id, cert.new_zone_head);
        rt.certificates_produced += 1;
        rt.spends_processed += spend_count;
    }
}

async fn publish_action(
    action: &ConsensusAction,
    zone_to_topic: &HashMap<u32, String>,
    zone_id: u32,
    global_topic: &str,
    publish_tx: &mpsc::Sender<(String, Vec<u8>)>,
) {
    let (topic, data) = match action {
        ConsensusAction::BroadcastProposal(p) => {
            let topic = zone_to_topic.get(&zone_id).cloned().unwrap_or_default();
            let msg = ZephyrZoneMessage::Proposal(p.clone());
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode proposal");
                    return;
                }
            };
            (topic, data)
        }
        ConsensusAction::BroadcastVote(v) => {
            let topic = zone_to_topic.get(&zone_id).cloned().unwrap_or_default();
            let msg = ZephyrZoneMessage::Vote(v.clone());
            let data = match grid_core::encode_canonical(&msg) {
                Ok(d) => d,
                Err(e) => {
                    warn!(error = %e, "failed to encode vote");
                    return;
                }
            };
            (topic, data)
        }
        ConsensusAction::BroadcastCertificate(c) => {
            let msg = ZephyrGlobalMessage::Certificate(c.clone());
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

    if publish_tx.try_send((topic, data)).is_err() {
        warn!("publish channel full or closed");
    }
}

// --- HTTP Handlers ---

async fn status_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    let rt = state.runtime.read().unwrap_or_else(|e| e.into_inner());
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
    let rt = state.runtime.read().unwrap_or_else(|e| e.into_inner());
    let head = rt.zone_heads.get(&id).map(|h| hex::encode(h));
    Json(serde_json::json!({
        "zone_id": id,
        "program_id": pid.to_hex(),
        "head": head,
    }))
    .into_response()
}

async fn epoch_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    let rt = state.runtime.read().unwrap_or_else(|e| e.into_inner());
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
