use tokio::sync::broadcast;
use tracing::{info, warn};
use zfs_core::GossipSectorAppend;
use zfs_storage::SectorStore;

use crate::sector_handler::SectorRequestHandler;
use crate::types::LogEvent;

pub(crate) fn handle_gossip_message<S: SectorStore>(
    sector_handler: &SectorRequestHandler<S>,
    event_tx: &broadcast::Sender<LogEvent>,
    topic: &str,
    data: &[u8],
) {
    info!(%topic, bytes = data.len(), "gossip message received");
    match zfs_core::decode_canonical::<GossipSectorAppend>(data) {
        Ok(msg) => {
            let accepted = sector_handler.handle_gossip_append(&msg);
            info!(
                program_id = %msg.program_id,
                sector_id = %msg.sector_id.to_hex(),
                index = msg.index,
                accepted,
                "gossip sector append processed"
            );
            let _ = event_tx.send(LogEvent::GossipSectorReceived {
                program_id: msg.program_id.to_hex(),
                sector_id: msg.sector_id.to_hex(),
                accepted,
            });
        }
        Err(e) => {
            warn!(%topic, error = %e, "failed to decode gossip message");
        }
    }
}
