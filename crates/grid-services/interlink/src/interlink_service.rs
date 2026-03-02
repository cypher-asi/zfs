use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use grid_core::ProgramId;
use grid_programs_interlink::{InterlinkDescriptor, ZMessage};
use grid_service::{RouteInfo, Service, ServiceContext, ServiceDescriptor, ServiceError};
use serde::Deserialize;
use std::sync::Arc;

pub struct InterlinkService {
    descriptor: ServiceDescriptor,
    interlink_program_id: ProgramId,
}

impl InterlinkService {
    pub fn new() -> Result<Self, ServiceError> {
        let pid = InterlinkDescriptor::v2()
            .program_id()
            .map_err(|e| ServiceError::Descriptor(e.to_string()))?;

        Ok(Self {
            descriptor: ServiceDescriptor {
                name: "INTERLINK".into(),
                version: "1.0.0".into(),
                required_programs: vec![pid],
                owned_programs: vec![],
            },
            interlink_program_id: pid,
        })
    }
}

impl Default for InterlinkService {
    fn default() -> Self {
        // INVARIANT: Interlink descriptor program_id uses compile-time constants.
        Self::new().expect("interlink service descriptor should be valid")
    }
}

#[async_trait]
impl Service for InterlinkService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn routes(&self, ctx: &ServiceContext) -> Router {
        let store = Arc::new(ctx.store(&self.interlink_program_id));
        Router::new()
            .route("/messages", get(get_messages))
            .route("/health", get(health_handler))
            .with_state(store)
    }

    async fn on_start(&self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
        tracing::info!("Interlink service started");
        Ok(())
    }

    async fn on_stop(&self) -> Result<(), ServiceError> {
        tracing::info!("Interlink service stopped");
        Ok(())
    }

    fn route_info(&self) -> Vec<RouteInfo> {
        vec![
            RouteInfo {
                method: "GET",
                path: "/messages",
                description: "Fetch messages for a channel",
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
struct MessagesQuery {
    channel: String,
    #[serde(default)]
    from: u64,
}

const MAX_CHANNEL_LEN: usize = 256;

async fn get_messages(
    State(store): State<Arc<grid_service::ProgramStore>>,
    Query(query): Query<MessagesQuery>,
) -> impl IntoResponse {
    if query.channel.is_empty() || query.channel.len() > MAX_CHANNEL_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid channel name length" })),
        )
            .into_response();
    }
    let channel_id = grid_programs_interlink::ChannelId::from_str_id(&query.channel);
    let sector_id_bytes = channel_id.sector_id();
    let key = sector_id_bytes.as_bytes();

    match store.list_from(key, query.from) {
        Ok(entries) => {
            let messages: Vec<serde_json::Value> = entries
                .iter()
                .filter_map(|bytes| {
                    ZMessage::decode_canonical(bytes).ok().map(|msg| {
                        serde_json::json!({
                            "sender_did": msg.sender_did,
                            "content": msg.content,
                            "timestamp_ms": msg.timestamp_ms,
                        })
                    })
                })
                .collect();

            Json(serde_json::json!({
                "channel": query.channel,
                "messages": messages,
                "count": messages.len(),
            }))
            .into_response()
        }
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
