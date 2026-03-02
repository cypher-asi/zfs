use crate::ProgramId;

/// Build the GossipSub topic string for a given program ID.
///
/// Format: `prog/<program_id_hex>` (64 lowercase hex characters).
pub fn program_topic(program_id: &ProgramId) -> String {
    format!("prog/{}", program_id.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_has_prog_prefix() {
        let id = ProgramId::from([0xAA; 32]);
        let topic = program_topic(&id);
        assert!(topic.starts_with("prog/"));
    }

    #[test]
    fn topic_contains_hex_id() {
        let id = ProgramId::from([0xAA; 32]);
        let topic = program_topic(&id);
        assert_eq!(topic, format!("prog/{}", id.to_hex()));
    }

    #[test]
    fn topic_hex_is_lowercase_64_chars() {
        let id = ProgramId::from([0xFF; 32]);
        let topic = program_topic(&id);
        let hex_part = topic.strip_prefix("prog/").unwrap();
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn zero_program_id() {
        let id = ProgramId::from([0u8; 32]);
        let topic = program_topic(&id);
        assert_eq!(topic, format!("prog/{}", "00".repeat(32)));
    }

    #[test]
    fn different_ids_produce_different_topics() {
        let id_a = ProgramId::from([1u8; 32]);
        let id_b = ProgramId::from([2u8; 32]);
        assert_ne!(program_topic(&id_a), program_topic(&id_b));
    }
}
