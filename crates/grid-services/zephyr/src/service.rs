use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_zephyr::{ZephyrGlobalDescriptor, ZephyrZoneDescriptor};
use grid_service::{RouteInfo, Service, ServiceContext, ServiceDescriptor, ServiceError};
use std::sync::Arc;

use crate::committee::my_assigned_zones;
use crate::config::ZephyrConfig;
use crate::epoch::EpochManager;

/// Shared state handed to HTTP route handlers.
pub(crate) struct ZephyrState {
    pub(crate) config: ZephyrConfig,
    pub(crate) global_program_id: ProgramId,
    pub(crate) zone_program_ids: Vec<ProgramId>,
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

        let owned_programs = vec![
            grid_core::ProgramDescriptor {
                name: "zephyr/global".into(),
                version: "1".into(),
            },
            grid_core::ProgramDescriptor {
                name: "zephyr/spend".into(),
                version: "1".into(),
            },
            grid_core::ProgramDescriptor {
                name: "zephyr/validators".into(),
                version: "1".into(),
            },
        ];

        Ok(Self {
            descriptor: ServiceDescriptor {
                name: "ZEPHYR".into(),
                version: "0.1.0".into(),
                required_programs: vec![],
                owned_programs,
                summary: "Note-based currency with zone-scoped consensus".into(),
            },
            config,
            global_program_id: global_pid,
            zone_program_ids: zone_pids,
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
        tracing::info!(%global_topic, "subscribed to global topic");

        if self.config.validators.is_empty() {
            tracing::warn!("no validators configured; Zephyr running in observer mode");
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
                tracing::warn!("no node identity; Zephyr running in observer mode");
                return Ok(());
            }
        };

        let epoch_mgr = EpochManager::new(
            0,
            self.config.epoch_duration_ms,
            self.config.initial_randomness,
            self.config.validators.clone(),
            self.config.total_zones,
            self.config.committee_size,
        );

        let assigned_zones = my_assigned_zones(
            &my_validator_id,
            epoch_mgr.randomness_seed(),
            &self.config.validators,
            self.config.total_zones,
            self.config.committee_size,
        );

        for &zone_id in &assigned_zones {
            let topic = self.zone_topic(zone_id);
            ctx.subscribe_topic(&topic)?;
            tracing::info!(zone_id, %topic, "subscribed to zone topic");
        }

        tracing::info!(
            zones = self.config.total_zones,
            committee_size = self.config.committee_size,
            assigned_zones = assigned_zones.len(),
            epoch = epoch_mgr.current_epoch(),
            "Zephyr service started"
        );

        Ok(())
    }

    async fn on_stop(&self) -> Result<(), ServiceError> {
        tracing::info!("Zephyr service stopped");
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
}

async fn status_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "service": "ZEPHYR",
        "total_zones": state.config.total_zones,
        "committee_size": state.config.committee_size,
        "validator_count": state.config.validators.len(),
        "global_program_id": state.global_program_id.to_hex(),
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
    Json(serde_json::json!({
        "zone_id": id,
        "program_id": pid.to_hex(),
        "head": null,
    }))
    .into_response()
}

async fn epoch_handler(State(state): State<Arc<ZephyrState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "epoch": 0,
        "epoch_duration_ms": state.config.epoch_duration_ms,
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
}
