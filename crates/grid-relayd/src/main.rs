#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use futures::StreamExt;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{identify, kad, ping, relay, Multiaddr, PeerId, StreamProtocol};
use tracing::{debug, info, warn};

const ENV_LISTEN: &str = "GRID_RELAY_LISTEN";
const ENV_LOG: &str = "GRID_RELAY_LOG";
const ENV_KEY_FILE: &str = "GRID_RELAY_KEY_FILE";
const ENV_MAX_RESERVATIONS: &str = "GRID_RELAY_MAX_RESERVATIONS";
const ENV_MAX_CIRCUITS: &str = "GRID_RELAY_MAX_CIRCUITS";
const DEFAULT_LISTEN: &str = "/ip4/0.0.0.0/tcp/3690";
const DEFAULT_KEY_FILE: &str = "/var/lib/grid-relayd/keypair";

#[derive(Parser, Debug, Clone)]
#[command(name = "grid-relayd", version, about)]
struct Cli {
    /// Multiaddr to listen on.
    #[arg(long)]
    listen: Option<String>,

    /// Log filter override (falls back to GRID_RELAY_LOG, then RUST_LOG).
    #[arg(long)]
    log: Option<String>,

    /// Path to persist the relay keypair (ensures stable peer ID across restarts).
    #[arg(long)]
    key_file: Option<String>,

    /// Max relay reservations (CLI overrides GRID_RELAY_MAX_RESERVATIONS).
    #[arg(long)]
    max_reservations: Option<usize>,

    /// Max relay circuits (CLI overrides GRID_RELAY_MAX_CIRCUITS).
    #[arg(long)]
    max_circuits: Option<usize>,
}

#[derive(Debug, Clone)]
struct RelaydConfig {
    listen: Multiaddr,
    log_filter: Option<String>,
    key_file: PathBuf,
    max_reservations: Option<usize>,
    max_circuits: Option<usize>,
}

const GRID_KAD_PROTOCOL: &str = "/grid/kad/1.0.0";
const GRID_IDENTIFY_PROTOCOL: &str = "/grid/id/1.0.0";

