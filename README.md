<div align="center">

<img src="assets/v2ray-rs.png" width="128" alt="v2ray-rs">

# v2ray-rs

**A modern Linux desktop GUI for v2ray/xray/sing-box proxy management**

[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/victorzhuk/v2ray-rs/ci.yml)](https://github.com/victorzhuk/v2ray-rs/actions)

</div>

---

## Features

- **Multi-backend**: v2ray, xray, sing-box — auto-detected from system PATH
- **Subscriptions**: Fetch and parse from URLs or local files (VLESS, VMess, Shadowsocks, Trojan)
- **Routing rules**: GeoIP, GeoSite, domain patterns, IP CIDR with proxy/direct/block actions
- **Process management**: Async lifecycle with crash recovery, graceful shutdown, log capture
- **GTK4/libadwaita UI**: Native Linux desktop experience with system tray integration
- **XDG compliant**: Config in `~/.config/v2ray-rs/`, data in `~/.local/share/v2ray-rs/`

---

## Installation

### Arch Linux (AUR)

```bash
yay -S v2ray-rs
```

### Debian/Ubuntu (.deb)

Download the latest `.deb` from [GitHub Releases](https://github.com/victorzhuk/v2ray-rs/releases):

```bash
sudo dpkg -i v2ray-rs_*.deb
sudo apt-get install -f
```

### From Source

```bash
# Dependencies (Debian/Ubuntu)
sudo apt install libgtk-4-dev libadwaita-1-dev

# Dependencies (Fedora)
sudo dnf install gtk4-devel libadwaita-devel

# Build
git clone https://github.com/victorzhuk/v2ray-rs.git
cd v2ray-rs
cargo build --release
```

You also need at least one proxy backend installed: `v2ray`, `xray`, or `sing-box`.

---

## Usage

1. Launch the app (or `cargo run -p v2ray-rs-ui` from source)
2. The onboarding wizard detects installed backends and guides initial setup
3. Add a subscription URL — nodes are fetched and parsed automatically
4. Enable desired nodes, configure routing rules, click **Connect**

### Configuration

Settings are stored in `~/.config/v2ray-rs/settings.toml`:

```toml
[backend]
type = "xray"
binary_path = "/usr/bin/xray"

[proxy]
http_port = 10809
socks_port = 10808

[ui]
language = "en"
minimize_to_tray = true
```

---

## Building & Testing

```bash
cargo check --workspace       # type-check
cargo build --release         # release build
cargo test --workspace        # all tests
cargo clippy --workspace      # lint
cargo fmt --all               # format
```

Or via Makefile: `make build`, `make test`, `make lint`, `make fmt`.

---

<div align="center">

Made with ❤️ for the Linux community

[v2ray](https://github.com/v2fly/v2ray-core) / [xray](https://github.com/XTLS/Xray-core) / [sing-box](https://github.com/SagerNet/sing-box) / [Relm4](https://github.com/Relm4/Relm4) / [v2fly GeoIP/GeoSite](https://github.com/v2fly/geoip)

</div>
