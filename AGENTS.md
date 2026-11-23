# AGENTS.md

This file provides guidance to coding agents working in this repository.

## What This Is

A Linux desktop GUI wrapper for v2ray/xray/sing-box CLI proxy tools. The app manages subscriptions, generates config files, handles process lifecycle, and provides geo-routing rules — all without implementing any protocol logic. The protocol work is delegated entirely to the system-installed CLI binaries.

UI: Relm4 (GTK4) with libadwaita. Nine crates: core, subscription, process, netctl, tray, ui, run, tun-session, antidpi.

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

Rust workspace with nine crates:

### `crates/core` (`v2ray-rs-core`)

Domain models and infrastructure:

- **`models/`** — All domain types, organized by concern:
  - `proxy.rs` — `ProxyNode` enum (Vless/Vmess/Shadowsocks/Trojan) with per-protocol config structs and transport/TLS settings. Uses `#[serde(tag = "protocol")]` for tagged serialization.
  - `subscription.rs` — `Subscription` and `SubscriptionSource` (URL or file). Subscriptions own a `Vec<SubscriptionNode>` where each node can be individually enabled/disabled.
  - `routing.rs` — `RoutingRuleSet` with ordered `RoutingRule`s. Match conditions: GeoIP, GeoSite, Domain pattern, IP CIDR. Actions: Proxy/Direct/Block. Rule ordering matters (priority by position). CRUD with validation: `add_validated()`, `add_at()`, `edit_rule()`, `remove()`, `move_rule()`, `apply_preset()`.
  - `validation.rs` — `ValidationError` enum and validators for country codes (extended: ISO 3166-1 + GeoIP tags like GOOGLE/NETFLIX), IP CIDR, domain patterns (wildcard syntax), GeoSite categories.
  - `presets.rs` — `Preset` struct and `builtin_presets()` returning 4 presets: RU Bypass, CN Direct, Proxy Popular, Block Ads.
  - `settings.rs` — `AppSettings` with backend config, proxy ports, `listen_address` (default `127.0.0.1`, validated as an IPv4/IPv6 literal via `AppSettings::validate_listen_address`), update intervals, language, tray behavior, `auto_resolve_strategy`, `last_success: Option<LastSuccessMetadata>`, `dns: DnsConfig`, `tun: TunConfig`, and `logging: LoggingSettings` (backend log level, xray/v2ray connection log). Serializes to TOML.
  - `tun.rs` — `TunConfig` (enabled, interface_name, mtu, address_v4, address_v6, stack, strict_route, dns_hijack, exclude_routes, exclude_processes, exclude_domains) with `validate()` and a `TunConfigWire` forward-compat wire struct. `TunStack` (System/Gvisor/Mixed) and `DnsHijackMode` (Hijack/Native/Disabled) enums serialize to backend literals. Default is TUN disabled.
  - `dns.rs` — `DnsConfig` (enabled, strategy, servers, rules, fakeip, disable_cache, client_subnet, hosts, use_custom_rules). `DnsServerConfig` (tag, protocol, address, port, detour). `DnsProtocol` enum (UDP/TCP/DoH/DoT/DoQ/H3) with `server_address()` and `default_port()`. `DnsStrategy` (PreferIpv4/PreferIpv6/Ipv4Only/Ipv6Only). `DnsRule`, `FakeIpConfig`, `HostOverride`. `builtin_dns_presets()` returns 8 presets (Cloudflare, Google, AdGuard, Quad9, Ali DNS, Yandex DNS variants). `DnsConfigWire` for backward-compat migration from the old two-field format.
  - `resolve.rs` — `AutoResolveStrategy` enum (ListOrder/LowestLatency/Random/LastSuccessful/GeoAware). `ConnectionMetadata` for active connection details. `LastSuccessMetadata` and `LatencySample`.

- **`persistence/`** — XDG-compliant file storage via `directories` crate. Settings in TOML (`~/.config/v2ray-rs/settings.toml`), subscriptions and routing rules in JSON (`~/.local/share/v2ray-rs/`). Also persists `latency_snapshot.json`, a versioned `tun_session.v1.json` TUN ownership journal (written at TUN connect, removed on clean stop; its presence at startup triggers a route-recovery pass), and `logs/` under `state_dir` (`$XDG_STATE_HOME/<qualifier>`, falling back to `data_dir/state`), plus custom presets under `data_dir/presets`. Uses atomic writes via `tempfile::NamedTempFile` + persist. Directories created with 0o700 permissions. `AppPaths::new_dev()` isolates dev mode under a `v2ray-rs-dev` qualifier. `AppPaths::from_paths()` is gated on `cfg(any(test, feature = "test-utils"))`.

