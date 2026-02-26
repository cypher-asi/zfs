use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// CBOR major type classification for field-shape validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CborType {
    UnsignedInt,
    NegativeInt,
    ByteString,
    TextString,
    Array,
    Map,
    Bool,
    Null,
}

/// A single field in a program's message schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
    pub key: String,
    pub value_type: CborType,
    /// If true, the field may be CBOR null.
    pub optional: bool,
}

/// Canonical schema for a program's message shape.
///
/// Used as a public circuit input (via `schema_hash`) to bind Groth16
/// proofs to a specific message structure without revealing plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSchema {
    pub program_name: String,
    pub version: u32,
    pub fields: Vec<FieldDef>,
}

impl FieldSchema {
    /// Deterministic hash used as a public circuit input.
    ///
    /// `SHA-256(canonical_cbor(self))` — deterministic because canonical
    /// CBOR has a single valid encoding for any value.
    pub fn schema_hash(&self) -> [u8; 32] {
        let bytes = crate::encode_canonical(self).expect("FieldSchema is always serializable");
        Sha256::digest(&bytes).into()
    }
}

/// Proof system selector for per-program verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProofSystem {
    None,
    Groth16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_hash_deterministic() {
        let schema1 = FieldSchema {
            program_name: "test".into(),
            version: 1,
            fields: vec![
                FieldDef { key: "a".into(), value_type: CborType::TextString, optional: false },
                FieldDef { key: "b".into(), value_type: CborType::UnsignedInt, optional: true },
            ],
        };
        let schema2 = FieldSchema {
            program_name: "test".into(),
            version: 1,
            fields: vec![
                FieldDef { key: "a".into(), value_type: CborType::TextString, optional: false },
                FieldDef { key: "b".into(), value_type: CborType::UnsignedInt, optional: true },
            ],
        };
        assert_eq!(schema1.schema_hash(), schema2.schema_hash());
    }

    #[test]
    fn schema_hash_changes_on_different_fields() {
        let schema1 = FieldSchema {
            program_name: "test".into(),
            version: 1,
            fields: vec![
                FieldDef { key: "a".into(), value_type: CborType::TextString, optional: false },
            ],
        };
        let schema2 = FieldSchema {
            program_name: "test".into(),
            version: 1,
            fields: vec![
                FieldDef { key: "b".into(), value_type: CborType::ByteString, optional: true },
            ],
        };
        assert_ne!(schema1.schema_hash(), schema2.schema_hash());
    }

    #[test]
    fn proof_system_serialization_round_trip() {
        let groth16 = ProofSystem::Groth16;
        let none = ProofSystem::None;

        let g_bytes = crate::encode_canonical(&groth16).expect("encode Groth16");
        let g_decoded: ProofSystem = crate::decode_canonical(&g_bytes).expect("decode Groth16");
        assert_eq!(groth16, g_decoded);

        let n_bytes = crate::encode_canonical(&none).expect("encode None");
        let n_decoded: ProofSystem = crate::decode_canonical(&n_bytes).expect("decode None");
        assert_eq!(none, n_decoded);
    }
}
