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

    let key_file = config.key_file.clone();
    let key = tokio::task::spawn_blocking(move || load_or_generate_keypair(&key_file))
        .await
        .context("keypair task panicked")??;
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
            // INVARIANT: GRID_KAD_PROTOCOL is a well-formed static protocol string.
            let mut kad_config = kad::Config::new(
                StreamProtocol::try_from_owned(GRID_KAD_PROTOCOL.to_string())
                    .expect("valid protocol name"),
            );
            kad_config.set_query_timeout(Duration::from_secs(60));
            let store = kad::store::MemoryStore::new(peer_id);
            let mut kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);
            kademlia.set_mode(Some(kad::Mode::Server));

            let default_relay = relay::Config::default();
            let relay_config = relay::Config {
                reservation_duration: Duration::from_secs(60 * 60),
                max_circuit_duration: Duration::from_secs(24 * 60 * 60),
                max_circuit_bytes: 0,
                max_reservations: config
                    .max_reservations
                    .unwrap_or(default_relay.max_reservations),
                max_circuits: config.max_circuits.unwrap_or(512),
                max_circuits_per_peer: 16,
                ..default_relay
            };

            info!(
                max_reservations = relay_config.max_reservations,
                max_circuits = relay_config.max_circuits,
                max_circuits_per_peer = relay_config.max_circuits_per_peer,
                max_circuit_duration = ?relay_config.max_circuit_duration,
                max_circuit_bytes = relay_config.max_circuit_bytes,
                reservation_duration = ?relay_config.reservation_duration,
                "relay config"
            );

            RelayBehaviour {
                relay: relay::Behaviour::new(peer_id, relay_config),
                identify: identify::Behaviour::new(
                    identify::Config::new(GRID_IDENTIFY_PROTOCOL.to_string(), key.public())
                        .with_push_listen_addr_updates(true),
                ),
                kademlia,
                ping: ping::Behaviour::new(ping::Config::new()),
            }
        })
        .context("failed to build relay behaviour")?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(7200)))
        .build();

    let local_peer_id = *swarm.local_peer_id();
    info!(%local_peer_id, "relay peer ID");

    swarm
        .listen_on(config.listen.clone())
        .context("failed to start listener")?;

    let mut relay_external_addrs: Vec<Multiaddr> = Vec::new();
    let mut connected_peer_ids: HashSet<PeerId> = HashSet::new();
    let mut reserved_peer_ids: HashSet<PeerId> = HashSet::new();

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
                let normalized = normalize_multiaddr(&addr);
                if is_globally_routable(&normalized) {
                    swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, normalized);
                }
                connected_peer_ids.insert(peer_id);
                info!(
                    connected_count = connected_peer_ids.len(),
                    external_addrs = relay_external_addrs.len(),
                    "peer roster updated (circuit addrs deferred until reservation)"
                );
            }
            Some(SwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
                ..
            }) => {
                if num_established == 0 {
                    connected_peer_ids.remove(&peer_id);
                    let had_reservation = reserved_peer_ids.remove(&peer_id);
                    swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                    info!(
                        %peer_id,
                        had_reservation,
                        connected_count = connected_peer_ids.len(),
                        reserved_count = reserved_peer_ids.len(),
                        "peer fully disconnected, removed from kademlia"
                    );
                } else {
                    debug!(
                        %peer_id,
                        remaining = num_established,
                        "connection closed (peer still has other connections)"
                    );
                }
            }
            Some(SwarmEvent::OutgoingConnectionError { peer_id, error, .. }) => {
                warn!(
                    peer_id = ?peer_id,
                    error = %error,
                    "outgoing connection failed"
                );
                if let Some(failed_peer) = peer_id {
                    if !connected_peer_ids.contains(&failed_peer) {
                        swarm.behaviour_mut().kademlia.remove_peer(&failed_peer);
                        debug!(%failed_peer, "removed unreachable peer from kademlia");
                    }
                }
            }
            Some(SwarmEvent::Behaviour(event)) => match event {
                RelayBehaviourEvent::Identify(identify::Event::Received {
                    peer_id, info, ..
                }) => {
                    info!(
                        %peer_id,
                        listen_addrs = ?info.listen_addrs,
                        observed = %info.observed_addr,
                        protocols = ?info.protocols,
                        "identify received"
                    );
                    ingest_identify_update(&mut swarm, local_peer_id, &peer_id, &info);
                    ingest_circuit_addrs(
                        &mut swarm,
                        &peer_id,
                        &info.observed_addr,
                        local_peer_id,
                        &mut relay_external_addrs,
                        &reserved_peer_ids,
                    );
                }
                RelayBehaviourEvent::Identify(identify::Event::Pushed {
                    peer_id, info, ..
                }) => {
                    debug!(
                        %peer_id,
                        listen_addrs = ?info.listen_addrs,
                        observed = %info.observed_addr,
                        "identify pushed"
                    );
                    ingest_identify_update(&mut swarm, local_peer_id, &peer_id, &info);
                    ingest_circuit_addrs(
                        &mut swarm,
                        &peer_id,
                        &info.observed_addr,
                        local_peer_id,
                        &mut relay_external_addrs,
                        &reserved_peer_ids,
                    );
                }
                RelayBehaviourEvent::Identify(ref ev) => {
                    debug!(?ev, "identify event");
                }
                RelayBehaviourEvent::Relay(relay::Event::ReservationReqAccepted {
                    src_peer_id,
                    ..
                }) => {
                    reserved_peer_ids.insert(src_peer_id);
                    for ext_addr in &relay_external_addrs {
                        let circuit = strip_p2p_suffix(ext_addr)
                            .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
                            .with(libp2p::multiaddr::Protocol::P2pCircuit)
                            .with(libp2p::multiaddr::Protocol::P2p(src_peer_id));
                        debug!(
                            %src_peer_id,
                            %circuit,
                            "registered circuit addr in kademlia (reservation accepted)"
                        );
                        swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&src_peer_id, circuit);
                    }
                    info!(
                        %src_peer_id,
                        reserved_count = reserved_peer_ids.len(),
                        "relay reservation accepted, circuit addrs registered"
                    );
                }
                RelayBehaviourEvent::Relay(relay::Event::ReservationReqDenied {
                    src_peer_id,
                    ..
                }) => {
                    reserved_peer_ids.remove(&src_peer_id);
                    swarm.behaviour_mut().kademlia.remove_peer(&src_peer_id);
                    warn!(%src_peer_id, "relay reservation DENIED, removed from kademlia");
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
                    dst_peer_id,
                    ..
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
            },
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

