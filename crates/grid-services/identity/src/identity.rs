use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_zid::{ZidDescriptor, ZidMessage};
use grid_service::{Service, ServiceContext, ServiceDescriptor, ServiceError};
use serde::Deserialize;
use std::sync::Arc;

pub struct IdentityService {
    descriptor: ServiceDescriptor,
    zid_v1_program_id: ProgramId,
}

impl IdentityService {
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
                name: "identity".into(),
                version: "1.0.0".into(),
                required_programs: required,
                owned_programs: vec![],
            },
            zid_v1_program_id: zid_v1_pid,
        })
    }
}

impl Default for IdentityService {
    fn default() -> Self {
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
}

#[derive(Deserialize)]
struct ResolveRequest {
    did: String,
}

async fn resolve_handler(
    State(store): State<Arc<grid_service::ProgramStore>>,
    Json(req): Json<ResolveRequest>,
) -> impl IntoResponse {
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
