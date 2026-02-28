use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use grid_core::ProofSystem;
use grid_net::{format_zode_id, NetworkEvent, NetworkService, ZodeId};
use grid_programs_interlink::InterlinkDescriptor;
use grid_proof::{NoopVerifier, ProofVerifierRegistry};
use grid_proof_groth16::Groth16ShapeVerifier;
use grid_storage::{RocksStorage, SectorStore};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{debug, error, info, warn};

use grid_rpc::{RpcConfig, RpcServer};

use crate::config::ZodeConfig;
use crate::error::ZodeError;
use crate::metrics::ZodeMetrics;
use crate::sector_handler::SectorRequestHandler;
pub use crate::types::{LogEvent, ZodeStatus};

/// The Zode — ties together storage, network, proof, and programs.
///
/// Created via [`Zode::start`]. The event loop runs in a background tokio
/// task; the caller interacts via [`status`](Zode::status),
/// [`subscribe_events`](Zode::subscribe_events), and
/// [`shutdown`](Zode::shutdown).
pub struct Zode {
    metrics: Arc<ZodeMetrics>,
    storage: Arc<RocksStorage>,
    network: Arc<Mutex<NetworkService>>,
    zode_id: ZodeId,
    keypair_protobuf: Vec<u8>,
    data_dir: std::path::PathBuf,
    topics: Vec<String>,
    connected_peers: Arc<RwLock<Vec<String>>>,
    event_tx: broadcast::Sender<LogEvent>,
    shutdown_tx: mpsc::Sender<()>,
    publish_tx: mpsc::Sender<(String, Vec<u8>)>,
    event_loop_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    rpc_server: Mutex<Option<RpcServer>>,
    rpc_config: RpcConfig,
}

impl Zode {
    /// Start the Zode with the given configuration.
    ///
    /// Starts the network first to obtain the peer ID, then derives a
    /// unique storage path from the last 6 characters of the Zode ID,
    /// ensures proof keys exist, opens storage, and begins the event loop.
    pub async fn start(mut config: ZodeConfig) -> Result<Self, ZodeError> {
        let has_keypair = config.network.keypair.is_some();
        let (network, zode_id, keypair_protobuf, topic_strings, effective) =
            Self::start_network(&config).await?;

        if !has_keypair {
            let zode_id_str = format_zode_id(&zode_id);
            let suffix = &zode_id_str[zode_id_str.len().saturating_sub(6)..];
            let base_name = config
                .storage
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "data".to_string());
            let parent = config
                .storage
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            config.storage.path = parent.join(format!("{base_name}-{suffix}"));
        }
        let data_dir = config.storage.path.clone();

        let vk_dir = data_dir.join("proof_keys");
        grid_proof_groth16::ensure_keys(&vk_dir);

        let storage =
            Arc::new(RocksStorage::open(config.storage.clone()).map_err(ZodeError::Storage)?);
        info!(path = ?config.storage.path, "storage opened");

        let (event_tx, _) = broadcast::channel(256);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let (publish_tx, publish_rx) = mpsc::channel(64);
        let metrics = Arc::new(ZodeMetrics::default());
        let connected_peers: Arc<RwLock<Vec<String>>> = Arc::default();

        let mut proof_registry = ProofVerifierRegistry::new();
        proof_registry.register(ProofSystem::None, Arc::new(NoopVerifier));

        if vk_dir.exists() {
            match Groth16ShapeVerifier::load(&vk_dir) {
                Ok(verifier) => {
                    info!(?vk_dir, "loaded Groth16 verifying keys");
                    proof_registry.register(ProofSystem::Groth16, Arc::new(verifier));
                }
                Err(e) => {
                    warn!(error = %e, "failed to load Groth16 verifying keys");
                }
            }
        }
        let proof_registry = Arc::new(proof_registry);

        let mut program_proof_config: HashMap<grid_core::ProgramId, ProofSystem> = HashMap::new();
        if let Ok(pid) = InterlinkDescriptor::v2().program_id() {
            program_proof_config.insert(pid, ProofSystem::Groth16);
        }