const MAX_CONFIG_VALUE: usize = 100_000;

fn parse_usize(cli: Option<usize>, env: Option<&String>, name: &str) -> Result<Option<usize>> {
    let value = if cli.is_some() {
        cli
    } else {
        match env {
            Some(raw) => {
                let parsed = raw.parse::<usize>().map_err(|e| {
                    anyhow::anyhow!("{name}: invalid unsigned integer '{raw}': {e}")
                })?;
                Some(parsed)
            }
            None => None,
        }
    };
    if let Some(v) = value {
        if v > MAX_CONFIG_VALUE {
            anyhow::bail!("{name}: value {v} exceeds maximum {MAX_CONFIG_VALUE}");
        }
    }
    Ok(value)
}

fn current_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn strip_p2p_suffix(addr: &Multiaddr) -> Multiaddr {
    addr.iter()
        .filter(|p| !matches!(p, libp2p::multiaddr::Protocol::P2p(_)))
        .collect()
}

/// Normalize a multiaddr for Kademlia storage.
///
/// Direct addresses: strip all `/p2p/` segments (Kademlia stores peer ID
/// separately). Circuit addresses: rebuild canonical
/// `<transport>/p2p/<relay>/p2p-circuit/p2p/<dest>` form.
fn normalize_multiaddr(addr: &Multiaddr) -> Multiaddr {
    use libp2p::multiaddr::Protocol;

    let has_circuit = addr.iter().any(|p| matches!(p, Protocol::P2pCircuit));

    if !has_circuit {
        return strip_p2p_suffix(addr);
    }

    let mut transport = Multiaddr::empty();
    let mut relay_peer = None;
    let mut dest_peer = None;
    let mut past_circuit = false;

    for proto in addr.iter() {
        if matches!(&proto, Protocol::P2pCircuit) {
            past_circuit = true;
            continue;
        }
        if let Protocol::P2p(ref peer) = proto {
            if past_circuit {
                if dest_peer.is_none() {
                    dest_peer = Some(*peer);
                }
            } else {
                relay_peer = Some(*peer);
            }
            continue;
        }
        if !past_circuit {
            transport = transport.with(proto);
        }
    }

    let mut result = transport;
    if let Some(relay) = relay_peer {
        result = result.with(Protocol::P2p(relay));
    }
    result = result.with(Protocol::P2pCircuit);
    if let Some(dest) = dest_peer {
        result = result.with(Protocol::P2p(dest));
    }
    result
}

