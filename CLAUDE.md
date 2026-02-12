# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A Linux desktop GUI wrapper for v2ray/xray/sing-box CLI proxy tools. The app manages subscriptions, generates config files, handles process lifecycle, and provides geo-routing rules — all without implementing any protocol logic. The protocol work is delegated entirely to the system-installed CLI binaries.

UI: Relm4 (GTK4) with libadwaita. Five crates: core, subscription, process, tray, ui.

## Commands

```bash
cargo check                              # type-check the workspace
cargo build                              # build all crates
cargo test --workspace                   # run all tests
cargo test -p v2ray-rs-core              # test only the core crate
cargo test -p v2ray-rs-subscription      # test only the subscription crate
cargo test -p v2ray-rs-process           # test only the process crate
cargo test -p v2ray-rs-core -- test_name # run a single test by name
```

## Architecture

Rust workspace with five crates:

### `crates/core` (`v2ray-rs-core`)

Domain models and infrastructure:

- **`models/`** — All domain types, organized by concern:
  - `proxy.rs` — `ProxyNode` enum (Vless/Vmess/Shadowsocks/Trojan) with per-protocol config structs and transport/TLS settings. Uses `#[serde(tag = "protocol")]` for tagged serialization.
  - `subscription.rs` — `Subscription` and `SubscriptionSource` (URL or file). Subscriptions own a `Vec<SubscriptionNode>` where each node can be individually enabled/disabled.
  - `routing.rs` — `RoutingRuleSet` with ordered `RoutingRule`s. Match conditions: GeoIP, GeoSite, Domain pattern, IP CIDR. Actions: Proxy/Direct/Block. Rule ordering matters (priority by position). CRUD with validation: `add_validated()`, `add_at()`, `edit_rule()`, `remove()`, `move_rule()`, `apply_preset()`.
  - `validation.rs` — `ValidationError` enum and validators for country codes (ISO 3166-1 alpha-2), IP CIDR, domain patterns (wildcard syntax), GeoSite categories.
  - `presets.rs` — `Preset` struct and `builtin_presets()` returning 3 presets: RU Direct, CN Direct, Block Ads.
  - `settings.rs` — `AppSettings` with backend config, proxy ports, update intervals, language, tray behavior. Serializes to TOML.

- **`persistence.rs`** — XDG-compliant file storage via `directories` crate. Settings in TOML (`~/.config/v2ray-rs/settings.toml`), subscriptions and routing rules in JSON (`~/.local/share/v2ray-rs/`). Uses atomic writes via `tempfile::NamedTempFile` + persist. Directories created with 0o700 permissions.

- **`backend.rs`** — Detects installed v2ray/xray/sing-box binaries by checking well-known paths (`/usr/bin/`, `/usr/local/bin/`) and `$PATH` via `which`. Validates executability, extracts version strings. Provides install guidance strings per backend.

- **`geodata.rs`** — `GeodataManager` for GeoIP/GeoSite database management. Handles metadata (last check timestamp, versions), path resolution per backend type (.dat for v2ray/xray, .db for sing-box), update checks (`needs_update()`), and blocking downloads from v2fly/SagerNet GitHub releases. Feature-gated `geodata-fetch` for reqwest blocking client.

- **`routing_manager.rs`** — `RoutingManager` coordinating rule CRUD with persistence and config generation. All mutations (add, edit, delete, reorder, apply_preset) auto-persist. `write_config()` generates backend config from current enabled rules.

### `crates/subscription` (`v2ray-rs-subscription`)

Depends on `v2ray-rs-core`. Handles subscription fetching and URI parsing:

- **`fetch.rs`** — HTTP fetching (reqwest with rustls-tls, 30s connect / 60s total timeout) and local file reading. `decode_subscription_content()` handles both base64-encoded and plaintext subscription responses, splitting into individual URI lines.

- **`parser.rs`** — Parses proxy URIs (`vless://`, `vmess://`, `ss://`, `trojan://`) into `ProxyNode` variants. VMess uses base64-encoded JSON. Shadowsocks uses base64-encoded `method:password` userinfo. VLESS and Trojan use standard URL parsing.

- **`ping.rs`** — TCP connect latency testing. `tcp_ping()` measures TCP connection time with 5s timeout. `ping_nodes()` pings all nodes concurrently via `tokio::spawn`.

### `crates/process` (`v2ray-rs-process`)

Depends on `v2ray-rs-core`, `tokio`, and `nix`. Async process lifecycle management:

