use grid_core::{GridError, ProgramId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// A program owned by a service, pairing the authoritative [`ProgramId`]
/// (derived from the program's own canonical descriptor) with display
/// metadata for the UI.
#[derive(Debug, Clone)]
pub struct OwnedProgram {
    pub name: String,
    pub version: String,
    pub program_id: ProgramId,
}

/// Canonical descriptor for a Grid Service.
///
/// Mirrors the Program pattern: a canonical CBOR-serialized descriptor hashed
/// to produce a deterministic [`ServiceId`]. Services can both depend on
/// existing Programs (`required_programs`) and define new Programs that they
/// own (`owned_programs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    /// Human-readable service name (e.g. `"IDENTITY"`).
    pub name: String,
    /// SemVer version string for this service descriptor.
    pub version: String,
    /// Programs this service reads/writes (must already exist on the Zode).
    pub required_programs: Vec<ProgramId>,
    /// Programs this service defines and owns, each carrying the
    /// authoritative [`ProgramId`] derived from the program's own descriptor.
    #[serde(skip)]
    pub owned_programs: Vec<OwnedProgram>,
    /// Short human-readable summary shown in the ZODE UI.
    /// Excluded from canonical encoding so it does not affect the [`ServiceId`].
    #[serde(skip)]
    pub summary: String,
}

/// 32-byte service identity: `SHA-256(canonical_cbor(ServiceDescriptor))`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServiceId(#[serde(with = "serde_bytes")] [u8; 32]);

impl ServiceDescriptor {
    /// Compute the deterministic [`ServiceId`] by hashing the canonical CBOR encoding.
    pub fn service_id(&self) -> Result<ServiceId, GridError> {
        let bytes = grid_core::encode_canonical(self)?;
        let hash = Sha256::digest(&bytes);
        Ok(ServiceId(hash.into()))
    }

    /// GossipSub topic for service discovery: `svc/{service_id_hex}`.
    pub fn topic(&self) -> Result<String, GridError> {
        let id = self.service_id()?;
        Ok(format!("svc/{}", hex::encode(id.0)))
    }

    /// All program IDs this service needs: required + owned.
    pub fn all_program_ids(&self) -> Vec<ProgramId> {
        let mut ids: Vec<ProgramId> = self.required_programs.clone();
        for op in &self.owned_programs {
            ids.push(op.program_id);
        }
        ids
    }
}

impl ServiceId {
    /// Return the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Hex-encode the 32-byte id.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse a [`ServiceId`] from a 64-char hex string.
    pub fn from_hex(s: &str) -> Result<Self, GridError> {
        let bytes = hex::decode(s).map_err(|e| GridError::Decode(e.to_string()))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| {
            GridError::Decode("ServiceId hex must decode to exactly 32 bytes".into())
        })?;
        Ok(Self(arr))
    }
}

impl From<[u8; 32]> for ServiceId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceId({})", self.to_hex())
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_id_is_deterministic() {
        let desc = ServiceDescriptor {
            name: "test-service".into(),
            version: "1.0.0".into(),
            required_programs: vec![],
            owned_programs: vec![],
            summary: String::new(),
        };
        let id1 = desc.service_id().unwrap();
        let id2 = desc.service_id().unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_descriptors_produce_different_ids() {
        let desc1 = ServiceDescriptor {
            name: "svc-a".into(),
            version: "1.0.0".into(),
            required_programs: vec![],
            owned_programs: vec![],
            summary: String::new(),
        };
        let desc2 = ServiceDescriptor {
            name: "svc-b".into(),
            version: "1.0.0".into(),
            required_programs: vec![],
            owned_programs: vec![],
            summary: String::new(),
        };
        assert_ne!(desc1.service_id().unwrap(), desc2.service_id().unwrap());
    }

    #[test]
    fn topic_format() {
        let desc = ServiceDescriptor {
            name: "test".into(),
            version: "1.0.0".into(),
            required_programs: vec![],
            owned_programs: vec![],
            summary: String::new(),
        };
        let topic = desc.topic().unwrap();
        assert!(topic.starts_with("svc/"));
        assert_eq!(topic.len(), 4 + 64);
    }

    #[test]
    fn service_id_hex_round_trip() {
        let desc = ServiceDescriptor {
            name: "rt".into(),
            version: "0.1.0".into(),
            required_programs: vec![],
            owned_programs: vec![],
            summary: String::new(),
        };
        let id = desc.service_id().unwrap();
        let hex_str = id.to_hex();
        let parsed = ServiceId::from_hex(&hex_str).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn all_program_ids_includes_required_and_owned() {
        let owned_pid = ProgramId::from([0xBB; 32]);
        let owned = OwnedProgram {
            name: "owned-prog".into(),
            version: "1.0.0".into(),
            program_id: owned_pid,
        };
        let required_pid = ProgramId::from([0xAA; 32]);
        let desc = ServiceDescriptor {
            name: "svc".into(),
            version: "1.0.0".into(),
            required_programs: vec![required_pid],
            owned_programs: vec![owned],
            summary: String::new(),
        };
        let all = desc.all_program_ids();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], required_pid);
        assert_eq!(all[1], owned_pid);
    }
}
