# Run a ZODE

## Prerequisites

### All platforms

- **Rust** stable toolchain (edition 2021). Install via [rustup](https://rustup.rs/).
- **C/C++ compiler** — required by the `rocksdb` crate which builds RocksDB from source.
- **CMake** — required on some platforms for the RocksDB build.

### Windows

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload (provides MSVC, CMake, and the Windows SDK).

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

Clone the repository and build the entire workspace:

```sh
git clone <repo-url> && cd the-grid
cargo build
```

### Desktop GUI

```sh
cargo run -p zode-app
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
cargo build --release -p zode-app
```

Binary: `target\release\zode-app.exe`

### macOS (aarch64-apple-darwin / x86\_64-apple-darwin)

```sh
cargo build --release -p zode-app
```

Binary: `target/release/zode-app`

To produce a universal binary that runs on both Apple Silicon and Intel:

```sh
# Build each architecture
rustup target add x86_64-apple-darwin
cargo build --release -p zode-app --target aarch64-apple-darwin
cargo build --release -p zode-app --target x86_64-apple-darwin

# Combine with lipo
lipo -create \
  target/aarch64-apple-darwin/release/zode-app \
  target/x86_64-apple-darwin/release/zode-app \
  -output zode-app-universal
```

### Linux (x86\_64-unknown-linux-gnu)

Install the prerequisite system packages listed above, then:

```sh
cargo build --release -p zode-app
```

Binary: `target/release/zode-app`

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

### Environment Variables

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls tracing verbosity (e.g. `info`, `debug`, `warn`, `zode=debug,grid_net=trace`) |

## Running Tests

```sh
cargo test --workspace
```
