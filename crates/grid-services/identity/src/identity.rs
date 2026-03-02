use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_zid::{ZidDescriptor, ZidMessage};
use grid_service::{RouteInfo, Service, ServiceContext, ServiceDescriptor, ServiceError};
use serde::Deserialize;
use std::sync::Arc;

/// Service that resolves DIDs to their on-Zode identity records.
pub struct IdentityService {
    descriptor: ServiceDescriptor,
    zid_v1_program_id: ProgramId,
}

impl IdentityService {
    /// Create a new `IdentityService`, resolving required ZID program IDs.
    pub fn new() -> Result<Self, ServiceError> {
        let zid_v1_pid = ZidDescriptor::v1()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        let mut required = vec![zid_v1_pid];
        if let Ok(v2_pid) = ZidDescriptor::v2().program_id() {
            required.push(v2_pid);
        }

        Ok(Self {
            descriptor: ServiceDescriptor {
                name: "IDENTITY".into(),
                version: "1.0.0".into(),
                required_programs: required,
                owned_programs: vec![],
                summary: "A sovereign identity system backed by post-quantum cryptography.".into(),
            },
            zid_v1_program_id: zid_v1_pid,
        })
    }
}

impl Default for IdentityService {
    fn default() -> Self {
        // INVARIANT: ZID descriptor program_id uses compile-time constants.
        Self::new().expect("identity service descriptor should be valid")
    }
}

#[async_trait]
impl Service for IdentityService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn routes(&self, ctx: &ServiceContext) -> Router {
        let store = Arc::new(ctx.store(&self.zid_v1_program_id));
        Router::new()
            .route("/resolve", post(resolve_handler))
            .route("/health", get(health_handler))
            .with_state(store)
    }

    async fn on_start(&self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
        tracing::info!("Identity service started");
        Ok(())
    }

    async fn on_stop(&self) -> Result<(), ServiceError> {
        tracing::info!("Identity service stopped");
        Ok(())
    }

    fn route_info(&self) -> Vec<RouteInfo> {
        vec![
            RouteInfo {
                method: "POST",
                path: "/resolve",
                description: "Resolve a DID to its identity record",
            },
            RouteInfo {
                method: "GET",
                path: "/health",
                description: "Service health check",
            },
        ]
    }
}

#[derive(Deserialize)]
struct ResolveRequest {
    did: String,
}

const MAX_DID_LEN: usize = 256;

async fn resolve_handler(
    State(store): State<Arc<grid_service::ProgramStore>>,
    Json(req): Json<ResolveRequest>,
) -> impl IntoResponse {
    if req.did.is_empty() || req.did.len() > MAX_DID_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid DID length" })),
        )
            .into_response();
    }
    let key = format!("did/{}", req.did);
    match store.get(key.as_bytes()) {
        Ok(Some(bytes)) => match grid_core::decode_canonical::<ZidMessage>(&bytes) {
            Ok(msg) => Json(serde_json::json!({
                "did": req.did,
                "record": {
                    "owner_did": msg.owner_did,
                    "display_name": msg.display_name,
                    "timestamp_ms": msg.timestamp_ms,
                }
            }))
            .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Ok(None) => Json(serde_json::json!({
            "did": req.did,
            "record": null
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_service::Service;

    #[test]
    fn new_succeeds() {
        let svc = IdentityService::new().expect("IdentityService::new() should succeed");
        assert_eq!(svc.descriptor.name, "IDENTITY");
    }

    #[test]
    fn descriptor_has_expected_name_and_version() {
        let svc = IdentityService::default();
        let desc = svc.descriptor();
        assert_eq!(desc.name, "IDENTITY");
        assert_eq!(desc.version, "1.0.0");
        assert!(
            !desc.required_programs.is_empty(),
            "should require at least one program"
        );
    }

    #[test]
    fn route_info_contains_expected_paths() {
        let svc = IdentityService::default();
        let routes = svc.route_info();

        assert_eq!(routes.len(), 2);

        assert_eq!(routes[0].method, "POST");
        assert_eq!(routes[0].path, "/resolve");

        assert_eq!(routes[1].method, "GET");
        assert_eq!(routes[1].path, "/health");
    }
}
