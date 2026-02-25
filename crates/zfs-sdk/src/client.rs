use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use zfs_net::{
    format_zode_id, Multiaddr, NetworkConfig, NetworkEvent, NetworkService, OutboundRequestId,
    ZodeId,
};

use crate::error::SdkError;

/// Configuration for the SDK client.
#[derive(Debug, Clone, Default)]
pub struct SdkConfig {
    /// Network configuration (listen address, bootstrap peers).
    pub network: NetworkConfig,
}

/// Pending outbound request tracker.
pub(crate) enum PendingRequest {
    Sector(tokio::sync::oneshot::Sender<zfs_core::SectorResponse>),
}

/// SDK client wrapping network connectivity to Zodes.
///
/// Create via [`Client::connect`]. The client maintains a libp2p connection
/// and tracks connected peers for sector operations.
pub struct Client {
    pub(crate) network: Arc<Mutex<NetworkService>>,
    pub(crate) peers: Arc<Mutex<Vec<ZodeId>>>,
    pub(crate) pending: Arc<Mutex<HashMap<OutboundRequestId, PendingRequest>>>,
    shutdown_tx: tokio::sync::mpsc::Sender<()>,
}

impl Client {
    /// Connect to the ZFS network using the given configuration.
    pub async fn connect(config: &SdkConfig) -> Result<Self, SdkError> {
        let network = NetworkService::new(config.network.clone())
            .await
            .map_err(SdkError::Network)?;

        let network = Arc::new(Mutex::new(network));
        let peers = Arc::new(Mutex::new(Vec::new()));
        let pending: Arc<Mutex<HashMap<OutboundRequestId, PendingRequest>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);

        let event_net = Arc::clone(&network);
        let event_peers = Arc::clone(&peers);
        let event_pending = Arc::clone(&pending);
        tokio::spawn(async move {
            Self::event_loop(event_net, event_peers, event_pending, shutdown_rx).await;
        });

        Ok(Self {
            network,
            peers,
            pending,
            shutdown_tx,
        })
    }

    /// Return the list of currently connected Zode IDs.
    pub async fn connected_peers(&self) -> Vec<ZodeId> {
        self.peers.lock().await.clone()
    }

    /// The local Zode ID.
    pub async fn local_zode_id(&self) -> ZodeId {
        *self.network.lock().await.local_zode_id()
    }

    /// The local Zode ID as a `Zx`-prefixed display string.
    pub async fn local_zode_id_display(&self) -> String {
        format_zode_id(&self.local_zode_id().await)
    }

    /// Dial a specific address (e.g. a known Zode).
    pub async fn dial(&self, addr: Multiaddr) -> Result<(), SdkError> {
        self.network
            .lock()
            .await
            .dial(addr)
            .map_err(SdkError::Network)
    }

    /// Shut down the client.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }

    async fn event_loop(
        network: Arc<Mutex<NetworkService>>,
        peers: Arc<Mutex<Vec<ZodeId>>>,
        pending: Arc<Mutex<HashMap<OutboundRequestId, PendingRequest>>>,
        mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    ) {
        loop {
            let event = {
                let mut net = network.lock().await;
                tokio::select! {
                    event = net.next_event() => event,
                    _ = shutdown_rx.recv() => return,
                }
            };

            let Some(event) = event else { return };

            match event {
                NetworkEvent::PeerConnected(peer) => {
                    let mut p = peers.lock().await;
                    if !p.contains(&peer) {
                        p.push(peer);
                    }
                }
                NetworkEvent::PeerDisconnected(peer) => {
                    peers.lock().await.retain(|p| p != &peer);
                }
                NetworkEvent::SectorRequestResult {
                    request_id,
                    response,
                    ..
                } => {
                    let mut pend = pending.lock().await;
                    if let Some(PendingRequest::Sector(tx)) = pend.remove(&request_id) {
                        let _ = tx.send(*response);
                    }
                }
                NetworkEvent::SectorOutboundFailure {
                    request_id, error, ..
                } => {
                    let mut pend = pending.lock().await;
                    if let Some(PendingRequest::Sector(tx)) = pend.remove(&request_id) {
                        let _ = tx.send(zfs_core::SectorResponse::Store(
                            zfs_core::SectorStoreResponse {
                                ok: false,
                                error_code: Some(zfs_core::ErrorCode::InvalidPayload),
                            },
                        ));
                        tracing::warn!(%error, "outbound sector request failed");
                    }
                }
                _ => {}
            }
        }
    }
}