- **`backend.rs`** — Detects installed v2ray/xray/sing-box binaries by checking well-known paths (`/usr/bin/`, `/usr/local/bin/`) and `$PATH` via `which`. Validates executability, extracts version strings. Provides install guidance strings per backend.

- **`geodata.rs`** — `GeodataManager` for GeoIP/GeoSite database management. Handles metadata (last check timestamp, versions), per-backend geodata: `.dat` whole-file downloads for v2ray/xray, and per-tag binary rule-set (`.srs`) files under `cache_dir/geodata/rule-sets/` for sing-box (only the tags referenced by current routing rules; stale legacy `.db` files are deleted on refresh). Update checks (`needs_update()`) and blocking downloads from v2fly/SagerNet GitHub. Feature-gated `geodata-fetch` for reqwest blocking client.

- **`routing_manager.rs`** — `RoutingManager` coordinating rule CRUD with persistence and config generation. All mutations (add, edit, delete, reorder, apply_preset) auto-persist. `write_config()` delegates to `ConfigWriter` from `config/`.

- **`resolve.rs`** — `ConnectionPlanner` selects and orders enabled proxy node candidates per `AutoResolveStrategy`. `LatencySnapshot` stores per-node latency keyed by `ConnectionNodeRef` — `(subscription_id, node_id)` for subscription nodes.

- **`config/`** — Backend config generation module:
  - `mod.rs` — `ConfigGenerator` trait, `ResolvedNodeSet` (primary node plus detour nodes), and `generator_for(backend)` factory.
  - `dns_plan.rs` — `ResolvedDnsPlan` resolves protocol downgrades, bootstrap domains, hosts, and split-horizon detours once; `v2ray`/`xray`/`sing-box` and Effective DNS lower from it.
  - `writer.rs` — `ConfigWriter` takes `AppSettings` + `AppPaths` plus a `ResolvedNodeSet`, calls the right generator, atomic-writes the output JSON (e.g., `{runtime_dir}/generated/xray.json`). Respects `settings.backend.config_output_dir` override.
  - `v2ray.rs`, `xray.rs`, `singbox.rs` — per-backend `ConfigGenerator` implementations.
  - `common.rs` — shared helpers (outbound tag, test fixtures).

- **`state.rs`** — Pure `ProcessState`, `ProcessEvent`, and `TransitionError` used by process, tray, and UI without async or OS dependencies.

- **`log_buffer.rs`** — Circular `LogBuffer` (VecDeque, 10K lines max) with `LogLine` (source: Stdout/Stderr, content). Methods: push, last_n, search (case-insensitive).

### `crates/subscription` (`v2ray-rs-subscription`)

Depends on `v2ray-rs-core` and `v2ray-rs-process`, which manages ephemeral backends for Real Delay and availability probing. Handles subscription fetching and URI parsing:

- **`fetch.rs`** — HTTP fetching (reqwest with rustls-tls, 30s connect / 60s total timeout) and local file reading. `decode_subscription_content()` handles both base64-encoded and plaintext subscription responses, splitting into individual URI lines.

- **`parser.rs`** — Parses proxy URIs (`vless://`, `vmess://`, `ss://`, `trojan://`) into `ProxyNode` variants. VMess uses base64-encoded JSON. Shadowsocks uses base64-encoded `method:password` userinfo. VLESS and Trojan use standard URL parsing.

- **`ping.rs`** — TCP connect latency testing. `tcp_ping()` measures TCP connection time with 5s timeout. `ping_nodes()` pings all nodes concurrently via `tokio::spawn`.

### `crates/process` (`v2ray-rs-process`)

Depends on `v2ray-rs-core`, `tokio`, and `nix`. Async process lifecycle management:

- **`state.rs`** — `StateManager` wraps the core state types with a tokio broadcast channel for event subscribers; the process and event types are re-exported from `v2ray-rs-core`.

