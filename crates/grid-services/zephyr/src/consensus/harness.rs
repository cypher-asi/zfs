use grid_programs_zephyr::ValidatorInfo;

use super::engine::ConsensusAction;
use super::pending_certs::PendingCertBuffer;
use super::ZoneConsensus;
use crate::config::ZephyrConfig;

pub struct TestNode {
    pub engine: ZoneConsensus,
    pub pending: PendingCertBuffer,
    pub outbox: Vec<ConsensusAction>,
    pub connected: bool,
}

pub struct TestNetwork {
    pub nodes: Vec<TestNode>,
    round_timeout_ticks: u32,
}

fn identity_sign(data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

impl TestNetwork {
    pub fn new(n: usize, zone_id: u32) -> Self {
        let committee: Vec<ValidatorInfo> = (0..n)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = i as u8;
                ValidatorInfo {
                    validator_id: id,
                    pubkey: id,
                    p2p_endpoint: format!("/ip4/127.0.0.1/tcp/{}", 4000 + i),
                }
            })
            .collect();

        let config = ZephyrConfig {
            total_zones: 4,
            committee_size: n,
            quorum_threshold: n / 2 + 1,
            max_block_size: 64,
            round_timeout_ticks: 3,
            ..ZephyrConfig::default()
        };

        let nodes = committee
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let engine = ZoneConsensus::new(
                    zone_id,
                    0,
                    committee.clone(),
                    v.validator_id,
                    [0u8; 32],
                    config.clone(),
                    i,
                );
                TestNode {
                    engine,
                    pending: PendingCertBuffer::new(64),
                    outbox: Vec::new(),
                    connected: true,
                }
            })
            .collect();

        Self {
            nodes,
            round_timeout_ticks: config.round_timeout_ticks,
        }
    }

    pub fn tick_all(&mut self) {
        let timeout_ticks = self.round_timeout_ticks;
        for node in &mut self.nodes {
            node.engine.tick();
            if node.engine.is_round_timed_out(timeout_ticks) {
                node.engine.timeout_round();
            }
            if node.engine.is_leader() && !node.engine.has_pending_proposal() {
                if let Some(action) = node.engine.propose(vec![], identity_sign) {
                    node.outbox.push(action);
                }
            }
        }
    }

    pub fn deliver_proposals(&mut self) {
        let proposals: Vec<_> = self
            .nodes
            .iter_mut()
            .filter(|n| n.connected)
            .flat_map(|n| {
                n.outbox
                    .drain(..)
                    .filter_map(|a| match a {
                        ConsensusAction::BroadcastProposal(b) => Some(b),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        for proposal in &proposals {
            for node in &mut self.nodes {
                if !node.connected {
                    continue;
                }
                if let Some(vote) =
                    node.engine
                        .vote_on_proposal(proposal, identity_sign)
                {
                    node.outbox.push(vote);
                }
            }
        }
    }

    pub fn deliver_votes(&mut self) {
        let votes: Vec<_> = self
            .nodes
            .iter_mut()
            .filter(|n| n.connected)
            .flat_map(|n| {
                n.outbox
                    .drain(..)
                    .filter_map(|a| match a {
                        ConsensusAction::BroadcastVote(v) => Some(v),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        for vote in &votes {
            for node in &mut self.nodes {
                if !node.connected {
                    continue;
                }
                if let Some(cert_action) = node.engine.receive_vote(vote.clone()) {
                    node.outbox.push(cert_action);
                }
            }
        }
    }

    pub fn deliver_certs(&mut self) {
        let certs: Vec<_> = self
            .nodes
            .iter_mut()
            .filter(|n| n.connected)
            .flat_map(|n| {
                n.outbox
                    .drain(..)
                    .filter_map(|a| match a {
                        ConsensusAction::BroadcastCertificate(c) => Some(c),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        for cert in &certs {
            for node in &mut self.nodes {
                if !node.connected {
                    continue;
                }
                if !node.engine.apply_certificate(cert) {
                    node.pending.push(cert.clone());
                }
                let drained = node.pending.drain_applicable(&mut node.engine);
                for dc in &drained {
                    node.engine.apply_certificate(dc);
                }
            }
        }
    }

    pub fn run_round(&mut self) {
        self.tick_all();
        self.deliver_proposals();
        self.deliver_votes();
        self.deliver_certs();
    }

    pub fn run_rounds(&mut self, n: usize) {
        for _ in 0..n {
            self.run_round();
        }
    }

    pub fn partition(&mut self, isolated: &[usize]) {
        for &idx in isolated {
            if idx < self.nodes.len() {
                self.nodes[idx].connected = false;
            }
        }
    }

    pub fn heal(&mut self) {
        for node in &mut self.nodes {
            node.connected = true;
        }
    }

    pub fn assert_converged(&self) {
        let first = &self.nodes[0].engine;
        for (i, node) in self.nodes.iter().enumerate().skip(1) {
            assert_eq!(
                node.engine.parent_hash(),
                first.parent_hash(),
                "node {i} parent_hash diverged from node 0"
            );
            assert_eq!(
                node.engine.height(),
                first.height(),
                "node {i} height diverged from node 0"
            );
        }
    }

    pub fn heights(&self) -> Vec<u64> {
        self.nodes.iter().map(|n| n.engine.height()).collect()
    }
}
