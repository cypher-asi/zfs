#![forbid(unsafe_code)]

mod app;
mod ui;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use grid_core::ProgramId;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;
use zode::{DefaultProgramsConfig, RpcConfig, Zode, ZodeConfig};

use crate::app::{App, Screen};

/// ZODE CLI — console-only ZODE with TUI.
#[derive(Parser, Debug)]
#[command(name = "zode-cli", version, about)]
struct Cli {
    /// Path to the ZODE configuration file (TOML).
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Storage data directory.
    #[arg(long, default_value = ".zode/data")]
    data_dir: PathBuf,

    /// libp2p listen address (multiaddr).
    #[arg(long, default_value = "/ip4/0.0.0.0/udp/3690/quic-v1")]
    listen: String,

    /// Bootstrap peer multiaddrs (repeatable).
    #[arg(long)]
    bootstrap: Vec<String>,

    /// Disable the default ZID (Zero Identity) program.
    #[arg(long)]
    no_zid: bool,

    /// Disable the default Interlink program.
    #[arg(long)]
    no_interlink: bool,

    /// Additional program IDs (hex) to subscribe to (repeatable).
    #[arg(long)]
    topic: Vec<String>,

    /// Disable Kademlia DHT automatic peer discovery.
    #[arg(long)]
    no_kademlia: bool,

    /// Enable relay transport support.
    #[arg(long)]
    enable_relay: bool,

    /// Relay peer multiaddrs (repeatable).
    #[arg(long)]
    relay: Vec<String>,

    /// Kademlia mode: "server" (default, for ZODEs) or "client" (for SDK clients).
    #[arg(long, default_value = "server")]
    kademlia_mode: String,

    /// Interval in seconds between DHT random walk queries.
    #[arg(long, default_value = "30")]
    random_walk_interval: u64,

    /// Enable the JSON-RPC HTTP server.
    #[arg(long)]
    rpc: bool,

    /// RPC server bind address (requires --rpc).
    #[arg(long, default_value = "127.0.0.1:4690")]
    rpc_bind: String,

    /// API key for RPC authentication (requires --rpc). Omit for open access.
    #[arg(long)]
    rpc_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = build_config(&cli)?;
    let zode = Zode::start(config).await.context("failed to start ZODE")?;

    let result = run_tui(&zode).await;

    zode.shutdown().await;

    result
}

fn build_config(cli: &Cli) -> Result<ZodeConfig> {
    let listen_addr: grid_net::Multiaddr =
        cli.listen.parse().context("invalid listen multiaddr")?;

    let bootstrap_peers: Vec<grid_net::Multiaddr> = cli
        .bootstrap
        .iter()
        .map(|s| {
            grid_net::strip_zx_multiaddr(s)
                .parse()
                .context("invalid bootstrap multiaddr")
        })
        .collect::<Result<_>>()?;

    let relay_peers: Vec<grid_net::Multiaddr> = cli
        .relay
        .iter()
        .map(|s| {
            grid_net::strip_zx_multiaddr(s)
                .parse()
                .context("invalid relay multiaddr")
        })
        .collect::<Result<_>>()?;

    let topics: HashSet<ProgramId> = cli
        .topic
        .iter()
        .map(|hex| ProgramId::from_hex(hex).map_err(|e| anyhow::anyhow!("{e}")))
        .collect::<Result<_>>()?;

    let kad_mode = match cli.kademlia_mode.as_str() {
        "client" => grid_net::KademliaMode::Client,
        _ => grid_net::KademliaMode::Server,
    };

    let discovery = grid_net::DiscoveryConfig {
        enable_kademlia: !cli.no_kademlia,
        kademlia_mode: kad_mode,
        random_walk_interval: Duration::from_secs(cli.random_walk_interval),
        ..Default::default()
    };

    let network = grid_net::NetworkConfig::new(listen_addr)
        .with_bootstrap_peers(bootstrap_peers)
        .with_relay(grid_net::RelayConfig {
            enabled: cli.enable_relay || !relay_peers.is_empty(),
            relay_peers,
        })
        .with_discovery(discovery);

    let storage = grid_storage::StorageConfig::new(cli.data_dir.clone());

    let rpc = if cli.rpc {
        let bind_addr = cli.rpc_bind.parse().context("invalid --rpc-bind address")?;
        RpcConfig {
            enabled: true,
            bind_addr,
            api_key: cli.rpc_api_key.clone(),
        }
    } else {
        RpcConfig::default()
    };

    Ok(ZodeConfig {
        storage,
        default_programs: DefaultProgramsConfig {
            zid: !cli.no_zid,
            interlink: !cli.no_interlink,
        },
        topics,
        sector_limits: Default::default(),
        sector_filter: Default::default(),
        network,
        rpc,
    })
}

async fn run_tui(zode: &Zode) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;

    let mut app = App::new(zode);

    let result = event_loop(&mut terminal, &mut app).await;

    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App<'_>,
) -> Result<()> {
    loop {
        app.refresh().await;

        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Tab => app.next_screen(),
                    KeyCode::BackTab => app.prev_screen(),
                    KeyCode::Char('1') => app.screen = Screen::Status,
                    KeyCode::Char('2') => app.screen = Screen::Traverse,
                    KeyCode::Char('3') => app.screen = Screen::Peers,
                    KeyCode::Char('4') => app.screen = Screen::Log,
                    KeyCode::Char('5') => app.screen = Screen::Info,
                    KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
                    KeyCode::Enter => app.select(),
                    KeyCode::Backspace => app.back(),
                    _ => {}
                }
            }
        }
    }
}
