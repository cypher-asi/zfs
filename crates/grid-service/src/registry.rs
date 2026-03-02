use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use axum::response::IntoResponse;
use axum::Router;
use grid_core::ProgramId;
use grid_rpc::SectorDispatch;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::context::{ServiceContext, ServiceEvent, TopicCommand};
use crate::descriptor::ServiceId;
use crate::error::ServiceError;
use crate::gossip::ServiceGossipHandler;
use crate::identity::NodeIdentity;
use crate::service::Service;

/// Manages the lifecycle of all active services on a Zode.
///
/// Tracks per-service running state via `active_services`. The merged
/// axum router is built once (after `start_all`) and stays static;
/// a per-service middleware layer gates requests at runtime, returning
/// 503 for stopped services.
pub struct ServiceRegistry {
    services: HashMap<ServiceId, Arc<dyn Service>>,
    contexts: HashMap<ServiceId, ServiceContext>,
    active_services: Arc<RwLock<HashSet<ServiceId>>>,
    sector_dispatch: Option<Arc<dyn SectorDispatch>>,
    ephemeral_key: [u8; 32],
    event_tx: broadcast::Sender<ServiceEvent>,
    shutdown: CancellationToken,
    gossip_handlers: Vec<Arc<dyn ServiceGossipHandler>>,
    publish_tx: Option<mpsc::Sender<(String, Vec<u8>)>>,
    topic_tx: Option<mpsc::Sender<TopicCommand>>,
    direct_tx: Option<mpsc::Sender<(String, String, Vec<u8>)>>,
    identity: Option<Arc<NodeIdentity>>,
    proof_registry: Option<Arc<grid_proof::ProofVerifierRegistry>>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            services: HashMap::new(),
            contexts: HashMap::new(),
            active_services: Arc::new(RwLock::new(HashSet::new())),
            sector_dispatch: None,
            ephemeral_key: [0u8; 32],
            event_tx,
            shutdown: CancellationToken::new(),
            gossip_handlers: Vec::new(),
            publish_tx: None,
            topic_tx: None,
            direct_tx: None,
            identity: None,
            proof_registry: None,
        }
    }

    /// Set the GossipSub publish and topic management channels.
    ///
    /// Must be called before `start_all` so that each `ServiceContext`
    /// receives a clone of these senders.
    pub fn set_channels(
        &mut self,
        publish_tx: mpsc::Sender<(String, Vec<u8>)>,
        topic_tx: mpsc::Sender<TopicCommand>,
        direct_tx: mpsc::Sender<(String, String, Vec<u8>)>,
    ) {
        self.publish_tx = Some(publish_tx);
        self.topic_tx = Some(topic_tx);
        self.direct_tx = Some(direct_tx);
    }

    /// Set the node identity that will be shared with all service contexts.
    pub fn set_identity(&mut self, identity: Arc<NodeIdentity>) {
        self.identity = Some(identity);
    }

    /// Set the proof verifier registry shared with all service contexts.
    pub fn set_proof_registry(&mut self, registry: Arc<grid_proof::ProofVerifierRegistry>) {
        self.proof_registry = Some(registry);
    }

    /// Register a service. Does NOT start it yet.
    pub fn register(&mut self, service: Arc<dyn Service>) -> Result<(), ServiceError> {
        let id = service
            .descriptor()
            .service_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        if self.services.contains_key(&id) {
            return Err(ServiceError::AlreadyRegistered(
                service.descriptor().name.clone(),
            ));
        }

        info!(
            name = %service.descriptor().name,
            version = %service.descriptor().version,
            service_id = %id,
            "service registered"
        );
        self.services.insert(id, service);
        Ok(())
    }

    /// Start all registered services. Creates a [`ServiceContext`] for each and
    /// calls `on_start()`. Retains the `sector_dispatch` and `ephemeral_key`
    /// so that individual services can be restarted later via [`start_service`].
    pub async fn start_all(
        &mut self,
        sector_dispatch: Arc<dyn SectorDispatch>,
    ) -> Result<(), ServiceError> {
        let ephemeral_key = generate_ephemeral_key();
        self.sector_dispatch = Some(Arc::clone(&sector_dispatch));
        self.ephemeral_key = ephemeral_key;

        for (&id, service) in &self.services {
            let mut ctx = ServiceContext::new(
                id,
                Arc::clone(&sector_dispatch),
                ephemeral_key,
                self.event_tx.clone(),
                self.shutdown.child_token(),
            );
            if let (Some(ptx), Some(ttx)) = (&self.publish_tx, &self.topic_tx) {
                ctx.set_channels(ptx.clone(), ttx.clone());
            }
            if let Some(dtx) = &self.direct_tx {
                ctx.set_direct_channel(dtx.clone());
            }
            if let Some(ident) = &self.identity {
                ctx.set_identity(Arc::clone(ident));
            }
            if let Some(pr) = &self.proof_registry {
                ctx.set_proof_registry(Arc::clone(pr));
            }

            if let Err(e) = service.on_start(&ctx).await {
                error!(
                    name = %service.descriptor().name,
                    error = %e,
                    "service failed to start"
                );
                return Err(e);
            }

            if let Ok(mut active) = self.active_services.write() {
                active.insert(id);
            }

            let _ = self.event_tx.send(ServiceEvent::Started { service_id: id });
            info!(
                name = %service.descriptor().name,
                "service started"
            );
            self.contexts.insert(id, ctx);
        }
        Ok(())
    }

    /// Stop all running services gracefully.
    pub async fn stop_all(&mut self) -> Result<(), ServiceError> {
        self.shutdown.cancel();

        for (&id, service) in &self.services {
            if let Err(e) = service.on_stop().await {
                error!(
                    name = %service.descriptor().name,
                    error = %e,
                    "service failed to stop cleanly"
                );
            }
            let _ = self.event_tx.send(ServiceEvent::Stopped { service_id: id });
            info!(name = %service.descriptor().name, "service stopped");
        }
        self.contexts.clear();
        if let Ok(mut active) = self.active_services.write() {
            active.clear();
        }
        Ok(())
    }

    /// Stop a single service by ID.
    ///
    /// Cancels the service's shutdown token, calls `on_stop()`, removes
    /// it from the active set, and emits [`ServiceEvent::Stopped`].
    /// No-op if the service is already stopped.
    pub async fn stop_service(&mut self, id: &ServiceId) -> Result<(), ServiceError> {
        let service = self
            .services
            .get(id)
            .ok_or_else(|| ServiceError::NotFound(id.to_hex()))?;

        let Some(ctx) = self.contexts.remove(id) else {
            return Ok(());
        };
        ctx.shutdown.cancel();

        if let Err(e) = service.on_stop().await {
            error!(service_id = %id, error = %e, "service failed to stop cleanly");
        }

        if let Ok(mut active) = self.active_services.write() {
            active.remove(id);
        }

        let _ = self
            .event_tx
            .send(ServiceEvent::Stopped { service_id: *id });
        info!(service_id = %id, "service stopped");
        Ok(())
    }

    /// Start a single previously-stopped service.
    ///
    /// Requires [`start_all`](Self::start_all) to have been called first
    /// (to initialize the shared sector dispatch). Creates a fresh
    /// [`ServiceContext`], calls `on_start()`, and adds the service to
    /// the active set. No-op if already running.
    pub async fn start_service(&mut self, id: &ServiceId) -> Result<(), ServiceError> {
        if self.contexts.contains_key(id) {
            return Ok(());
        }

        let sector_dispatch = self.sector_dispatch.as_ref().ok_or_else(|| {
            ServiceError::NotInitialized("start_all must be called before start_service".into())
        })?;

        let service = self
            .services
            .get(id)
            .ok_or_else(|| ServiceError::NotFound(id.to_hex()))?;

        let mut ctx = ServiceContext::new(
            *id,
            Arc::clone(sector_dispatch),
            self.ephemeral_key,
            self.event_tx.clone(),
            self.shutdown.child_token(),
        );
        if let (Some(ptx), Some(ttx)) = (&self.publish_tx, &self.topic_tx) {
            ctx.set_channels(ptx.clone(), ttx.clone());
        }
        if let Some(dtx) = &self.direct_tx {
            ctx.set_direct_channel(dtx.clone());
        }
        if let Some(ident) = &self.identity {
            ctx.set_identity(Arc::clone(ident));
        }
        if let Some(pr) = &self.proof_registry {
            ctx.set_proof_registry(Arc::clone(pr));
        }

        service.on_start(&ctx).await?;

        if let Ok(mut active) = self.active_services.write() {
            active.insert(*id);
        }

        let _ = self
            .event_tx
            .send(ServiceEvent::Started { service_id: *id });
        info!(service_id = %id, "service started");
        self.contexts.insert(*id, ctx);
        Ok(())
    }

    /// Shared handle to the set of currently-active service IDs.
    ///
    /// Used by the per-service router middleware to gate requests at
    /// runtime without rebuilding the router.
    pub fn active_services(&self) -> Arc<RwLock<HashSet<ServiceId>>> {
        Arc::clone(&self.active_services)
    }

    /// Build a merged axum `Router` with all service routes mounted at
    /// `/services/{service_id_hex}/`.
    ///
    /// Each sub-router is wrapped with a middleware that returns
    /// `503 Service Unavailable` when the service is not in the
    /// active set, allowing per-service stop/start without rebuilding
    /// the router.
    pub fn merged_router(&self) -> Router {
        let mut app = Router::new();
        for (&id, service) in &self.services {
            if let Some(ctx) = self.contexts.get(&id) {
                let prefix = format!("/services/{}", id.to_hex());
                let gate_state = (id, Arc::clone(&self.active_services));
                let service_router =
                    service
                        .routes(ctx)
                        .layer(axum::middleware::from_fn_with_state(
                            gate_state,
                            active_service_gate,
                        ));
                app = app.nest(&prefix, service_router);
            }
        }
        app
    }

    /// List all registered service descriptors, sorted by name.
    pub fn list_services(&self) -> Vec<ServiceInfo> {
        let mut list: Vec<ServiceInfo> = self
            .services
            .iter()
            .map(|(&id, svc)| {
                let running = self.contexts.contains_key(&id);
                ServiceInfo {
                    id,
                    descriptor: svc.descriptor().clone(),
                    running,
                    routes: svc.route_info(),
                    config_schema: svc.config_schema(),
                    config: svc.current_config(),
                }
            })
            .collect();
        list.sort_by(|a, b| a.descriptor.name.cmp(&b.descriptor.name));
        list
    }

    /// Collect the union of all registered services' required + owned programs.
    pub fn required_programs(&self) -> HashSet<ProgramId> {
        let mut programs = HashSet::new();
        for service in self.services.values() {
            programs.extend(service.descriptor().all_program_ids());
        }
        programs
    }

    /// Collect the union of programs needed by currently **running** services.
    pub fn active_programs(&self) -> HashSet<ProgramId> {
        let mut programs = HashSet::new();
        for (&id, service) in &self.services {
            if self.contexts.contains_key(&id) {
                programs.extend(service.descriptor().all_program_ids());
            }
        }
        programs
    }

    pub fn event_tx(&self) -> &broadcast::Sender<ServiceEvent> {
        &self.event_tx
    }

    /// Register a gossip handler that can intercept GossipSub messages
    /// on topics it claims via [`ServiceGossipHandler::handles_topic`].
    pub fn register_gossip_handler(&mut self, handler: Arc<dyn ServiceGossipHandler>) {
        self.gossip_handlers.push(handler);
    }

    /// Find the first handler that claims this topic, if any.
    pub fn gossip_handler_for(&self, topic: &str) -> Option<&Arc<dyn ServiceGossipHandler>> {
        self.gossip_handlers.iter().find(|h| h.handles_topic(topic))
    }
}

