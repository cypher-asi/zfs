use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Router;
use grid_core::ProgramId;
use grid_rpc::SectorDispatch;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::context::{ServiceContext, ServiceEvent};
use crate::descriptor::ServiceId;
use crate::error::ServiceError;
use crate::service::Service;

/// Manages the lifecycle of all active services on a Zode.
pub struct ServiceRegistry {
    services: HashMap<ServiceId, Arc<dyn Service>>,
    contexts: HashMap<ServiceId, ServiceContext>,
    event_tx: broadcast::Sender<ServiceEvent>,
    shutdown: CancellationToken,
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
            event_tx,
            shutdown: CancellationToken::new(),
        }
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
    /// calls `on_start()`.
    pub async fn start_all(
        &mut self,
        sector_dispatch: Arc<dyn SectorDispatch>,
    ) -> Result<(), ServiceError> {
        let ephemeral_key = generate_ephemeral_key();

        for (&id, service) in &self.services {
            let ctx = ServiceContext::new(
                id,
                Arc::clone(&sector_dispatch),
                ephemeral_key,
                self.event_tx.clone(),
                self.shutdown.child_token(),
            );

            if let Err(e) = service.on_start(&ctx).await {
                error!(
                    name = %service.descriptor().name,
                    error = %e,
                    "service failed to start"
                );
                return Err(e);
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
        Ok(())
    }

    /// Build a merged axum `Router` with all service routes mounted at
    /// `/services/{service_id_hex}/`.
    pub fn merged_router(&self) -> Router {
        let mut app = Router::new();
        for (&id, service) in &self.services {
            if let Some(ctx) = self.contexts.get(&id) {
                let prefix = format!("/services/{}", id.to_hex());
                let service_router = service.routes(ctx);
                app = app.nest(&prefix, service_router);
            }
        }
        app
    }

    /// List all registered service descriptors.
    pub fn list_services(&self) -> Vec<ServiceInfo> {
        self.services
            .iter()
            .map(|(&id, svc)| {
                let running = self.contexts.contains_key(&id);
                ServiceInfo {
                    id,
                    descriptor: svc.descriptor().clone(),
                    running,
                }
            })
            .collect()
    }

    /// Collect the union of all registered services' required + owned programs.
    pub fn required_programs(&self) -> HashSet<ProgramId> {
        let mut programs = HashSet::new();
        for service in self.services.values() {
            if let Ok(ids) = service.descriptor().all_program_ids() {
                programs.extend(ids);
            }
        }
        programs
    }

    pub fn event_tx(&self) -> &broadcast::Sender<ServiceEvent> {
        &self.event_tx
    }
}

/// Snapshot info about a registered service.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub id: ServiceId,
    pub descriptor: crate::descriptor::ServiceDescriptor,
    pub running: bool,
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
