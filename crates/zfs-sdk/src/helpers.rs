use zfs_programs::{ZChatDescriptor, ZidDescriptor};

/// Create the canonical ZID v1 program descriptor.
pub fn zid_descriptor() -> ZidDescriptor {
    ZidDescriptor::v1()
}

/// Create the canonical Interlink v1 program descriptor.
pub fn zchat_descriptor() -> ZChatDescriptor {
    ZChatDescriptor::v1()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zid_descriptor_has_deterministic_program_id() {
        let d1 = zid_descriptor();
        let d2 = zid_descriptor();
        assert_eq!(d1.program_id().unwrap(), d2.program_id().unwrap(),);
    }

    #[test]
    fn zchat_descriptor_has_deterministic_program_id() {
        let d1 = zchat_descriptor();
        let d2 = zchat_descriptor();
        assert_eq!(d1.program_id().unwrap(), d2.program_id().unwrap(),);
    }

    #[test]
    fn zid_and_zchat_have_different_program_ids() {
        let zid = zid_descriptor().program_id().unwrap();
        let zchat = zchat_descriptor().program_id().unwrap();
        assert_ne!(zid, zchat);
    }
}
