# Run a ZODE

- [Prerequisites](#prerequisites)
- [Running Locally](#running-locally)
- [Building Release Binaries](#building-release-binaries)
- [Configuration](#configuration)
- [Run a Relay](#run-a-relay)
- [Running Tests](#running-tests)

## Prerequisites

### All platforms

- **Rust** stable toolchain (edition 2021). Install via [rustup](https://rustup.rs/).
- **C/C++ compiler** — required by the `rocksdb` crate which builds RocksDB from source.
- **CMake** — required on some platforms for the RocksDB build.
- **[zid](https://github.com/cypher-asi/zid)** — the PQ-hybrid identity crate. Clone it so that it sits **next to** (sibling of) the `the-grid` directory (see [Running Locally](#running-locally)).

### Windows

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload (provides MSVC, CMake, and the Windows SDK).
- **Windows x64 `libclang` requirement (if build fails):** If you see `Unable to find libclang ... set the LIBCLANG_PATH environment variable`, install 64-bit [LLVM](https://llvm.org/) and set `LIBCLANG_PATH` to your LLVM `bin` folder (where `libclang.dll` lives), for example:

```powershell
$env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"
setx LIBCLANG_PATH "C:\Program Files\LLVM\bin"
```

### macOS

- Xcode Command Line Tools:

```sh
xcode-select --install
```

### Linux

- GCC/G++ (or Clang), CMake, and GUI libraries for eframe:

```sh
# Debian / Ubuntu
sudo apt install build-essential cmake libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxkbcommon-dev libgtk-3-dev
```

## Running Locally

Clone both **the-grid** and the **zid** dependency so they sit side-by-side:

```sh
git clone https://github.com/cypher-asi/zid
git clone https://github.com/cypher-asi/the-grid && cd the-grid
cargo build
```

> **Important:** Several workspace crates depend on the `zid` crate via a
> relative path (`../../../zid`). The final directory layout must look like:
>
> ```
> parent-dir/
>   zid/          # ← git clone https://github.com/cypher-asi/zid
>   the-grid/     # ← git clone https://github.com/cypher-asi/the-grid
> ```
>
> If the `zid` repo is missing or in the wrong location, `cargo build` will
> fail with a path-dependency error.

### Desktop GUI

```sh
cargo run -p zode-bin
```

The GUI opens a settings panel on first launch where you can configure the data
directory, listen address, bootstrap peers, and program subscriptions before
starting the node.

### Console TUI

```sh
cargo run -p zode-cli -- --help
```

Key flags:

| Flag | Default | Description |
|---|---|---|
| `--data-dir` | `.zode/data` | RocksDB storage directory |
| `--listen` | `/ip4/0.0.0.0/udp/3690/quic-v1` | libp2p listen multiaddr |
| `--bootstrap <ADDR>` | *(none)* | Bootstrap peer multiaddr (repeatable) |
| `--enable-relay` | off | Enable relay transport for NAT-restricted nodes |
| `--relay <ADDR>` | *(none)* | Public relay multiaddr (repeatable) |
| `--enable-kademlia` | off | Enable Kademlia DHT peer discovery |
| `--kademlia-mode` | `server` | `server` for Zodes, `client` for SDK clients |
| `--no-zid` | off | Disable the ZID program |
| `--no-interlink` | off | Disable the Interlink program |

### Logging

Set the `RUST_LOG` environment variable to control log output:

```sh
RUST_LOG=info cargo run -p zode-cli
```

## Building Release Binaries

All release builds use the same command pattern. Run the build **natively** on
each target platform — there is no cross-compilation configuration.

### Windows (x86\_64-pc-windows-msvc)

```powershell
cargo build --release -p zode-bin
```

Binary: `target\release\zode-bin.exe`

### macOS (aarch64-apple-darwin / x86\_64-apple-darwin)

```sh
cargo build --release -p zode-bin
```

Binary: `target/release/zode-bin`

To produce a universal binary that runs on both Apple Silicon and Intel:

```sh
# Build each architecture
rustup target add x86_64-apple-darwin
cargo build --release -p zode-bin --target aarch64-apple-darwin
cargo build --release -p zode-bin --target x86_64-apple-darwin

# Combine with lipo
lipo -create \
  target/aarch64-apple-darwin/release/zode-bin \
  target/x86_64-apple-darwin/release/zode-bin \
  -output zode-bin-universal
```

### Linux (x86\_64-unknown-linux-gnu)

Install the prerequisite system packages listed above, then:

```sh
cargo build --release -p zode-bin
```

Binary: `target/release/zode-bin`

## Configuration

### ZodeConfig

The `ZodeConfig` struct (in `crates/zode/src/config.rs`) controls node behavior:

| Field | Type | Default | Description |
|---|---|---|---|
| `storage` | `StorageConfig` | `.zode/data`, LZ4 compression, 512 open files | RocksDB path and tuning |
| `default_programs` | `DefaultProgramsConfig` | ZID + Interlink enabled | Toggle built-in programs |
| `topics` | `HashSet<ProgramId>` | empty | Additional program topics to subscribe to |
| `sector_limits` | `SectorLimitsConfig` | 256 KB max slot, unlimited per-program | Sector size constraints |
| `sector_filter` | `SectorFilter` | `All` | Per-sector accept filter |
| `network` | `NetworkConfig` | QUIC on `0.0.0.0:3690`, Kademlia server mode | libp2p transport and discovery |

### Adding third-party Programs

To subscribe a ZODE to a custom (non-default) Program, pass one or more
`--topic <program_id_hex>` flags to `zode-cli`, or add the 32-byte
`ProgramId` to `ZodeConfig::topics` (and the desktop GUI's Settings →
Programs panel for `zode-bin`). Both paths feed into
`ZodeConfig::effective_topics()` in `crates/zode/src/config.rs`.

For a full walkthrough of defining, publishing, and operating third-party
Programs and Services, see [Build a Program](build-a-program.md).

### Environment Variables

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls tracing verbosity (e.g. `info`, `debug`, `warn`, `zode=debug,grid_net=trace`) |

### Relay-First NAT Connectivity

Run a public relay service:

```sh
cargo run -p grid-relayd -- --listen /ip4/0.0.0.0/tcp/3691
```

Configure a Zode to use that relay:

```sh
cargo run -p zode-cli -- \
  --enable-relay \
  --relay /ip4/<relay-public-ip>/tcp/3691/p2p/<relay-peer-id> \
  --bootstrap /ip4/<bootstrap-ip>/udp/3690/quic-v1/p2p/<bootstrap-peer-id>
```

Relay service env vars (`CLI > env` precedence):

| Variable | Description |
|---|---|
| `GRID_RELAY_LISTEN` | Relay listen multiaddr |
| `GRID_RELAY_LOG` | Relay log filter (fallback to `RUST_LOG`) |
| `GRID_RELAY_MAX_RESERVATIONS` | Optional reservation limit (parsed and validated) |
| `GRID_RELAY_MAX_CIRCUITS` | Optional circuit limit (parsed and validated) |

Example systemd-style environment:

```sh
GRID_RELAY_LISTEN=/ip4/0.0.0.0/tcp/3691
GRID_RELAY_LOG=info
GRID_RELAY_MAX_RESERVATIONS=256
GRID_RELAY_MAX_CIRCUITS=512
```

Networking notes:

- Expose the relay TCP port publicly (security groups/firewall).
- Keep regular Zode bootstrap peers configured; relay is a connectivity fallback path.

## Run a Relay

For standalone relay hosting (local or cloud), firewall setup, and validation:

- See [Run a Relay](run-a-relay.md).

## Running Tests

```sh
cargo test --workspace
```
