use tokio::sync::broadcast;
use tracing::{info, warn};
use zfs_core::GossipSector;
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
    match zfs_core::decode_canonical::<GossipSector>(data) {
        Ok(sector) => {
            let accepted = sector_handler.handle_gossip_sector(&sector);
            info!(
                program_id = %sector.program_id,
                sector_id = %sector.sector_id.to_hex(),
                accepted,
                "gossip sector processed"
            );
            let _ = event_tx.send(LogEvent::GossipSectorReceived {
                program_id: sector.program_id.to_hex(),
                sector_id: sector.sector_id.to_hex(),
                accepted,
            });
        }
        Err(e) => {
            warn!(%topic, error = %e, "failed to decode gossip message as GossipSector");
        }
    }
}
