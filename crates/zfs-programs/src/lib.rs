#![forbid(unsafe_code)]

mod topic;
pub mod zchat;
pub mod zid;

pub use topic::program_topic;
pub use zchat::{ChannelId, ZChatDescriptor, ZChatMessage};
pub use zid::{ZidDescriptor, ZidMessage};

use zfs_core::ProgramId;

/// Returns the ProgramIds of the v0.1.0 default programs (ZID and Interlink).
///
/// These are the standard programs a Zode subscribes to out of the box.
/// Each entry is `(human_name, program_id)`.
pub fn default_program_ids() -> Vec<(&'static str, ProgramId)> {
    let mut out = Vec::with_capacity(2);
    if let Ok(pid) = ZidDescriptor::v1().program_id() {
        out.push(("ZID", pid));
    }
    if let Ok(pid) = ZChatDescriptor::v1().program_id() {
        out.push(("Interlink", pid));
    }
    out
}

#[cfg(test)]
mod tests;
