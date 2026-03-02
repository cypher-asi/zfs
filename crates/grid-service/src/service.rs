use async_trait::async_trait;
use axum::Router;

use crate::config::ConfigField;
use crate::context::ServiceContext;
use crate::descriptor::ServiceDescriptor;
use crate::error::ServiceError;

/// HTTP route metadata exposed by a service for UI introspection.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub method: &'static str,
    pub path: &'static str,
    pub description: &'static str,
}

/// The core trait implemented by all Grid Services (native or WASM-bridged).
///
/// - `descriptor()` — returns the service's identity and program requirements
/// - `routes()` — returns an axum `Router` mounted at `/services/{service_id}/`
/// - `on_start()` — called after boot; spawn background tasks using `ctx.shutdown`
/// - `on_stop()` — cleanup hook during Zode shutdown
/// - `route_info()` — returns metadata about exposed HTTP routes
/// - `config_schema()` — declares configurable fields for the generic settings UI
/// - `current_config()` — returns the service's active configuration as JSON
#[async_trait]
pub trait Service: Send + Sync + 'static {
    fn descriptor(&self) -> &ServiceDescriptor;

    fn routes(&self, ctx: &ServiceContext) -> Router;

    async fn on_start(&self, ctx: &ServiceContext) -> Result<(), ServiceError>;

    async fn on_stop(&self) -> Result<(), ServiceError>;

    fn route_info(&self) -> Vec<RouteInfo> {
        vec![]
    }

    /// Declare configurable fields. Services with no settings return `vec![]`.
    fn config_schema(&self) -> Vec<ConfigField> {
        vec![]
    }

    /// Snapshot of live configuration values as a flat JSON object.
    fn current_config(&self) -> serde_json::Value {
        serde_json::Value::Object(serde_json::Map::new())
    }
}