        let sector_handler = Arc::new(SectorRequestHandler::new(
            Arc::clone(&storage),
            effective,
            config.sector_limits.clone(),
            config.sector_filter.clone(),
            Arc::clone(&metrics),
            proof_registry,
            program_proof_config,
        ));

        let rpc_server = if config.rpc.enabled {
            match RpcServer::start(&config.rpc, Arc::clone(&sector_handler) as _).await {
                Ok(rpc) => {
                    let _ = event_tx.send(LogEvent::RpcStarted {
                        bind_addr: rpc.bind_addr().to_string(),
                    });
                    Some(rpc)
                }
                Err(e) => {
                    warn!(error = %e, "failed to start RPC server");
                    None
                }
            }
        } else {
            None
        };

        let network = Arc::new(Mutex::new(network));
        let event_loop_handle = Self::spawn_event_loop(
            sector_handler,
            Arc::clone(&network),
            event_tx.clone(),
            Arc::clone(&metrics),
            Arc::clone(&connected_peers),
            shutdown_rx,
            publish_rx,
        );

        Ok(Self {
            metrics,
            storage,
            network,
            zode_id,
            keypair_protobuf,
            data_dir,
            topics: topic_strings,
            connected_peers,
            event_tx,
            shutdown_tx,
            publish_tx,
            event_loop_handle: Mutex::new(Some(event_loop_handle)),
            rpc_server: Mutex::new(rpc_server),
            rpc_config: config.rpc.clone(),
        })
    }

    async fn start_network(
        config: &ZodeConfig,
    ) -> Result<
        (
            NetworkService,
            ZodeId,
            Vec<u8>,
            Vec<String>,
            std::collections::HashSet<grid_core::ProgramId>,
        ),
        ZodeError,
    > {
        let mut network = NetworkService::new(config.network.clone())
            .await
            .map_err(ZodeError::Network)?;
        let zode_id = *network.local_zode_id();
        let keypair_protobuf = network.keypair_to_protobuf();
        info!(%zode_id, "network started");

        let effective = config.effective_topics();
        let mut topic_strings = Vec::new();
        for pid in &effective {
            let topic = grid_core::program_topic(pid);
            network.subscribe(&topic).map_err(ZodeError::Network)?;
            topic_strings.push(topic);
            debug!(program_id = %pid.to_hex(), "subscribed to topic");
        }
        Ok((network, zode_id, keypair_protobuf, topic_strings, effective))
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_event_loop<S: SectorStore + Send + Sync + 'static>(
        sector_handler: Arc<SectorRequestHandler<S>>,
        network: Arc<Mutex<NetworkService>>,
        event_tx: broadcast::Sender<LogEvent>,
        metrics: Arc<ZodeMetrics>,
        connected_peers: Arc<RwLock<Vec<String>>>,
        shutdown_rx: mpsc::Receiver<()>,
        publish_rx: mpsc::Receiver<(String, Vec<u8>)>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            Self::event_loop(
                sector_handler,
                network,
                event_tx,
                metrics,
                connected_peers,
                shutdown_rx,
                publish_rx,
            )
            .await;
        })
    }

    /// Get a status snapshot of the running Zode (lock-free, never blocks).
    pub fn status(&self) -> ZodeStatus {
        if let Ok(rpc) = self.rpc_server.try_lock() {
            if let Some(ref rpc) = *rpc {
                self.metrics.set_rpc_requests(rpc.requests_total());
            }
        }
        let metrics = self.metrics.snapshot();
        let connected_peers = self
            .connected_peers
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let peer_count = connected_peers.len() as u64;

        let (rpc_enabled, rpc_addr, rpc_auth_required) = if let Ok(rpc) = self.rpc_server.try_lock()
        {
            match *rpc {
                Some(ref rpc) => (true, Some(rpc.bind_addr().to_string()), rpc.auth_required()),
                None => (false, None, false),
            }
        } else {
            (
                self.rpc_config.enabled,
                None,
                self.rpc_config.api_key.is_some(),
            )
        };

        ZodeStatus {
            zode_id: format_zode_id(&self.zode_id),
            peer_count,
            connected_peers,
            topics: self.topics.clone(),
            metrics,
            rpc_enabled,
            rpc_addr,
            rpc_auth_required,
        }
    }

    /// Subscribe to structured log events from the Zode.
    pub fn subscribe_events(&self) -> broadcast::Receiver<LogEvent> {
        self.event_tx.subscribe()
    }

    /// Access the metrics (for direct reads from atomic counters).
    pub fn metrics(&self) -> &Arc<ZodeMetrics> {
        &self.metrics
    }

    /// The libp2p keypair as protobuf-encoded bytes (for vault persistence).
    pub fn keypair_protobuf(&self) -> &[u8] {
        &self.keypair_protobuf
    }

    /// Returns all peer multiaddrs observed during this session, suitable
    /// for persistence and later bootstrap dialing.  Safe to call after
    /// [`shutdown`](Self::shutdown) (uses addresses cached in the service,
    /// not the live swarm connection list).
    pub async fn peer_multiaddrs(&self) -> Vec<String> {
        let net = self.network.lock().await;
        net.peer_multiaddr_strings()
    }

    /// The actual data directory (unique per peer ID).
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Access the underlying storage (for advanced queries).
    pub fn storage(&self) -> &Arc<RocksStorage> {
        &self.storage
    }

    /// Lock-free access to subscribed topic strings (e.g. `"prog/{hex}"`).
    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    /// Queue a GossipSub publish.  The event loop will send it on the
    /// next iteration (non-blocking from the caller's perspective).
    pub fn publish(&self, topic: String, data: Vec<u8>) {
        if let Err(e) = self.publish_tx.try_send((topic, data)) {
            warn!(error = %e, "publish channel full or closed");
        }
    }

    /// Access the network service (for advanced operations).
    pub fn network(&self) -> &Arc<Mutex<NetworkService>> {
        &self.network
    }

    /// Gracefully shut down the Zode and wait for the event loop to exit.
    pub async fn shutdown(&self) {
        let _ = self.event_tx.send(LogEvent::ShuttingDown);
        if let Some(rpc) = self.rpc_server.lock().await.take() {
            rpc.shutdown().await;
        }
        let _ = self.shutdown_tx.send(()).await;
        if let Some(handle) = self.event_loop_handle.lock().await.take() {
            let _ = handle.await;
        }
        info!("zode shutdown complete");
    }

    #[allow(clippy::too_many_arguments)]
    async fn event_loop<S: SectorStore>(
        sector_handler: Arc<SectorRequestHandler<S>>,
        network: Arc<Mutex<NetworkService>>,
        event_tx: broadcast::Sender<LogEvent>,
        metrics: Arc<ZodeMetrics>,
        connected_peers: Arc<RwLock<Vec<String>>>,
        mut shutdown_rx: mpsc::Receiver<()>,
        mut publish_rx: mpsc::Receiver<(String, Vec<u8>)>,
    ) {
        loop {
            let event = {
                let mut net = network.lock().await;
                tokio::select! {
                    event = net.next_event() => event,
                    _ = shutdown_rx.recv() => {
                        info!("event loop shutting down");
                        return;
                    }
                    Some((topic, data)) = publish_rx.recv() => {
                        if let Err(e) = net.publish(&topic, data) {
                            warn!(error = %e, "gossip publish failed");
                        }
                        continue;
                    }
                }
            };

            let Some(event) = event else {
                warn!("network event stream ended");
                return;
            };

            Self::dispatch_event(
                event,
                &sector_handler,
                &network,
                &event_tx,
                &metrics,
                &connected_peers,
            )
            .await;
        }
    }

    async fn dispatch_event<S: SectorStore>(
        event: NetworkEvent,
        sector_handler: &SectorRequestHandler<S>,
        network: &Arc<Mutex<NetworkService>>,
        event_tx: &broadcast::Sender<LogEvent>,
        metrics: &Arc<ZodeMetrics>,
        connected_peers: &Arc<RwLock<Vec<String>>>,
    ) {
        match event {
            NetworkEvent::PeerConnected(peer) => {
                Self::handle_peer_connected(peer, metrics, connected_peers, event_tx);
            }
            NetworkEvent::PeerDisconnected(peer) => {
                Self::handle_peer_disconnected(peer, metrics, connected_peers, event_tx);
            }
            NetworkEvent::IncomingSectorRequest {
                peer,
                request,
                channel,
            } => {
                Self::handle_incoming_sector(
                    sector_handler,
                    network,
                    event_tx,
                    peer,
                    request,
                    channel,
                )
                .await;
            }
            NetworkEvent::ListenAddress(addr) => {
                info!(%addr, "listening");
                let _ = event_tx.send(LogEvent::Started {
                    listen_addr: addr.to_string(),
                });
            }
            NetworkEvent::GossipMessage {
                source,
                topic,
                data,
            } => {
                let sender = source.map(|id| format_zode_id(&id));
                crate::gossip::handle_gossip_message(
                    sector_handler,
                    event_tx,
                    &topic,
                    &data,
                    sender,
                );
            }
            NetworkEvent::PeerDiscovered {
                zode_id, addresses, ..
            } => {
                debug!(%zode_id, addr_count = addresses.len(), "zode discovered via DHT");
                let _ = event_tx.send(LogEvent::PeerDiscovered(format_zode_id(&zode_id)));
            }
            NetworkEvent::SectorRequestResult { .. }
            | NetworkEvent::SectorOutboundFailure { .. } => {}
        }
    }

    fn handle_peer_connected(
        peer: ZodeId,
        metrics: &Arc<ZodeMetrics>,
        connected_peers: &Arc<RwLock<Vec<String>>>,
        event_tx: &broadcast::Sender<LogEvent>,
    ) {
        metrics.inc_peer_count();
        let peer_str = format_zode_id(&peer);
        if let Ok(mut peers) = connected_peers.write() {
            if !peers.contains(&peer_str) {
                peers.push(peer_str.clone());
            }
        }
        debug!(%peer, "peer connected");
        let _ = event_tx.send(LogEvent::PeerConnected(peer_str));
    }

    fn handle_peer_disconnected(
        peer: ZodeId,
        metrics: &Arc<ZodeMetrics>,
        connected_peers: &Arc<RwLock<Vec<String>>>,
        event_tx: &broadcast::Sender<LogEvent>,
    ) {
        metrics.dec_peer_count();
        let peer_str = format_zode_id(&peer);
        if let Ok(mut peers) = connected_peers.write() {
            peers.retain(|p| p != &peer_str);
        }
        debug!(%peer, "peer disconnected");
        let _ = event_tx.send(LogEvent::PeerDisconnected(peer_str));
    }

    async fn handle_incoming_sector<S: SectorStore>(
        sector_handler: &SectorRequestHandler<S>,
        network: &Arc<Mutex<NetworkService>>,
        event_tx: &broadcast::Sender<LogEvent>,
        peer: ZodeId,
        request: Box<grid_core::SectorRequest>,
        channel: grid_net::ResponseChannel<grid_core::SectorResponse>,
    ) {
        debug!(%peer, "incoming sector request");
        let response = sector_handler.handle_sector_request(&request);
        emit_sector_log(event_tx, &request, &response);
        let mut net = network.lock().await;
        if let Err(e) = net.send_sector_response(channel, response) {
            error!(error = %e, "failed to send sector response");
        }
    }
}

fn emit_sector_log(
    event_tx: &broadcast::Sender<LogEvent>,
    request: &grid_core::SectorRequest,
    response: &grid_core::SectorResponse,
) {
    match (request, response) {
        (grid_core::SectorRequest::Append(r), grid_core::SectorResponse::Append(s)) => {
            let _ = event_tx.send(LogEvent::SectorAppendProcessed {
                program_id: r.program_id.to_hex(),
                sector_id: r.sector_id.to_hex(),
                index: s.index,
                accepted: s.ok,
            });
        }
        (grid_core::SectorRequest::ReadLog(r), grid_core::SectorResponse::ReadLog(s)) => {
            let _ = event_tx.send(LogEvent::SectorReadLogProcessed {
                program_id: r.program_id.to_hex(),
                sector_id: r.sector_id.to_hex(),
                entries: s.entries.len(),
            });
        }
        _ => {}
    }
}
