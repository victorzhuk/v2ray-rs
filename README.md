<div align="center">

<img src="assets/v2ray-rs.png" width="256" alt="v2ray-rs">

# v2ray-rs

**A modern Linux desktop GUI for v2ray/xray/sing-box proxy management**

[![Rust](https://img.shields.io/badge/rust-1.93.1-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/victorzhuk/v2ray-rs/ci.yml)](https://github.com/victorzhuk/v2ray-rs/actions)

</div>

---

## Features

- **Multi-backend**: v2ray, xray, sing-box — auto-detected from system PATH
- **Subscriptions**: Fetch and parse from URLs or local files (VLESS, VMess, Shadowsocks, Trojan)
- **Routing rules**: GeoIP, GeoSite, domain patterns, IP CIDR with proxy/direct/block actions
- **Process management**: Async lifecycle with crash recovery, graceful shutdown, log capture
- **GTK4/libadwaita UI**: Native Linux desktop experience with system tray integration
- **XDG compliant**: Full XDG Base Directory layout with runtime profiles and per-directory overrides
- **Real Delay testing**: End-to-end latency probes through each proxy node. Supports sing-box with Clash API and xray/v2ray with ObservatoryService.
- **TUN mode**: System-wide transparent proxying for sing-box and xray via a virtual interface — no per-app setup. One-time `setcap` privilege grant, with automatic route recovery after an unclean shutdown.

---

## Installation

Prebuilt releases target **GTK 4.18 / libadwaita 1.7 and glibc 2.41** — Debian 13+,
Ubuntu 25.04+, Fedora 42+, Arch, openSUSE Tumbleweed. On anything older, use the AppImage.

### Install script

```bash
curl -fsSL https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install.sh | sh
```

Installs under `$HOME/.local` by default:

| Path | Contents |
| --- | --- |
| `bin/` | `v2ray-rs`, `v2ray-rs-netctl`, `v2ray-rs-run` |
| `share/applications/` | desktop entry |
| `share/icons/hicolor/` | app and symbolic icons |
| `share/locale/` | `en_US`, `ru_RU` translations |

The script verifies the download against a checksum baked in when the installer
was published, falling back to the `.sha256` beside the tarball when you pin a
different version or override the URL, and telling you plainly if it ends up
with neither. It checks that GTK and libadwaita are new enough before unpacking
anything, and adds `bin/` to `PATH` only when it is missing — appending to
`~/.profile`, `~/.bashrc` and `~/.zshrc`, never creating a `~/.bash_profile`
that would shadow an existing `~/.profile`.

It never runs `sudo`. A home-directory install cannot support TUN mode, because
the privileged helpers have to be owned by root; the script says so and points
at the system-wide install or a distribution package (see [TUN mode](#tun-mode)).

```bash
V2RAY_RS_INSTALL_DIR=/usr/local sh install.sh   # different prefix
V2RAY_RS_VERSION=0.16.1 sh install.sh           # pin a release
V2RAY_RS_NO_MODIFY_PATH=1 sh install.sh         # leave shell rc files alone
sh install.sh --uninstall                       # remove it again
```

### AppImage

Self-contained, bundles GTK 4.18 and libadwaita — use it when your distribution
ships something older.

```bash
curl -fsSLO https://github.com/victorzhuk/v2ray-rs/releases/latest/download/v2ray-rs-x86_64.AppImage
chmod +x v2ray-rs-x86_64.AppImage
./v2ray-rs-x86_64.AppImage
```

TUN mode works, with one extra step. An AppImage is mounted `nosuid`, so the
route helper cannot hold `cap_net_admin` where it sits — **Grant TUN
privileges** in the TUN preferences installs it to `/usr/local/lib/v2ray-rs/`
during the same `pkexec` prompt and grants it there. Re-run the grant after
updating the AppImage; the app prompts when it notices a stale helper.

*Run with bypass* is unavailable from an AppImage: it needs the
`v2ray-rs-bypass` system user, which only a distribution package creates.

### Arch Linux (AUR)

```bash
yay -S v2ray-rs-bin   # prebuilt from the release tarball
yay -S v2ray-rs       # compiled from source
```

Either package installs the privileged helpers and sets their capabilities
through the install hook, so TUN mode works after a single re-login.

### From Source

Requires GTK 4.18 and libadwaita 1.7 or newer.

```bash
# Dependencies (Debian/Ubuntu)
sudo apt install libgtk-4-dev libadwaita-1-dev protobuf-compiler

# Dependencies (Fedora)
sudo dnf install gtk4-devel libadwaita-devel protobuf-compiler

# Dependencies (Arch)
sudo pacman -S gtk4 libadwaita protobuf

# Build
git clone https://github.com/victorzhuk/v2ray-rs.git
cd v2ray-rs
cargo build --release
```

`make dist` produces the same tarball the release workflow ships, provided the
`x86_64-unknown-linux-musl` target is installed.

You also need at least one proxy backend installed: `v2ray`, `xray`, or `sing-box`.

---

## Usage

1. Launch the app (or `cargo run -p v2ray-rs-ui` from source)
2. The onboarding wizard detects installed backends and guides initial setup
3. Add a subscription URL or local subscription file — nodes are fetched and parsed automatically
4. Enable desired nodes, configure routing rules, click **Connect**

### Real Delay

Real Delay measures the full proxy path by sending the configured test URL through each tested node. It is distinct from TCP ping and can be used for manual sorting or the Lowest Latency strategy.

Supported backends:
- sing-box with Clash API
- xray with ObservatoryService
- v2fly/v2ray-core with ObservatoryService

Privacy details: [`docs/real-delay-privacy.md`](docs/real-delay-privacy.md).

### TUN mode

TUN mode creates a virtual network interface that becomes the system default route, transparently proxying **all** traffic with no per-app configuration. It is available for **sing-box** (self-routes via `auto_route`) and **xray** (a minimal `v2ray-rs-netctl` helper programs the address and split routes); v2ray-core has no native TUN inbound and is excluded.

TUN requires `CAP_NET_ADMIN` on the backend binary. The TUN preferences page offers a one-time **Grant TUN privileges** action (`setcap` via `pkexec`) and re-checks capabilities before each start. The Arch package installs `v2ray-rs-netctl` and grants it `cap_net_admin` via its install hook; from source, build the workspace so the helper sits alongside the UI binary.

### Configuration

Settings are stored in `~/.config/v2ray-rs/settings.toml`:

```toml
version = 1
socks_port = 1080
http_port = 1081
auto_resolve_strategy = "last-successful"
auto_update_subscriptions = true
subscription_update_interval_secs = 86400
auto_update_geodata = true
geodata_update_interval_secs = 604800
language = "english"
minimize_to_tray = true
notifications_enabled = true
onboarding_complete = true

[backend]
backend_type = "xray"
binary_path = "/usr/bin/xray"

[dns]
enabled = false
```

### Runtime Profiles

The app supports isolated storage profiles so development, test, and production builds never share data:

| Profile | Qualifier | App ID | Default |
|---------|-----------|--------|---------|
| `production` | `v2ray-rs` | `com.github.v2ray-rs` | Release builds |
| `development` | `v2ray-rs-dev` | `com.github.v2ray-rs.dev` | Debug builds |
| `test` | `v2ray-rs-test` | `com.github.v2ray-rs.test` | — |
| `custom:<name>` | `v2ray-rs-<name>` | `com.github.v2ray-rs.<name>` | — |

Resolution order: `--profile` flag > `V2RAY_RS_PROFILE` env > `V2RAY_RS_DEV` env (deprecated) > compile-time default.

Per-directory overrides:

```bash
v2ray-rs --data-dir /tmp/scratch/data --cache-dir /tmp/scratch/cache
```

CLI flags and matching env vars: `--config-dir` (`V2RAY_RS_CONFIG_DIR`), `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`.

To wipe a non-production profile and start fresh:

```bash
v2ray-rs --profile development --reset-instance
```

Production profiles require explicit confirmation:

```bash
v2ray-rs --profile production --reset-instance --i-understand
```

---

## Building & Testing

```bash
cargo check --workspace --all-targets       # type-check
cargo build --release         # release build
cargo test --workspace --all-targets        # all tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all               # format
```

Or via Makefile: `make build`, `make test`, `make lint`, `make fmt`.

---

<div align="center">


[v2ray](https://github.com/v2fly/v2ray-core) / [xray](https://github.com/XTLS/Xray-core) / [sing-box](https://github.com/SagerNet/sing-box) / [Relm4](https://github.com/Relm4/Relm4) / [v2fly GeoIP/GeoSite](https://github.com/v2fly/geoip)

</div>