- **`state.rs`** — `ProcessState` enum (Stopped/Starting/Running/Stopping/Error) with validated transitions. `StateManager` wraps state + tokio broadcast channel for event subscribers. `ProcessEvent` enum: StateChanged, LogLine, ProcessExited.

- **`log_buffer.rs`** — Circular `LogBuffer` (VecDeque, 10K lines max) with `LogLine` (source: Stdout/Stderr, content). Methods: push, last_n, search (case-insensitive). Pure sync data structure.

- **`pid.rs`** — `PidFile` for writing/reading/removing PID files. `check_and_kill_orphaned()` detects stale processes from previous runs using `kill(pid, 0)` signal probe.

- **`manager.rs`** — `ProcessManager` orchestrator. Spawns backend via `tokio::process::Command` with ETXTBSY retry (handles overlayfs race in containers), pipes stdout/stderr through async line readers into shared `Arc<Mutex<LogBuffer>>` + broadcast channel. Graceful stop (SIGTERM → 5s → SIGKILL). Crash recovery with 2s delay, max 3 crashes per minute before Error state. PID file lifecycle.

### `crates/tray` (`v2ray-rs-tray`)

System tray integration via ksni (StatusNotifierItem protocol):

- **`tray.rs`** — `AppTray` implements `ksni::Tray`. Uses `icon_name()` + `icon_theme_path()` for FreeDesktop theme-aware symbolic icons, with `icon_pixmap()` as fallback. Menu items: Connect/Disconnect toggle, status label, Open Main Window, Quit. `TrayService::spawn()` listens for process state events and updates tray state.

- **`icons.rs`** — Embeds both PNG icons (pixmap fallback) and symbolic SVGs. `setup_icon_theme()` creates a temporary FreeDesktop icon theme directory at runtime with hicolor/scalable/status structure. Symbolic icons: shield outline (disconnected), shield+checkmark (connected), shield+X (error).

- **`notification.rs`** — Desktop notifications via `notify-rust` for state changes.

### `crates/ui` (`v2ray-rs-ui`)

GTK4/Relm4 GUI application:

- **`app.rs`** — Main `App` component. HeaderBar with ViewSwitcher and Connect/Disconnect button (pack_end). ToastOverlay wraps content for status messages. Connection state tracked inline (connected, button_sensitive). Process lifecycle management via tokio tasks.

- **`subscriptions.rs`** — Subscription management page. Features: add/rename/delete subscriptions, toggle nodes, move up/down reordering (subscriptions and nodes), latency testing (TCP ping), sort by latency. Uses `capture_expanded()` to preserve ExpanderRow state across re-renders.

- **`routing.rs`** — Routing rule management with drag-and-drop reordering.

- **`settings.rs`** — App settings editor.

- **`logs.rs`** — Process log viewer.

- **`wizard.rs`** — First-run onboarding wizard.

### Data flow

```
Subscription URL/File → fetch → decode (base64?) → split lines → parse URIs → Vec<ProxyNode>
                                                                                    ↓
AppSettings + ProxyNodes + RoutingRuleSet → config generation → JSON config file
                                                                                    ↓
                                                         ProcessManager → spawn backend binary
                                                              ↕                     ↕
                                                    state events (broadcast)   log capture (buffer)
```

## Key Patterns

- **Tagged enums for serde**: `ProxyNode`, `TransportSettings`, `RuleMatch`, `SubscriptionSource` all use `#[serde(tag = "...")]` for self-describing JSON/TOML.
- **Atomic file writes**: All persistence goes through `tempfile::NamedTempFile` → `persist()` to avoid corruption.
- **Tests use `tempfile::TempDir`**: Persistence tests create isolated temp directories via `AppPaths::from_paths()` (cfg(test) only constructor).
- **Workspace dependencies**: All shared deps declared in root `Cargo.toml` `[workspace.dependencies]` and used via `.workspace = true`.
- **Edition 2024**: Uses Rust edition 2024.

## Versioning

When bumping the version, update all three places:
1. `Cargo.toml` — `[workspace.package] version`
2. `pkg/archlinux/PKGBUILD` — `pkgver`
3. `CHANGELOG.md` — new section + link refs at bottom

Then run `cargo check` to regenerate `Cargo.lock`.

## OpenSpec

Feature specifications live in `openspec/changes/`. Each change has a proposal, design, delta specs, and task breakdown. Planned features: backend detection, process management, config generation, subscription management, routing rule manager, main window UI, system tray integration.