- **`session/`** — Headless connection supervisor moved out of the UI: candidate failover, DNS pinning and lookups, connected-network snapshots, health monitoring (plain HTTP proxy probe, no TLS client), DNS failure windows, log batching, backend warning and failure classification. It emits `SessionEvent`s and holds no GTK dependency.

- **`log_buffer`** — Re-exported from `v2ray-rs-core` for existing callers.

- **`pid.rs`** — `PidFile` for writing/reading/removing PID files. `check_and_kill_orphaned()` detects stale processes from previous runs using `kill(pid, 0)` signal probe.

- **`manager.rs`** — `ProcessManager` orchestrator. Spawns backend via `tokio::process::Command` with ETXTBSY retry (handles overlayfs race in containers), pipes stdout/stderr through async line readers into shared `Arc<Mutex<LogBuffer>>` + broadcast channel. Graceful stop (SIGTERM → 5s → SIGKILL). Crash recovery with 2s delay, max 3 crashes per minute before Error state. PID file lifecycle. `with_tun(Option<TunRuntime>)` makes start/stop TUN-aware: a `CAP_NET_ADMIN` gate before start, an xray device-wait + `netctl xray-up`, and an `xray-down` safeguard on stop.

- **`privilege.rs`** — TUN capability model. `has_net_admin()` reads a binary's file capabilities via `getcap`; `grant()` runs a single `pkexec` elevation applying `setcap` to the backend binary and the route helper, and also sets root ownership + the setuid bit on the `v2ray-rs-run` wrapper when it is present; `file_caps_supported()` detects a `nosuid` mount and surfaces the manual `setcap` command.

- **`tun.rs`** — `TunRuntime` (backend, iface, addresses, helper path, optional bypass UID), `helper_path()`/`run_path()` resolution via the pure `pick_bin()` (capability-capable sibling of the running exe, else the relocated copy under `RELOCATE_DIR`, else the capability-less sibling, else `$PATH`), `wait_for_device()` polling `/sys/class/net/<iface>`, and `xray_up`/`xray_down` helper invocations. `bypass_uid`, when set, is passed to `netctl xray-up --bypass-uid` so traffic from the `v2ray-rs-bypass` user skips the tunnel.

### `crates/run` (`v2ray-rs-run`)

Minimal setuid-root wrapper binary (deps: `libc`). It resolves the `v2ray-rs-bypass` system user, drops real/effective/saved UID and GID to it, clears supplementary groups, sanitizes the environment, and `execvp`s the requested command. Used only for the xray TUN *Run with bypass* action so app-launched tools can exit the tunnel by UID via the `ip rule uidrange` policy rule.

### `crates/netctl` (`v2ray-rs-netctl`)

Minimal privileged route helper binary (deps: `rtnetlink`, `tokio`, `clap`, `futures`) invoked by the process layer for xray TUN, since xray creates the device but does not program routes on Linux. Automatic commands require `--journal` and `--session-id`; `reserve` establishes kernel ownership before xray spawn. Legacy operator cleanup requires explicit `--force-legacy`. Subcommands, all idempotent and argument-validated before any netlink call: `xray-up` (link up, assign address, add a device-bound default route in table 2023 and policy rules at priorities 9000–9002, with IPv6 equivalents when configured; installs a `uidrange` policy rule at priority 8998 when `--bypass-uid` is given; repeatable `--exclude <CIDR>` installs priority-8997 rules sending those prefixes to table `main`, replaced on every `xray-up`), `xray-down` (remove exact journaled resources and the verified device), and `recover --singbox|--xray` (verify xray ownership before exact cleanup; refuse automatic sing-box cleanup without ownership evidence). Kept dependency-light on purpose: it is `setcap`'d with `cap_net_admin` and restricted to the `v2ray-rs` group with `root:v2ray-rs` ownership and mode `0750`. Privileged netns tests are gated behind the `privileged-tests` feature.


### `crates/tun-session` (`v2ray-rs-tun-session`)

Shared versioned TUN journal schema and bounded owner-only atomic I/O. Used by core and netctl so the privileged helper does not depend on the full domain crate. Recovery verifies session, boot, network namespace, device identity, and exact recorded route/rule objects. Unknown or legacy ownership requires operator recovery.

### `crates/antidpi` (`v2ray-rs-antidpi`)