#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    identify: identify::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    ping: ping::Behaviour,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let env_map = current_env();
    let config = parse_config(&cli, &env_map)?;

    let log_filter = config
        .log_filter
        .clone()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_string());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&log_filter)
                .context("invalid effective log filter")?,
        )
        .init();

    info!(
        listen = %config.listen,
        relay_max_reservations = ?config.max_reservations,
        relay_max_circuits = ?config.max_circuits,
        log_filter = %log_filter,
        "starting grid-relayd"
    );

    if config.max_reservations.is_some() || config.max_circuits.is_some() {
        warn!("relay limits are parsed but not currently applied to libp2p relay config");
    }

    let key = load_or_generate_keypair(&config.key_file)?;
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .context("failed to build TCP transport")?
        .with_quic()
        .with_behaviour(|key| {
            let peer_id = key.public().to_peer_id();
            let mut kad_config = kad::Config::new(
                StreamProtocol::try_from_owned(GRID_KAD_PROTOCOL.to_string())
                    .expect("valid protocol name"),
            );
            kad_config.set_query_timeout(Duration::from_secs(60));
            let store = kad::store::MemoryStore::new(peer_id);
            let mut kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);
            kademlia.set_mode(Some(kad::Mode::Server));

            RelayBehaviour {
                relay: relay::Behaviour::new(peer_id, Default::default()),
                identify: identify::Behaviour::new(
                    identify::Config::new(
                        GRID_IDENTIFY_PROTOCOL.to_string(),
                        key.public(),
                    )
                    .with_push_listen_addr_updates(true),
                ),
                kademlia,
                ping: ping::Behaviour::new(ping::Config::new()),
            }
        })
        .context("failed to build relay behaviour")?
        .build();

    let local_peer_id = *swarm.local_peer_id();
    info!(%local_peer_id, "relay peer ID");

    swarm
        .listen_on(config.listen.clone())
        .context("failed to start listener")?;

    let mut relay_external_addrs: Vec<Multiaddr> = Vec::new();
    let mut connected_peer_ids: HashSet<PeerId> = HashSet::new();

    loop {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { address, .. }) => {
                let full = address
                    .clone()
                    .with(libp2p::multiaddr::Protocol::P2p(local_peer_id));
                info!(%address, %full, "relay listening");
            }
            Some(SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            }) => {
                let addr = endpoint.get_remote_address().clone();
                info!(
                    %peer_id,
                    remote_addr = %addr,
                    direction = ?endpoint,
                    "peer connected"
                );
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr);
                connected_peer_ids.insert(peer_id);
                for ext_addr in &relay_external_addrs {
                    let circuit = strip_p2p_suffix(ext_addr)
                        .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
                        .with(libp2p::multiaddr::Protocol::P2pCircuit)
                        .with(libp2p::multiaddr::Protocol::P2p(peer_id));
                    debug!(
                        %peer_id,
                        %circuit,
                        "registered circuit addr in kademlia"
                    );
                    swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, circuit);
                }
                info!(
                    connected_count = connected_peer_ids.len(),
                    external_addrs = relay_external_addrs.len(),
                    "peer roster updated"
                );
            }
            Some(SwarmEvent::ConnectionClosed { peer_id, .. }) => {
                connected_peer_ids.remove(&peer_id);
                info!(
                    %peer_id,
                    connected_count = connected_peer_ids.len(),
                    "peer disconnected"
                );
            }
            Some(SwarmEvent::OutgoingConnectionError {
                peer_id, error, ..
            }) => {
                warn!(
                    peer_id = ?peer_id,
                    error = %error,
                    "outgoing connection failed"
                );
            }
            Some(SwarmEvent::Behaviour(event)) => {
                match event {
                    RelayBehaviourEvent::Identify(identify::Event::Received {
                        peer_id,
                        info,
                        ..
                    }) => {
                        info!(
                            %peer_id,
                            listen_addrs = ?info.listen_addrs,
                            observed = %info.observed_addr,
                            protocols = ?info.protocols,
                            "identify received"
                        );
                        ingest_identify_update(&mut swarm, &peer_id, &info);
                        ingest_circuit_addrs(
                            &mut swarm,
                            &peer_id,
                            &info.observed_addr,
                            local_peer_id,
                            &mut relay_external_addrs,
                            &connected_peer_ids,
                        );
                    }
                    RelayBehaviourEvent::Identify(identify::Event::Pushed {
                        peer_id,
                        info,
                        ..
                    }) => {
                        debug!(
                            %peer_id,
                            listen_addrs = ?info.listen_addrs,
                            observed = %info.observed_addr,
                            "identify pushed"
                        );
                        ingest_identify_update(&mut swarm, &peer_id, &info);
                        ingest_circuit_addrs(
                            &mut swarm,
                            &peer_id,
                            &info.observed_addr,
                            local_peer_id,
                            &mut relay_external_addrs,
                            &connected_peer_ids,
                        );
                    }
                    RelayBehaviourEvent::Identify(ref ev) => {
                        debug!(?ev, "identify event");
                    }
                    RelayBehaviourEvent::Relay(relay::Event::ReservationReqAccepted {
                        src_peer_id,
                        ..
                    }) => {
                        info!(%src_peer_id, "relay reservation accepted");
                    }
                    RelayBehaviourEvent::Relay(relay::Event::ReservationReqDenied {
                        src_peer_id, ..
                    }) => {
                        warn!(%src_peer_id, "relay reservation DENIED");
                    }
                    RelayBehaviourEvent::Relay(relay::Event::CircuitReqAccepted {
                        src_peer_id,
                        dst_peer_id,
                        ..
                    }) => {
                        info!(
                            %src_peer_id,
                            %dst_peer_id,
                            "relay circuit opened"
                        );
                    }
                    RelayBehaviourEvent::Relay(relay::Event::CircuitReqDenied {
                        src_peer_id,
                        dst_peer_id, ..
                    }) => {
                        warn!(
                            %src_peer_id,
                            %dst_peer_id,
                            "relay circuit DENIED"
                        );
                    }
                    RelayBehaviourEvent::Relay(ref ev) => {
                        debug!(?ev, "relay event");
                    }
                    RelayBehaviourEvent::Kademlia(kad::Event::RoutingUpdated {
                        peer,
                        ref addresses,
                        ..
                    }) => {
                        let addrs: Vec<_> = addresses.iter().collect();
                        debug!(
                            %peer,
                            addresses = ?addrs,
                            "kademlia routing updated"
                        );
                    }
                    RelayBehaviourEvent::Kademlia(kad::Event::OutboundQueryProgressed {
                        ref result,
                        ..
                    }) => {
                        debug!(?result, "kademlia query progressed");
                    }
                    RelayBehaviourEvent::Kademlia(ref ev) => {
                        debug!(?ev, "kademlia event");
                    }
                    RelayBehaviourEvent::Ping(ref ev) => {
                        debug!(?ev, "ping event");
                    }
                }
            }
            Some(SwarmEvent::IncomingConnectionError {
                send_back_addr,
                error,
                ..
            }) => {
                warn!(
                    %send_back_addr,
                    error = %error,
                    "incoming connection error"
                );
            }
            Some(other) => {
                debug!(?other, "swarm event");
            }
            None => break,
        }
    }

    Ok(())
}

