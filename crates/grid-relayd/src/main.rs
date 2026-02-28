#![forbid(unsafe_code)]

use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::Parser;
use futures::StreamExt;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{identify, ping, relay, Multiaddr};
use tracing::{debug, info, warn};

const ENV_LISTEN: &str = "GRID_RELAY_LISTEN";
const ENV_LOG: &str = "GRID_RELAY_LOG";
const ENV_MAX_RESERVATIONS: &str = "GRID_RELAY_MAX_RESERVATIONS";
const ENV_MAX_CIRCUITS: &str = "GRID_RELAY_MAX_CIRCUITS";
const DEFAULT_LISTEN: &str = "/ip4/0.0.0.0/tcp/3690";

#[derive(Parser, Debug, Clone)]
#[command(name = "grid-relayd", version, about)]
struct Cli {
    /// Multiaddr to listen on.
    #[arg(long)]
    listen: Option<String>,

    /// Log filter override (falls back to GRID_RELAY_LOG, then RUST_LOG).
    #[arg(long)]
    log: Option<String>,

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
    max_reservations: Option<usize>,
    max_circuits: Option<usize>,
}

#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    identify: identify::Behaviour,
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

    let key = libp2p::identity::Keypair::generate_ed25519();
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .context("failed to build TCP transport")?
        .with_quic()
        .with_behaviour(|key| RelayBehaviour {
            relay: relay::Behaviour::new(key.public().to_peer_id(), Default::default()),
            identify: identify::Behaviour::new(identify::Config::new(
                "/grid/relayd/1.0.0".to_string(),
                key.public(),
            )),
            ping: ping::Behaviour::new(ping::Config::new()),
        })
        .context("failed to build relay behaviour")?
        .build();

    swarm
        .listen_on(config.listen.clone())
        .context("failed to start listener")?;

    loop {
        match swarm.next().await {
            Some(SwarmEvent::NewListenAddr { address, .. }) => {
                info!(%address, "relay listening");
            }
            Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) => {
                info!(%peer_id, "peer connected");
            }
            Some(SwarmEvent::ConnectionClosed { peer_id, .. }) => {
                info!(%peer_id, "peer disconnected");
            }
            Some(SwarmEvent::Behaviour(event)) => {
                if let RelayBehaviourEvent::Identify(identify::Event::Received { info, .. }) =
                    &event
                {
                    swarm.add_external_address(info.observed_addr.clone());
                }
                debug!(?event, "relay behaviour event");
            }
            Some(_) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_only_config_path() {
        let cli = Cli {
            listen: None,
            log: None,
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
