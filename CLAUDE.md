# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A Linux desktop GUI wrapper for v2ray/xray/sing-box CLI proxy tools. The app manages subscriptions, generates config files, handles process lifecycle, and provides geo-routing rules — all without implementing any protocol logic. The protocol work is delegated entirely to the system-installed CLI binaries.

UI: Relm4 (GTK4) with libadwaita. Six crates: core, subscription, process, netctl, tray, ui.

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
  - `validation.rs` — `ValidationError` enum and validators for country codes (extended: ISO 3166-1 + GeoIP tags like GOOGLE/NETFLIX), IP CIDR, domain patterns (wildcard syntax), GeoSite categories.
  - `presets.rs` — `Preset` struct and `builtin_presets()` returning 4 presets: RU Bypass, CN Direct, Proxy Popular, Block Ads.
  - `settings.rs` — `AppSettings` with backend config, proxy ports, `listen_address` (default `127.0.0.1`, validated as an IPv4/IPv6 literal via `AppSettings::validate_listen_address`), update intervals, language, tray behavior, `auto_resolve_strategy`, `last_success: Option<LastSuccessMetadata>`, `dns: DnsConfig`, and `tun: TunConfig`. Serializes to TOML.
  - `tun.rs` — `TunConfig` (enabled, interface_name, mtu, address_v4, address_v6, stack, strict_route, dns_hijack, exclude_routes, exclude_processes, exclude_domains) with `validate()` and a `TunConfigWire` forward-compat wire struct. `TunStack` (System/Gvisor/Mixed) and `DnsHijackMode` (Hijack/Native/Disabled) enums serialize to backend literals. Default is TUN disabled.
  - `dns.rs` — `DnsConfig` (enabled, strategy, servers, rules, fakeip, disable_cache, client_subnet, hosts, use_custom_rules). `DnsServerConfig` (tag, protocol, address, port, detour). `DnsProtocol` enum (UDP/TCP/DoH/DoT/DoQ/H3) with `server_address()` and `default_port()`. `DnsStrategy` (PreferIpv4/PreferIpv6/Ipv4Only/Ipv6Only). `DnsRule`, `FakeIpConfig`, `HostOverride`. `builtin_dns_presets()` returns 8 presets (Cloudflare, Google, AdGuard, Quad9, Ali DNS, Yandex DNS variants). `DnsConfigWire` for backward-compat migration from the old two-field format.
  - `resolve.rs` — `AutoResolveStrategy` enum (ListOrder/LowestLatency/Random/LastSuccessful/GeoAware). `ConnectionMetadata` for active connection details. `LastSuccessMetadata` and `LatencySample`.

- **`persistence.rs`** — XDG-compliant file storage via `directories` crate. Settings in TOML (`~/.config/v2ray-rs/settings.toml`), subscriptions and routing rules in JSON (`~/.local/share/v2ray-rs/`). Also persists `latency_snapshot.json`, a `tun_session.json` TUN-active marker (written at TUN connect, removed on clean stop; its presence at startup triggers a route-recovery pass), and custom presets. Uses atomic writes via `tempfile::NamedTempFile` + persist. Directories created with 0o700 permissions. `AppPaths::new_dev()` isolates dev mode under a `v2ray-rs-dev` qualifier. `AppPaths::from_paths()` is gated on `cfg(any(test, feature = "test-utils"))`.

- **`backend.rs`** — Detects installed v2ray/xray/sing-box binaries by checking well-known paths (`/usr/bin/`, `/usr/local/bin/`) and `$PATH` via `which`. Validates executability, extracts version strings. Provides install guidance strings per backend.

- **`geodata.rs`** — `GeodataManager` for GeoIP/GeoSite database management. Handles metadata (last check timestamp, versions), path resolution per backend type (.dat for v2ray/xray, .db for sing-box), update checks (`needs_update()`), and blocking downloads from v2fly/SagerNet GitHub releases. Feature-gated `geodata-fetch` for reqwest blocking client.

- **`routing_manager.rs`** — `RoutingManager` coordinating rule CRUD with persistence and config generation. All mutations (add, edit, delete, reorder, apply_preset) auto-persist. `write_config()` delegates to `ConfigWriter` from `config/`.

- **`resolve.rs`** — `ConnectionPlanner` selects and orders enabled proxy node candidates per `AutoResolveStrategy`. `LatencySnapshot` stores per-node latency keyed by (subscription_id, node_index).

- **`config/`** — Backend config generation module:
  - `mod.rs` — `ConfigGenerator` trait and `generator_for(backend)` factory.
  - `writer.rs` — `ConfigWriter` takes `AppSettings` + `AppPaths`, calls the right generator, atomic-writes the output JSON (e.g., `{data_dir}/generated/xray.json`). Respects `settings.backend.config_output_dir` override.
  - `v2ray.rs`, `xray.rs`, `singbox.rs` — per-backend `ConfigGenerator` implementations.
  - `common.rs` — shared helpers (outbound tag, test fixtures).

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

- **`manager.rs`** — `ProcessManager` orchestrator. Spawns backend via `tokio::process::Command` with ETXTBSY retry (handles overlayfs race in containers), pipes stdout/stderr through async line readers into shared `Arc<Mutex<LogBuffer>>` + broadcast channel. Graceful stop (SIGTERM → 5s → SIGKILL). Crash recovery with 2s delay, max 3 crashes per minute before Error state. PID file lifecycle. `with_tun(Option<TunRuntime>)` makes start/stop TUN-aware: a `CAP_NET_ADMIN` gate before start, an xray device-wait + `netctl xray-up`, and an `xray-down` safeguard on stop.

