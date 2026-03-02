use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::config::RpcConfig;
use crate::dispatch::{self, JsonRpcRequest};
use crate::error::RpcError;
use crate::SectorDispatch;

#[derive(Clone)]
struct AppState {
    handler: Arc<dyn SectorDispatch>,
    api_key: Option<String>,
    requests_total: Arc<AtomicU64>,
}

/// A running JSON-RPC HTTP server.
pub struct RpcServer {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
    requests_total: Arc<AtomicU64>,
    bind_addr: SocketAddr,
    auth_required: bool,
}

impl RpcServer {
    /// Start the RPC server with the given config, sector dispatch handler, and
    /// an optional service router to merge into the HTTP server.
    pub async fn start(
        config: &RpcConfig,
        handler: Arc<dyn SectorDispatch>,
        service_router: Option<Router>,
    ) -> Result<Self, RpcError> {
        let requests_total = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let auth_required = config.api_key.is_some();
        let state = AppState {
            handler,
            api_key: config.api_key.clone(),
            requests_total: Arc::clone(&requests_total),
        };

        let mut app = Router::new()
            .route("/rpc", post(rpc_handler))
            .with_state(state);

        if let Some(svc_router) = service_router {
            app = app.merge(svc_router);
        }

        let app = app
            .layer(axum::extract::DefaultBodyLimit::max(5 * 1024 * 1024))
            .layer(CorsLayer::permissive());

        let listener = tokio::net::TcpListener::bind(config.bind_addr)
            .await
            .map_err(RpcError::Bind)?;
        let actual_addr = listener.local_addr().map_err(RpcError::Bind)?;
        info!(addr = %actual_addr, "RPC server listening");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
            requests_total,
            bind_addr: actual_addr,
            auth_required,
        })
    }

    /// Total number of JSON-RPC requests received.
    pub fn requests_total(&self) -> u64 {
        self.requests_total.load(Ordering::Relaxed)
    }

    /// The address the server is listening on.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Whether API key authentication is required.
    pub fn auth_required(&self) -> bool {
        self.auth_required
    }

    /// Gracefully shut down the RPC server.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
        info!("RPC server shut down");
    }
}

async fn rpc_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if let Some(ref expected_key) = state.api_key {
        let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());

        let authorized = match auth_header {
            Some(value) if value.starts_with("Bearer ") => {
                constant_time_eq(&value.as_bytes()[7..], expected_key.as_bytes())
            }
            _ => false,
        };

        if !authorized {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    state.requests_total.fetch_add(1, Ordering::Relaxed);

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::OK, Json(dispatch::parse_error(&e.to_string()))).into_response();
        }
    };

    let response = dispatch::dispatch(&*state.handler, &request);
    (StatusCode::OK, Json(response)).into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