/// Returns true when every IP in the multiaddr is globally routable
/// (not loopback, private, link-local, or unspecified).
fn is_globally_routable(addr: &Multiaddr) -> bool {
    use libp2p::multiaddr::Protocol;

    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => {
                if ip.is_loopback()
                    || ip.is_private()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast()
                {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                if ip.is_loopback() || ip.is_unspecified() {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
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

/// Ingest the identify observed address as our own external address.
///
/// Peer-reported `listen_addrs` are intentionally NOT added to Kademlia here.
/// NAT-ed peers report dozens of stale ephemeral port mappings that pollute
/// the DHT and cause dial storms across the network. The verified connection
/// address is already added in the `ConnectionEstablished` handler.
fn ingest_identify_update(
    swarm: &mut libp2p::Swarm<RelayBehaviour>,
    local_peer_id: libp2p::PeerId,
    peer_id: &libp2p::PeerId,
    info: &identify::Info,
) {
    debug!(
        %peer_id,
        observed = %info.observed_addr,
        listen_addrs = info.listen_addrs.len(),
        "ingesting identify: adding external addr"
    );
    if is_globally_routable(&info.observed_addr) {
        swarm.add_external_address(info.observed_addr.clone());
    }

    // Also ingest relay-circuit listen addresses advertised by this peer.
    // We intentionally ignore direct listen_addrs here (they can be stale NAT
    // ephemeral ports), but valid `.../p2p/<this-relay>/p2p-circuit/p2p/<peer>`
    // addresses are high-signal and required for relay-routed discovery.
    for advertised in &info.listen_addrs {
        let normalized = normalize_multiaddr(advertised);
        if is_peer_circuit_addr_via_relay(&normalized, *peer_id, local_peer_id) {
            debug!(
                %peer_id,
                %normalized,
                "ingesting identify circuit listen addr into kademlia"
            );
            swarm
                .behaviour_mut()
                .kademlia
                .add_address(peer_id, normalized);
        }
    }
}

fn is_peer_circuit_addr_via_relay(addr: &Multiaddr, peer_id: PeerId, relay_id: PeerId) -> bool {
    use libp2p::multiaddr::Protocol;

    let mut saw_circuit = false;
    let mut relay_before_circuit = None;
    let mut dest_after_circuit = None;

    for proto in addr.iter() {
        match proto {
            Protocol::P2pCircuit => saw_circuit = true,
            Protocol::P2p(pid) if !saw_circuit => relay_before_circuit = Some(pid),
            Protocol::P2p(pid) => {
                if dest_after_circuit.is_none() {
                    dest_after_circuit = Some(pid);
                }
            }
            _ => {}
        }
    }

    saw_circuit
        && relay_before_circuit == Some(relay_id)
        && dest_after_circuit == Some(peer_id)
        && is_globally_routable(addr)
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
    reserved_peer_ids: &HashSet<PeerId>,
) {
    if !is_globally_routable(observed_addr) {
        debug!(
            %observed_addr,
            "ignoring non-routable observed address for circuit registration"
        );
        return;
    }

    let is_new_ext = !relay_external_addrs.contains(observed_addr);
    if is_new_ext {
        info!(
            %observed_addr,
            total_external = relay_external_addrs.len() + 1,
            "new external address learned"
        );
        relay_external_addrs.push(observed_addr.clone());
    }

    if reserved_peer_ids.contains(peer_id) {
        for ext_addr in relay_external_addrs.iter() {
            let circuit = strip_p2p_suffix(ext_addr)
                .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
                .with(libp2p::multiaddr::Protocol::P2pCircuit)
                .with(libp2p::multiaddr::Protocol::P2p(*peer_id));
            debug!(
                %peer_id,
                %circuit,
                "adding circuit addr to kademlia for reserved peer"
            );
            swarm.behaviour_mut().kademlia.add_address(peer_id, circuit);
        }
    }

    if is_new_ext {
        info!(
            %observed_addr,
            reserved_peers = reserved_peer_ids.len(),
            "backfilling circuit addrs for reserved peers under new external addr"
        );
        for &reserved_peer in reserved_peer_ids {
            let circuit = strip_p2p_suffix(observed_addr)
                .with(libp2p::multiaddr::Protocol::P2p(local_peer_id))
                .with(libp2p::multiaddr::Protocol::P2pCircuit)
                .with(libp2p::multiaddr::Protocol::P2p(reserved_peer));
            debug!(
                %reserved_peer,
                %circuit,
                "backfill: adding circuit addr to kademlia"
            );
            swarm
                .behaviour_mut()
                .kademlia
                .add_address(&reserved_peer, circuit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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

    // ── parse_usize: MAX_CONFIG_VALUE enforcement ──────────────────────

    #[test]
    fn parse_usize_at_max_is_ok() {
        let result = parse_usize(Some(MAX_CONFIG_VALUE), None, "test").unwrap();
        assert_eq!(result, Some(MAX_CONFIG_VALUE));
    }

    #[test]
    fn parse_usize_above_max_is_rejected() {
        let err = parse_usize(Some(MAX_CONFIG_VALUE + 1), None, "MY_VAR")
            .expect_err("should reject values above MAX_CONFIG_VALUE");
        let msg = err.to_string();
        assert!(msg.contains("MY_VAR"));
        assert!(msg.contains("exceeds maximum"));
    }

    #[test]
    fn parse_usize_env_above_max_is_rejected() {
        let raw = (MAX_CONFIG_VALUE + 42).to_string();
        let err =
            parse_usize(None, Some(&raw), "ENV_VAR").expect_err("env value above max should fail");
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn parse_usize_none_when_both_absent() {
        assert_eq!(parse_usize(None, None, "test").unwrap(), None);
    }

    #[test]
    fn parse_usize_cli_overrides_env() {
        let env_val = "50".to_string();
        let result = parse_usize(Some(7), Some(&env_val), "test").unwrap();
        assert_eq!(result, Some(7));
    }

    // ── normalize_multiaddr ────────────────────────────────────────────

    fn peer_id_a() -> PeerId {
        PeerId::from_str("12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN").unwrap()
    }

    fn peer_id_b() -> PeerId {
        PeerId::from_str("12D3KooWGzBLb7F5rUFtj6VqPxRQySmRsLeFmh9G7zhHWGmSqUir").unwrap()
    }

    #[test]
    fn normalize_direct_addr_strips_p2p() {
        let addr: Multiaddr = format!("/ip4/1.2.3.4/tcp/4001/p2p/{}", peer_id_a())
            .parse()
            .unwrap();
        let norm = normalize_multiaddr(&addr);
        assert_eq!(norm.to_string(), "/ip4/1.2.3.4/tcp/4001");
    }

    #[test]
    fn normalize_direct_addr_without_p2p_unchanged() {
        let addr: Multiaddr = "/ip4/10.0.0.1/tcp/5000".parse().unwrap();
        let norm = normalize_multiaddr(&addr);
        assert_eq!(norm, addr);
    }

    #[test]
    fn normalize_circuit_addr_canonical_form() {
        let raw = format!(
            "/ip4/1.2.3.4/tcp/4001/p2p/{}/p2p-circuit/p2p/{}",
            peer_id_a(),
            peer_id_b()
        );
        let addr: Multiaddr = raw.parse().unwrap();
        let norm = normalize_multiaddr(&addr);
        let expected = format!(
            "/ip4/1.2.3.4/tcp/4001/p2p/{}/p2p-circuit/p2p/{}",
            peer_id_a(),
            peer_id_b()
        );
        assert_eq!(norm.to_string(), expected);
    }

    #[test]
    fn normalize_circuit_without_dest_peer() {
        let raw = format!("/ip4/1.2.3.4/tcp/4001/p2p/{}/p2p-circuit", peer_id_a());
        let addr: Multiaddr = raw.parse().unwrap();
        let norm = normalize_multiaddr(&addr);
        let expected = format!("/ip4/1.2.3.4/tcp/4001/p2p/{}/p2p-circuit", peer_id_a());
        assert_eq!(norm.to_string(), expected);
    }

    #[test]
    fn normalize_circuit_without_relay_peer() {
        let raw = format!("/ip4/1.2.3.4/tcp/4001/p2p-circuit/p2p/{}", peer_id_b());
        let addr: Multiaddr = raw.parse().unwrap();
        let norm = normalize_multiaddr(&addr);
        let expected = format!("/ip4/1.2.3.4/tcp/4001/p2p-circuit/p2p/{}", peer_id_b());
        assert_eq!(norm.to_string(), expected);
    }

    // ── strip_p2p_suffix ───────────────────────────────────────────────

    #[test]
    fn strip_p2p_removes_peer_id() {
        let addr: Multiaddr = format!("/ip4/1.2.3.4/tcp/9999/p2p/{}", peer_id_a())
            .parse()
            .unwrap();
        assert_eq!(strip_p2p_suffix(&addr).to_string(), "/ip4/1.2.3.4/tcp/9999");
    }

    #[test]
    fn strip_p2p_noop_without_p2p() {
        let addr: Multiaddr = "/ip4/10.0.0.1/udp/3000".parse().unwrap();
        assert_eq!(strip_p2p_suffix(&addr), addr);
    }

    // ── is_globally_routable ───────────────────────────────────────────

    #[test]
    fn routable_public_ipv4() {
        let addr: Multiaddr = "/ip4/8.8.8.8/tcp/443".parse().unwrap();
        assert!(is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_loopback_ipv4() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_private_ipv4() {
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_link_local_ipv4() {
        let addr: Multiaddr = "/ip4/169.254.1.1/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_unspecified_ipv4() {
        let addr: Multiaddr = "/ip4/0.0.0.0/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_broadcast_ipv4() {
        let addr: Multiaddr = "/ip4/255.255.255.255/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn routable_public_ipv6() {
        let addr: Multiaddr = "/ip6/2607:f8b0:4004:800::200e/tcp/443".parse().unwrap();
        assert!(is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_loopback_ipv6() {
        let addr: Multiaddr = "/ip6/::1/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_unspecified_ipv6() {
        let addr: Multiaddr = "/ip6/::/tcp/4001".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    // ── is_peer_circuit_addr_via_relay ──────────────────────────────────

    #[test]
    fn circuit_addr_matches_relay_and_dest() {
        let relay = peer_id_a();
        let dest = peer_id_b();
        let addr: Multiaddr = format!(
            "/ip4/8.8.8.8/tcp/4001/p2p/{}/p2p-circuit/p2p/{}",
            relay, dest
        )
        .parse()
        .unwrap();
        assert!(is_peer_circuit_addr_via_relay(&addr, dest, relay));
    }

    #[test]
    fn circuit_addr_wrong_relay_rejected() {
        let relay = peer_id_a();
        let dest = peer_id_b();
        let addr: Multiaddr = format!(
            "/ip4/8.8.8.8/tcp/4001/p2p/{}/p2p-circuit/p2p/{}",
            dest, dest
        )
        .parse()
        .unwrap();
        assert!(!is_peer_circuit_addr_via_relay(&addr, dest, relay));
    }

    #[test]
    fn circuit_addr_non_routable_rejected() {
        let relay = peer_id_a();
        let dest = peer_id_b();
        let addr: Multiaddr = format!(
            "/ip4/127.0.0.1/tcp/4001/p2p/{}/p2p-circuit/p2p/{}",
            relay, dest
        )
        .parse()
        .unwrap();
        assert!(!is_peer_circuit_addr_via_relay(&addr, dest, relay));
    }

    #[test]
    fn non_circuit_addr_rejected() {
        let relay = peer_id_a();
        let dest = peer_id_b();
        let addr: Multiaddr = format!("/ip4/8.8.8.8/tcp/4001/p2p/{}", dest)
            .parse()
            .unwrap();
        assert!(!is_peer_circuit_addr_via_relay(&addr, dest, relay));
    }

    // ── load_or_generate_keypair ───────────────────────────────────────

    #[test]
    fn keypair_generate_and_reload_stable_peer_id() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join("test-keypair");

        let kp1 = load_or_generate_keypair(&key_path).expect("should generate keypair");
        assert!(key_path.exists(), "keypair file should be written");

        let kp2 = load_or_generate_keypair(&key_path).expect("should reload keypair");
        assert_eq!(
            kp1.public().to_peer_id(),
            kp2.public().to_peer_id(),
            "peer ID should be stable across reloads"
        );
    }

    #[test]
    fn keypair_creates_parent_directories() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join("deep").join("nested").join("keypair");

        let kp = load_or_generate_keypair(&key_path).expect("should create dirs and generate");
        assert!(key_path.exists());
        assert!(!kp.public().to_peer_id().to_string().is_empty());
    }
}