Standalone DPI-circumvention engine binary (NFQUEUE desync strategies; deps: `nfq`, `clap`; L3/L4 packet parsing is hand-rolled in `src/packet.rs`). Subcommands: `check --strategy …` (parses strategy chains, then probes capabilities), `run --queue N --strategy … [--hosts F] [--no-fw]` (installs firewall rules, runs the NFQUEUE loop, removes rules on SIGTERM; `--no-fw` when rules are managed externally), and one-shot `install-rules --queue N` / `remove-rules [--sweep]`. The process crate drives it as an external binary — never a crate dependency: `install-rules` before the backend spawn, the long-lived engine child after route setup, `remove-rules` on stop (`crates/process/src/antidpi.rs`). Settings live in `[antidpi]` of `settings.toml`; an empty strategy list delegates to the crate's built-in `DEFAULT_CHAINS`. Technique catalog and strategy DSL: `docs/antidpi-registry.md`. It is `setcap`'d with `cap_net_admin,cap_net_raw`, restricted to the `v2ray-rs` group with `root:v2ray-rs` ownership and mode `0750`. Privileged netns tests are gated behind the `privileged-tests` feature.

### `crates/tray` (`v2ray-rs-tray`)

System tray integration via ksni (StatusNotifierItem protocol):

- **`tray.rs`** — `AppTray` implements `ksni::Tray`. `icon_name()` returns an empty string, and `icon_pixmap()` supplies state-specific ARGB32 icons. Menu items: Connect/Disconnect toggle, status label, Open Main Window, Quit. `TrayService::spawn()` listens for process state events without setting up an icon theme.

- **`icons.rs`** — Embeds 3 symbolic SVGs (disconnected/connected/error). `render_svg()` replaces `currentColor` with `#DEDDDA` and renders ARGB32 pixmaps. Startup does not install icons or update an icon cache.

- **`notification.rs`** — Desktop notifications via `notify-rust` for state changes.

### `crates/ui` (`v2ray-rs-ui`)

GTK4/Relm4 GUI application:

- **`app/`** — Main `App` component split into `mod.rs` (state, update loop), `bootstrap.rs` (startup, recovery, tray/icon wiring), `session_bridge.rs` (connect/failover/reconnect policy), `config_regen.rs` (coalesced regeneration), and `dialogs.rs` (quit/disconnect/toast plans). `gtk::Paned` (vertical split) between subscriptions and logs pages — no ViewStack/ViewSwitcher. Routing and Settings live in an `adw::PreferencesDialog` opened from a hamburger menu. ToastOverlay wraps content. Connection state and auto-resolve strategy managed via tokio tasks. Status details displayed in a bottom `gtk::ActionBar`.

- **`connection.rs`** — Thin Relm4 bridge that starts `v2ray_rs_process::session::spawn` and forwards `SessionEvent`s to `AppMsg`.

- **`workspace.rs`** — Authoritative `WorkspaceStore`: in-memory `Arc` snapshots of settings, subscriptions, manual nodes, routing rules, and latency data, with serialized persistence on one background writer thread.

- **`subscriptions/`** — Subscription management page split into `mod.rs` (component and mutations), `rows.rs` (expander/node rendering), `dialogs.rs` (add/rename/delete and forms), and `probes.rs` (latency and real-delay runs). Features: add/rename/delete subscriptions, toggle nodes, move up/down reordering (subscriptions and nodes), latency testing (TCP ping), sort by latency. Uses `capture_expanded()` to preserve ExpanderRow state across re-renders.

- **`routing.rs`** — Routing rule management with drag-and-drop reordering.

- **`preferences/`** — App settings editor. `dns/` is split into `mod.rs` (page state and mutations), `servers.rs`, `rules.rs`, `hosts.rs`, and `effective.rs` (Effective DNS rendering and backend notes).

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

When bumping the version, update all four places:
1. `Cargo.toml` — `[workspace.package] version`
2. `pkg/archlinux/PKGBUILD` — `pkgver`
3. `pkg/archlinux/bin/PKGBUILD` — `pkgver`
4. `CHANGELOG.md` — new section + link refs at bottom

Then run `cargo check` to regenerate `Cargo.lock`.

Release artifacts, the glibc/GTK floor, and the AUR publish order: `docs/RELEASE.md`.

## OpenSpec

Feature specifications live in `openspec/specs/` (canonical) and `openspec/changes/` (per-change deltas, archived after implementation). Each change has a proposal, design, delta specs, and task breakdown.
