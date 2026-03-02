use grid_programs_zephyr::{ValidatorInfo, ZoneId};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};

/// Deterministic committee sampling for a zone in an epoch.
///
/// Uses Fisher-Yates partial shuffle seeded by `SHA-256(randomness_seed || zone_id)`.
/// All validators running the same inputs derive identical committees.
pub fn sample_committee(
    randomness_seed: &[u8; 32],
    zone_id: ZoneId,
    validators: &[ValidatorInfo],
    committee_size: usize,
) -> Vec<ValidatorInfo> {
    if validators.is_empty() {
        return vec![];
    }

    let mut seed_input = Vec::with_capacity(36);
    seed_input.extend_from_slice(randomness_seed);
    seed_input.extend_from_slice(&zone_id.to_be_bytes());
    let seed = Sha256::digest(&seed_input);
    let mut rng = ChaCha20Rng::from_seed(seed.into());

    let mut indices: Vec<usize> = (0..validators.len()).collect();
    let k = committee_size.min(indices.len());

    for i in 0..k {
        let j = rng.gen_range(i..indices.len());
        indices.swap(i, j);
    }

    indices[..k]
        .iter()
        .map(|&i| validators[i].clone())
        .collect()
}

/// Compute all zone assignments for a validator in a given epoch.
pub fn my_assigned_zones(
    my_validator_id: &[u8; 32],
    randomness_seed: &[u8; 32],
    validators: &[ValidatorInfo],
    total_zones: u32,
    committee_size: usize,
) -> Vec<ZoneId> {
    let mut zones = Vec::new();
    for zone_id in 0..total_zones {
        let committee = sample_committee(randomness_seed, zone_id, validators, committee_size);
        if committee.iter().any(|v| v.validator_id == *my_validator_id) {
            zones.push(zone_id);
        }
    }
    zones
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validators(n: usize) -> Vec<ValidatorInfo> {
        (0..n)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = i as u8;
                ValidatorInfo {
                    validator_id: id,
                    pubkey: id,
                    p2p_endpoint: format!("/ip4/127.0.0.1/tcp/{}", 4000 + i),
                }
            })
            .collect()
    }

    #[test]
    fn deterministic_same_inputs_same_committee() {
        let seed = [0xABu8; 32];
        let validators = make_validators(20);
        let c1 = sample_committee(&seed, 5, &validators, 5);
        let c2 = sample_committee(&seed, 5, &validators, 5);
        assert_eq!(c1, c2);
    }

    #[test]
    fn committee_size_respected() {
        let seed = [1u8; 32];
        let validators = make_validators(20);
        let committee = sample_committee(&seed, 0, &validators, 5);
        assert_eq!(committee.len(), 5);
    }

    #[test]
    fn committee_size_capped_at_validator_count() {
        let seed = [2u8; 32];
        let validators = make_validators(3);
        let committee = sample_committee(&seed, 0, &validators, 10);
        assert_eq!(committee.len(), 3);
    }

    #[test]
    fn different_zones_get_different_committees() {
        let seed = [3u8; 32];
        let validators = make_validators(20);
        let c1 = sample_committee(&seed, 0, &validators, 5);
        let c2 = sample_committee(&seed, 1, &validators, 5);
        let ids1: Vec<_> = c1.iter().map(|v| v.validator_id).collect();
        let ids2: Vec<_> = c2.iter().map(|v| v.validator_id).collect();
        assert_ne!(ids1, ids2);
    }

    #[test]
    fn no_duplicates_in_committee() {
        let seed = [4u8; 32];
        let validators = make_validators(20);
        let committee = sample_committee(&seed, 0, &validators, 10);
        let mut ids: Vec<_> = committee.iter().map(|v| v.validator_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), committee.len());
    }

    #[test]
    fn empty_validators_returns_empty() {
        let seed = [5u8; 32];
        let committee = sample_committee(&seed, 0, &[], 5);
        assert!(committee.is_empty());
    }

    #[test]
    fn my_assigned_zones_finds_assignments() {
        let seed = [6u8; 32];
        let validators = make_validators(10);
        let zones = my_assigned_zones(&validators[0].validator_id, &seed, &validators, 16, 5);
        assert!(
            !zones.is_empty(),
            "validator should be assigned to at least one zone"
        );
        for &zone_id in &zones {
            assert!(zone_id < 16);
        }
    }

    #[test]
    fn all_validators_assigned_somewhere() {
        let seed = [7u8; 32];
        let validators = make_validators(10);
        for v in &validators {
            let zones = my_assigned_zones(&v.validator_id, &seed, &validators, 32, 5);
            assert!(
                !zones.is_empty(),
                "validator {:?} should have at least one zone assignment",
                &v.validator_id[..4]
            );
        }
    }

    #[test]
    fn different_seeds_produce_different_assignments() {
        let validators = make_validators(20);
        let c1 = sample_committee(&[0u8; 32], 0, &validators, 5);
        let c2 = sample_committee(&[1u8; 32], 0, &validators, 5);
        let ids1: Vec<_> = c1.iter().map(|v| v.validator_id).collect();
        let ids2: Vec<_> = c2.iter().map(|v| v.validator_id).collect();
        assert_ne!(ids1, ids2);
    }
}