- **`privilege.rs`** — TUN capability model. `has_net_admin()` reads a binary's file capabilities via `getcap` (the `caps` crate only handles live process caps, not file xattrs); `grant()` runs a single `pkexec` elevation applying `setcap` to the backend binary and the route helper; `file_caps_supported()` detects a `nosuid` mount and surfaces the manual `setcap` command.

- **`tun.rs`** — `TunRuntime` (backend, iface, addresses, helper path), `helper_path()` resolution (sibling of the running exe, else `$PATH`), `wait_for_device()` polling `/sys/class/net/<iface>`, and `xray_up`/`xray_down` helper invocations.

### `crates/netctl` (`v2ray-rs-netctl`)

Minimal privileged route helper binary (deps: `rtnetlink`, `tokio`, `clap`, `futures`) invoked by the process layer for xray TUN, since xray creates the device but does not program routes on Linux. Subcommands, all idempotent and argument-validated before any netlink call: `xray-up` (link up, assign address, add `0.0.0.0/1` + `128.0.0.0/1` split routes, plus `::/1` + `8000::/1` with `--addr6`), `xray-down` (delete device), and `recover --singbox|--xray` (remove a leftover device; for sing-box flush its `auto_route` table/rules). Kept dependency-light on purpose: it is `setcap`'d with `cap_net_admin` and invokable by any local process. Privileged netns tests are gated behind the `privileged-tests` feature.

### `crates/tray` (`v2ray-rs-tray`)

System tray integration via ksni (StatusNotifierItem protocol):

- **`tray.rs`** — `AppTray` implements `ksni::Tray`. Uses `icon_name()` + `icon_theme_path()` for FreeDesktop theme-aware symbolic icons (DE handles `currentColor` recoloring), with `icon_pixmap()` as ARGB32 fallback. Menu items: Connect/Disconnect toggle, status label, Open Main Window, Quit. `TrayService::spawn()` calls `setup_icon_theme()` and listens for process state events.

- **`icons.rs`** — Embeds 3 symbolic SVGs (disconnected/connected/error). `setup_icon_theme()` creates a temporary FreeDesktop hicolor icon theme directory with the SVGs preserving `currentColor` for theme-aware rendering. `icon_name_for_state()` maps `ProcessState` to icon names. Pixmap rendering (`render_svg()`) replaces `currentColor` with `#DEDDDA` as fallback for DEs without theme lookup.

- **`notification.rs`** — Desktop notifications via `notify-rust` for state changes.

### `crates/ui` (`v2ray-rs-ui`)

GTK4/Relm4 GUI application:

- **`app.rs`** — Main `App` component. `gtk::Paned` (vertical split) between subscriptions and logs pages — no ViewStack/ViewSwitcher. Routing and Settings live in an `adw::PreferencesDialog` opened from a hamburger menu. ToastOverlay wraps content. Connection state and auto-resolve strategy managed via tokio tasks. Status details displayed in a bottom `gtk::ActionBar`.

- **`subscriptions.rs`** — Subscription management page. Features: add/rename/delete subscriptions, toggle nodes, move up/down reordering (subscriptions and nodes), latency testing (TCP ping), sort by latency. Uses `capture_expanded()` to preserve ExpanderRow state across re-renders.

- **`routing.rs`** — Routing rule management with drag-and-drop reordering.

- **`settings.rs`** — App settings editor.

- **`logs.rs`** — Process log viewer.

- **`wizard.rs`** — First-run onboarding wizard.

### Data flow

```
Subscription URL/File → fetch → decode (base64?) → split lines → parse URIs → Vec<ProxyNode>
                                                                                    ↓
AppSettings + ProxyNodes + RoutingRuleSet → ConfigWriter → JSON config file
                                                                                    ↓
                                                         ProcessManager → spawn backend binary
                                                              ↕                     ↕
                                                    state events (broadcast)   log capture (buffer)
```

## Key Patterns

- **Tagged enums for serde**: `ProxyNode`, `TransportSettings`, `RuleMatch`, `SubscriptionSource`, `DnsRuleMatch` all use `#[serde(tag = "...")]` for self-describing JSON/TOML.
- **Atomic file writes**: All persistence goes through `tempfile::NamedTempFile` → `persist()` to avoid corruption.
- **Tests use `tempfile::TempDir`**: Persistence tests create isolated temp directories via `AppPaths::from_paths()` (gated on `cfg(any(test, feature = "test-utils"))`).
- **Workspace dependencies**: All shared deps declared in root `Cargo.toml` `[workspace.dependencies]` and used via `.workspace = true`.
- **Edition 2024**: Uses Rust edition 2024.

## Versioning

When bumping the version, update all three places:
1. `Cargo.toml` — `[workspace.package] version`
2. `pkg/archlinux/PKGBUILD` — `pkgver`
3. `CHANGELOG.md` — new section + link refs at bottom

Then run `cargo check` to regenerate `Cargo.lock`.

## OpenSpec

Feature specifications live in `openspec/specs/` (canonical) and `openspec/changes/` (per-change deltas, archived after implementation). Each change has a proposal, design, delta specs, and task breakdown.