/// Snapshot info about a registered service.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub id: ServiceId,
    pub descriptor: crate::descriptor::ServiceDescriptor,
    pub running: bool,
    pub routes: Vec<crate::service::RouteInfo>,
    /// Declared configuration fields (empty for services with no settings).
    pub config_schema: Vec<crate::config::ConfigField>,
    /// Current live configuration values as a flat JSON object.
    pub config: serde_json::Value,
}

/// Per-service middleware: returns 503 when the service is not in the active set.
async fn active_service_gate(
    axum::extract::State((service_id, active_set)): axum::extract::State<(
        ServiceId,
        Arc<RwLock<HashSet<ServiceId>>>,
    )>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_active = active_set
        .read()
        .map(|set| set.contains(&service_id))
        .unwrap_or(false);
    if is_active {
        next.run(request).await
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

fn generate_ephemeral_key() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"grid-service-ephemeral-");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.update(now.as_nanos().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use grid_core::{SectorRequest, SectorResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubDispatch;
    impl SectorDispatch for StubDispatch {
        fn dispatch(&self, _req: &SectorRequest) -> SectorResponse {
            unimplemented!("stub")
        }
    }

    struct StubService {
        descriptor: crate::descriptor::ServiceDescriptor,
    }

    impl StubService {
        fn new(name: &str) -> Self {
            Self {
                descriptor: crate::descriptor::ServiceDescriptor {
                    name: name.into(),
                    version: "1.0.0".into(),
                    required_programs: vec![],
                    owned_programs: vec![],
                    summary: String::new(),
                },
            }
        }
    }

    #[async_trait]
    impl Service for StubService {
        fn descriptor(&self) -> &crate::descriptor::ServiceDescriptor {
            &self.descriptor
        }
        fn routes(&self, _ctx: &ServiceContext) -> Router {
            Router::new()
        }
        async fn on_start(&self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
            Ok(())
        }
        async fn on_stop(&self) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    fn make_registry_with_service(name: &str) -> (ServiceRegistry, ServiceId) {
        let mut reg = ServiceRegistry::new();
        let svc = Arc::new(StubService::new(name));
        let id = svc.descriptor.service_id().unwrap();
        reg.register(svc).unwrap();
        (reg, id)
    }

    #[tokio::test]
    async fn start_all_populates_active_set() {
        let (mut reg, id) = make_registry_with_service("alpha");
        let dispatch: Arc<dyn SectorDispatch> = Arc::new(StubDispatch);
        reg.start_all(dispatch).await.unwrap();

        let active = reg.active_services();
        assert!(active.read().unwrap().contains(&id));
        assert!(reg.contexts.contains_key(&id));
    }

    #[tokio::test]
    async fn stop_service_removes_from_active_set() {
        let (mut reg, id) = make_registry_with_service("bravo");
        reg.start_all(Arc::new(StubDispatch)).await.unwrap();

        reg.stop_service(&id).await.unwrap();

        assert!(!reg.active_services().read().unwrap().contains(&id));
        assert!(!reg.contexts.contains_key(&id));
    }

    #[tokio::test]
    async fn start_service_re_adds_to_active_set() {
        let (mut reg, id) = make_registry_with_service("charlie");
        reg.start_all(Arc::new(StubDispatch)).await.unwrap();

        reg.stop_service(&id).await.unwrap();
        reg.start_service(&id).await.unwrap();

        assert!(reg.active_services().read().unwrap().contains(&id));
        assert!(reg.contexts.contains_key(&id));
    }

    #[tokio::test]
    async fn start_service_before_start_all_returns_not_initialized() {
        let (mut reg, id) = make_registry_with_service("delta");
        let err = reg.start_service(&id).await.unwrap_err();
        assert!(
            matches!(err, ServiceError::NotInitialized(_)),
            "expected NotInitialized, got {err:?}"
        );
    }

    #[tokio::test]
    async fn stop_service_unknown_id_returns_not_found() {
        let mut reg = ServiceRegistry::new();
        let fake_id = ServiceId::from([0xAA; 32]);
        let err = reg.stop_service(&fake_id).await.unwrap_err();
        assert!(
            matches!(err, ServiceError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn stop_service_is_idempotent() {
        let (mut reg, id) = make_registry_with_service("echo");
        reg.start_all(Arc::new(StubDispatch)).await.unwrap();

        reg.stop_service(&id).await.unwrap();
        reg.stop_service(&id).await.unwrap();
    }

    #[tokio::test]
    async fn start_service_is_idempotent() {
        let (mut reg, id) = make_registry_with_service("foxtrot");
        reg.start_all(Arc::new(StubDispatch)).await.unwrap();

        reg.start_service(&id).await.unwrap();
        assert!(reg.active_services().read().unwrap().contains(&id));
    }

    #[tokio::test]
    async fn stop_all_clears_active_set() {
        let (mut reg, id) = make_registry_with_service("golf");
        reg.start_all(Arc::new(StubDispatch)).await.unwrap();

        reg.stop_all().await.unwrap();

        assert!(!reg.active_services().read().unwrap().contains(&id));
        assert!(reg.contexts.is_empty());
    }

    struct CountingGossipHandler {
        topic_prefix: String,
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ServiceGossipHandler for CountingGossipHandler {
        fn handles_topic(&self, topic: &str) -> bool {
            topic.starts_with(&self.topic_prefix)
        }

        async fn on_gossip(&self, _topic: &str, _data: &[u8], _sender: Option<String>) {
            self.call_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn gossip_handler_for_returns_matching_handler() {
        let mut reg = ServiceRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        reg.register_gossip_handler(Arc::new(CountingGossipHandler {
            topic_prefix: "consensus/".into(),
            call_count: Arc::clone(&counter),
        }));

        assert!(reg.gossip_handler_for("consensus/epoch-1").is_some());
    }

    #[test]
    fn gossip_handler_for_returns_none_on_unhandled_topic() {
        let mut reg = ServiceRegistry::new();
        reg.register_gossip_handler(Arc::new(CountingGossipHandler {
            topic_prefix: "consensus/".into(),
            call_count: Arc::new(AtomicUsize::new(0)),
        }));

        assert!(reg.gossip_handler_for("prog/abc123").is_none());
    }

    #[tokio::test]
    async fn gossip_handler_receives_messages() {
        let mut reg = ServiceRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        reg.register_gossip_handler(Arc::new(CountingGossipHandler {
            topic_prefix: "my-svc/".into(),
            call_count: Arc::clone(&counter),
        }));

        let handler = reg.gossip_handler_for("my-svc/topic").unwrap();
        handler
            .on_gossip("my-svc/topic", b"payload", Some("peer1".into()))
            .await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
