use async_trait::async_trait;
use axum::Router;

use crate::context::ServiceContext;
use crate::descriptor::ServiceDescriptor;
use crate::error::ServiceError;

/// The core trait implemented by all Grid Services (native or WASM-bridged).
///
/// - `descriptor()` — returns the service's identity and program requirements
/// - `routes()` — returns an axum `Router` mounted at `/services/{service_id}/`
/// - `on_start()` — called after boot; spawn background tasks using `ctx.shutdown`
/// - `on_stop()` — cleanup hook during Zode shutdown
#[async_trait]
pub trait Service: Send + Sync + 'static {
    fn descriptor(&self) -> &ServiceDescriptor;

    fn routes(&self, ctx: &ServiceContext) -> Router;

    async fn on_start(&self, ctx: &ServiceContext) -> Result<(), ServiceError>;

    async fn on_stop(&self) -> Result<(), ServiceError>;
}
