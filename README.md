# v2ray-rs

<div align="center">

**A modern Linux desktop GUI for v2ray/xray/sing-box proxy management**

[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/victorzhuk/v2ray-rs/ci.yml)](https://github.com/victorzhuk/v2ray-rs/actions)
[![Version](https://img.shields.io/crates/v/v2ray-rs)](https://crates.io/crates/v2ray-rs)
[![Documentation](https://img.shields.io/badge/docs-latest-brightgreen.svg)](https://docs.rs/v2ray-rs)

</div>

---

## Table of Contents

- [Features](#features)
- [Quick Start](#quick-start)
- [Installation](#installation)
- [Architecture](#architecture)
- [Project Structure](#project-structure)
- [Building from Source](#building-from-source)
- [Usage](#usage)
- [Configuration](#configuration)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgments](#acknowledgments)

---

## Features

### Core Functionality

- **Multi-backend Support**: Works with v2ray, xray, and sing-box CLI tools
- **Subscription Management**: Fetch and parse proxy subscriptions from URLs or local files
- **Protocol Support**: VLESS, VMess, Shadowsocks, Trojan
- **Routing Rules**: GeoIP, GeoSite, domain pattern, and IP CIDR matching with proxy/direct/block actions
- **Rule Presets**: Built-in presets (RU Direct, CN Direct, Block Ads)
- **Process Management**: Async process lifecycle with crash recovery, graceful shutdown, and log capture
- **Geodata Management**: Automatic GeoIP/GeoSite database updates from v2fly/SagerNet

### User Interface

- **Modern GTK4/Relm4**: Native Linux desktop experience with libadwaita integration
- **System Tray**: Minimize to tray with status indicators
- **i18n Support**: Multi-language support via gettext

### Infrastructure

- **XDG Compliance**: Follows freedesktop.org standards for config and data directories
- **Atomic File Writes**: Corruption-safe persistence via temporary files
- **Clean Architecture**: DDD-inspired domain models with layered separation
- **Comprehensive Testing**: Full test coverage with isolated environments

---

## Quick Start

### Prerequisites

- Linux distribution with GTK4 support
- Rust 1.85 or later
- One or more of the following CLI tools:
  - v2ray (v4.40+)
  - xray (v1.8+)
  - sing-box (v1.8+)

### Basic Usage

```bash
# Check available backends
v2ray-rs check-backend

# Import a subscription
v2ray-rs import-url https://example.com/subscription

# Start the GUI
v2ray-rs gui
```

---

## Installation

### From Release Binaries

Download the latest release from [GitHub Releases](https://github.com/victorzhuk/v2ray-rs/releases).

### From Source

See [Building from Source](#building-from-source).

### Package Managers

*Coming soon: AUR, Flatpak, DEB/RPM packages*

---

## Architecture

v2ray-rs follows Clean Architecture principles with clear layer separation:

```
┌─────────────────────────────────────────────────────────┐
│                    Presentation Layer                   │
│              (GTK4/Relm4 UI, System Tray)               │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│                    Application Layer                    │
│        (RoutingManager, ProcessManager, Geodata)        │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│                      Domain Layer                       │
│  (ProxyNode, Subscription, RoutingRuleSet, AppSettings) │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│                  Infrastructure Layer                   │
│     (Persistence, HTTP Client, Process Spawning, IO)    │
└─────────────────────────────────────────────────────────┘
```

### Key Design Principles

- **Domain-Centric**: Business logic lives in domain models, not services
- **Zero Protocol Logic**: Delegates all proxy protocol work to CLI binaries
- **Testability**: Domain layer has zero external dependencies
- **Simplicity**: KISS over abstraction, concrete types over interfaces

---

## Project Structure

```
v2ray-rs/
├── crates/
│   ├── core/              # Domain models and infrastructure
│   │   ├── src/
│   │   │   ├── models/    # Domain entities (proxy, subscription, routing)
│   │   │   ├── config/    # Backend config generation (v2ray/xray/singbox)
│   │   │   ├── persistence.rs
│   │   │   ├── backend.rs
│   │   │   ├── geodata.rs
│   │   │   └── routing_manager.rs
│   │   └── Cargo.toml
│   ├── subscription/      # Subscription fetching and parsing
│   │   ├── src/
│   │   │   ├── fetch.rs
│   │   │   └── parser.rs
│   │   └── Cargo.toml
│   └── process/          # Async process lifecycle management
│       ├── src/
│       │   ├── state.rs
│       │   ├── log_buffer.rs
│       │   ├── pid.rs
│       │   └── manager.rs
│       └── Cargo.toml
├── openspec/              # Feature specifications (OpenSpec workflow)
├── docs/                  # Documentation
├── locale/                # i18n translations
├── assets/                # UI resources
├── Cargo.toml             # Workspace configuration
├── Makefile               # Build automation
└── README.md
```

### Crate Overview

- **v2ray-rs-core**: Domain models, persistence, backend detection, geodata, routing, config generation
- **v2ray-rs-subscription**: HTTP fetching, URI parsing, subscription content handling
- **v2ray-rs-process**: Tokio-based process management with graceful shutdown, log capture, crash recovery

---

## Building from Source

### Prerequisites

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install development dependencies
# Ubuntu/Debian
sudo apt install libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install gtk4-devel libadwaita-devel
```

### Build Commands

```bash
# Clone the repository
git clone https://github.com/victorzhuk/v2ray-rs.git
cd v2ray-rs

# Build all crates (debug)
cargo build

# Build release optimized
cargo build --release

# Run tests
cargo test --workspace

# Run tests with race detector
cargo test --workspace --release --all-features

# Run specific crate tests
cargo test -p v2ray-rs-core
cargo test -p v2ray-rs-subscription
cargo test -p v2ray-rs-process

# Type-check without building
cargo check --workspace

# Run linter
cargo clippy --workspace -- -D warnings

# Format code
cargo fmt --all
```

### Using Makefile

```bash
make build        # Build debug
make build-release # Build release
make test         # Run tests
make lint         # Run clippy
make fmt          # Format code
make clean        # Clean build artifacts
```

---

## Usage

### Command Line Interface

```bash
# Check installed backends
v2ray-rs check-backend

# Import subscription from URL
v2ray-rs import-url <SUBSCRIPTION_URL>

# Import subscription from file
v2ray-rs import-file <PATH>

# Generate configuration file
v2ray-rs gen-config --backend xray --output /tmp/config.json

# Start the GUI
v2ray-rs gui

# Start the backend daemon directly
v2ray-rs start --backend xray --config /path/to/config.json
```

### Configuration Files

Configuration is stored in XDG-compliant locations:

- **Settings**: `~/.config/v2ray-rs/settings.toml`
- **Subscriptions**: `~/.local/share/v2ray-rs/subscriptions.json`
- **Routing Rules**: `~/.local/share/v2ray-rs/routing.json`

### GUI Usage

1. **Backend Detection**: App automatically detects installed v2ray/xray/sing-box binaries
2. **Import Subscription**: File → Import from URL/File
3. **Configure Proxy Nodes**: Enable/disable nodes, test latency
4. **Setup Routing**: Add/edit rules, apply presets, reorder by priority
5. **Start Proxy**: Click "Connect" or toggle from system tray

---

## Configuration

### Application Settings (settings.toml)

```toml
[backend]
type = "xray"  # "v2ray", "xray", or "sing-box"
binary_path = "/usr/bin/xray"  # Optional: override auto-detection

[proxy]
http_port = 10809
socks_port = 10808

[updates]
check_interval_days = 1
auto_update_geodata = true

[ui]
language = "en"
minimize_to_tray = true
```

### Backend Config Generation

The app automatically generates backend-specific configs:

- **v2ray**: JSON config with inbound/outbound rules
- **xray**: Extended v2ray format with XTLS support
- **sing-box**: Native sing-box JSON schema

### Routing Rules

Rules are evaluated in order (priority by position):

```json
[
  {
    "match": {
      "type": "domain",
      "pattern": "ads.example.com"
    },
    "action": "block"
  },
  {
    "match": {
      "type": "geosite",
      "category": "cn"
    },
    "action": "direct"
  },
  {
    "match": {
      "type": "geoip",
      "country_code": "ru"
    },
    "action": "direct",
    "outbound": "proxy-node-1"
  }
]
```

---

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

### Quick Start for Contributors

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Make your changes
4. Run tests: `make test && make lint`
5. Commit: `git commit -m "Add amazing feature"`
6. Push: `git push origin feature/amazing-feature`
7. Open a Pull Request

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## Acknowledgments

### Open Source Projects

- **v2ray/xray**: Core proxy protocol implementations
- **sing-box**: Universal proxy platform
- **Relm4**: Elegant GTK4 framework for Rust
- **v2fly/SagerNet**: GeoIP/GeoSite databases

---

<div align="center">

Made with ❤️ for the Linux community

[GitHub](https://github.com/victorzhuk/v2ray-rs) · [Issues](https://github.com/victorzhuk/v2ray-rs/issues) · [Discussions](https://github.com/victorzhuk/v2ray-rs/discussions)

</div>