fn parse_config(cli: &Cli, env: &HashMap<String, String>) -> Result<RelaydConfig> {
    let listen_raw = cli
        .listen
        .clone()
        .or_else(|| env.get(ENV_LISTEN).cloned())
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let listen: Multiaddr = listen_raw
        .parse()
        .map_err(|e| anyhow::anyhow!("{ENV_LISTEN}: invalid multiaddr '{listen_raw}': {e}"))?;

    let log_filter = cli.log.clone().or_else(|| env.get(ENV_LOG).cloned());
    let key_file = PathBuf::from(
        cli.key_file
            .clone()
            .or_else(|| env.get(ENV_KEY_FILE).cloned())
            .unwrap_or_else(|| DEFAULT_KEY_FILE.to_string()),
    );
    let max_reservations = parse_usize(
        cli.max_reservations,
        env.get(ENV_MAX_RESERVATIONS),
        ENV_MAX_RESERVATIONS,
    )?;
    let max_circuits = parse_usize(
        cli.max_circuits,
        env.get(ENV_MAX_CIRCUITS),
        ENV_MAX_CIRCUITS,
    )?;

    Ok(RelaydConfig {
        listen,
        log_filter,
        key_file,
        max_reservations,
        max_circuits,
    })
}

fn parse_usize(cli: Option<usize>, env: Option<&String>, name: &str) -> Result<Option<usize>> {
    if cli.is_some() {
        return Ok(cli);
    }
    match env {
        Some(raw) => {
            let parsed = raw
                .parse::<usize>()
                .map_err(|e| anyhow::anyhow!("{name}: invalid unsigned integer '{raw}': {e}"))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn current_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn strip_p2p_suffix(addr: &Multiaddr) -> Multiaddr {
    addr.iter()
        .filter(|p| !matches!(p, libp2p::multiaddr::Protocol::P2p(_)))
        .collect()
}

fn load_or_generate_keypair(path: &std::path::Path) -> Result<libp2p::identity::Keypair> {
    if path.exists() {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read keypair from {}", path.display()))?;
        let kp = libp2p::identity::Keypair::from_protobuf_encoding(&bytes)
            .with_context(|| format!("failed to decode keypair from {}", path.display()))?;
        info!(path = %path.display(), peer_id = %kp.public().to_peer_id(), "loaded existing keypair");
        return Ok(kp);
    }

    let kp = libp2p::identity::Keypair::generate_ed25519();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create keypair directory {}", parent.display()))?;
    }
    let bytes = kp
        .to_protobuf_encoding()
        .context("failed to encode keypair")?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write keypair to {}", path.display()))?;
    info!(path = %path.display(), peer_id = %kp.public().to_peer_id(), "generated and saved new keypair");
    Ok(kp)
}

fn ingest_identify_update(
    swarm: &mut libp2p::Swarm<RelayBehaviour>,
    peer_id: &libp2p::PeerId,
    info: &identify::Info,
) {
    debug!(
        %peer_id,
        observed = %info.observed_addr,
        listen_addrs = info.listen_addrs.len(),
        "ingesting identify: adding external addr + kademlia entries"
    );
    swarm.add_external_address(info.observed_addr.clone());
    for addr in &info.listen_addrs {
        debug!(%peer_id, %addr, "adding peer listen addr to kademlia");
        swarm
            .behaviour_mut()
            .kademlia
            .add_address(peer_id, addr.clone());
    }
}

/// Register relay circuit addresses for the identified peer in the Kademlia
/// routing table so other nodes can discover a relay-routed path to it.
///
/// When a new external address is learned, retroactively registers circuit
/// addresses for all currently connected peers under that address.
fn ingest_circuit_addrs(
    swarm: &mut libp2p::Swarm<RelayBehaviour>,
    peer_id: &PeerId,
    observed_addr: &Multiaddr,
    local_peer_id: PeerId,
    relay_external_addrs: &mut Vec<Multiaddr>,
    connected_peer_ids: &HashSet<PeerId>,
) {
    let is_new_ext = !relay_external_addrs.contains(observed_addr);
    if is_new_ext {
        info!(
            %observed_addr,
            total_external = relay_external_addrs.len() + 1,
            "new external address learned"
        );
        relay_external_addrs.push(observed_addr.clone());
    }

    for ext_addr in relay_external_addrs.iter() {
        let circuit = strip_p2p_suffix(ext_addr)
            .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(*peer_id));
        debug!(
            %peer_id,
            %circuit,
            "adding circuit addr to kademlia for peer"
        );
        swarm
            .behaviour_mut()
            .kademlia
            .add_address(peer_id, circuit);
    }

    if is_new_ext {
        info!(
            %observed_addr,
            connected_peers = connected_peer_ids.len(),
            "backfilling circuit addrs for existing peers under new external addr"
        );
        for &existing_peer in connected_peer_ids {
            let circuit = strip_p2p_suffix(observed_addr)
                .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
                .with(libp2p::multiaddr::Protocol::P2pCircuit)
                .with(libp2p::multiaddr::Protocol::P2p(existing_peer));
            debug!(
                %existing_peer,
                %circuit,
                "backfill: adding circuit addr to kademlia"
            );
            swarm
                .behaviour_mut()
                .kademlia
                .add_address(&existing_peer, circuit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_only_config_path() {
        let cli = Cli {
            listen: None,
            log: None,
            key_file: None,
            max_reservations: None,
            max_circuits: None,
        };
        let mut env = HashMap::new();
        env.insert(
            ENV_LISTEN.to_string(),
            "/ip4/127.0.0.1/tcp/4900".to_string(),
        );
        env.insert(ENV_LOG.to_string(), "debug".to_string());
        env.insert(ENV_MAX_RESERVATIONS.to_string(), "64".to_string());
        env.insert(ENV_MAX_CIRCUITS.to_string(), "128".to_string());

        let cfg = parse_config(&cli, &env).expect("env config should parse");
        assert_eq!(cfg.listen.to_string(), "/ip4/127.0.0.1/tcp/4900");
        assert_eq!(cfg.log_filter.as_deref(), Some("debug"));
        assert_eq!(cfg.max_reservations, Some(64));
        assert_eq!(cfg.max_circuits, Some(128));
    }

    #[test]
    fn cli_overrides_env() {
        let cli = Cli {
            listen: Some("/ip4/0.0.0.0/tcp/4910".to_string()),
            log: Some("trace".to_string()),
            key_file: None,
            max_reservations: Some(5),
            max_circuits: Some(9),
        };
        let mut env = HashMap::new();
        env.insert(
            ENV_LISTEN.to_string(),
            "/ip4/127.0.0.1/tcp/4900".to_string(),
        );
        env.insert(ENV_LOG.to_string(), "debug".to_string());
        env.insert(ENV_MAX_RESERVATIONS.to_string(), "64".to_string());
        env.insert(ENV_MAX_CIRCUITS.to_string(), "128".to_string());

        let cfg = parse_config(&cli, &env).expect("cli should override env");
        assert_eq!(cfg.listen.to_string(), "/ip4/0.0.0.0/tcp/4910");
        assert_eq!(cfg.log_filter.as_deref(), Some("trace"));
        assert_eq!(cfg.max_reservations, Some(5));
        assert_eq!(cfg.max_circuits, Some(9));
    }

    #[test]
    fn invalid_env_values_fail_fast() {
        let cli = Cli {
            listen: None,
            log: None,
            key_file: None,
            max_reservations: None,
            max_circuits: None,
        };
        let mut env = HashMap::new();
        env.insert(ENV_MAX_CIRCUITS.to_string(), "abc".to_string());

        let err = parse_config(&cli, &env).expect_err("invalid env should fail");
        let msg = err.to_string();
        assert!(msg.contains(ENV_MAX_CIRCUITS));
        assert!(msg.contains("invalid unsigned integer"));
    }
}
