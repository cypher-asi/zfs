use grid_core::GossipSectorAppend;
use grid_service::ServiceRegistry;
use grid_storage::SectorStore;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::sector_handler::SectorRequestHandler;
use crate::types::LogEvent;

pub(crate) async fn handle_gossip_message<S: SectorStore>(
    sector_handler: &SectorRequestHandler<S>,
    service_registry: &ServiceRegistry,
    event_tx: &broadcast::Sender<LogEvent>,
    topic: &str,
    data: &[u8],
    sender: Option<String>,
) {
    info!(%topic, bytes = data.len(), ?sender, "gossip message received");

    if let Some(handler) = service_registry.gossip_handler_for(topic) {
        handler.on_gossip(topic, data, sender).await;
        return;
    }

    match grid_core::decode_canonical::<GossipSectorAppend>(data) {
        Ok(msg) => {
            let result = sector_handler.handle_gossip_append(&msg);
            let accepted = result.is_accepted();
            info!(
                program_id = %msg.program_id,
                sector_id = %msg.sector_id.to_hex(),
                index = msg.index,
                accepted,
                "gossip sector append processed"
            );
            let _ = event_tx.send(LogEvent::GossipSectorReceived {
                sender,
                program_id: msg.program_id.to_hex(),
                sector_id: msg.sector_id.to_hex(),
                result,
            });
        }
        Err(e) => {
            warn!(%topic, error = %e, "failed to decode gossip message");
        }
    }
}
