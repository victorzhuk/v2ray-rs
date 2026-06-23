# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- TUN mode for sing-box and xray: a virtual interface becomes the default route, transparently proxying all system traffic. sing-box self-routes via `auto_route`; xray uses a minimal privileged route helper (`v2ray-rs-netctl`) for the address and split routes. v2ray is excluded (no native TUN).
- `[tun]` settings section (`enabled`, `interface_name`, `mtu`, `address_v4`, `address_v6`, `stack`, `strict_route`, `dns_hijack`, `exclude_routes`) with a TUN preferences page, backend/capability gating, and a system-wide-routing warning.
- One-time TUN privilege grant: `setcap` via `pkexec` grants `cap_net_admin` to the backend binary and the route helper; capabilities are re-checked before each TUN start and re-prompted if lost (e.g. after a package upgrade).
- Route recovery after an unclean shutdown removes a leftover TUN device and flushes stale routes; orphan cleanup now escalates SIGTERM to SIGKILL.

---

## [0.9.0] - 2026-05-31

### Added
- Real Delay latency probes for sing-box with Clash API and xray/v2ray with native ObservatoryService gRPC.
- Per-node Real Delay persistence, badges, sorting, and optional Lowest Latency strategy integration.
- Real Delay settings for test URL, timeout, enable/disable, and Lowest Latency usage.
- Isolated probe backend runners and loopback-only probe configs for sing-box, xray, and v2ray.

### Fixed
- Real Delay availability now refreshes when the backend type or binary path changes.
- Hung or stale Real Delay probes no longer keep the subscriptions UI stuck in testing state.
- Real Delay preferences are wired to `AppSettings.real_delay` and persist immediately.

---

## [0.8.1] - 2026-05-22

### Fixed
- CI: clippy `field_reassign_with_default` errors in `AppSettings` listen-address tests (use struct-init syntax instead of `Default::default()` + reassignment).
- CI: `rustfmt` violations in `crates/core/src/config/v2ray.rs` and `crates/ui/src/preferences/network.rs`.

---

## [0.8.0] - 2026-05-22

### Added
- `listen_address` setting (default `127.0.0.1`) controlling the inbound bind address for the SOCKS5/mixed and HTTP proxies; surfaced as a Settings → Network entry with validation and a one-shot warning toast when set to a non-loopback address.
- Per-backend regression tests asserting UDP on the SOCKS-capable inbound (v2ray/xray: `settings.udp = true`; sing-box: `mixed` type with no `udp_disabled: true`).

### Changed
- Inbound listen address is now configurable (default `127.0.0.1`) instead of hard-coded; applies to both SOCKS and HTTP inbounds for v2ray, xray, and sing-box.
- `ConfigWriter` defensively validates `listen_address` before generation and falls back to `127.0.0.1` (with a `log::warn`) for malformed values, so backends never start with a broken bind string.

---

## [0.7.4] - 2026-04-28

### Fixed
- Flaky test: add process initialization delay in orphan detection test
- AUR publish: update deploy action to v4.1.3 (fixes `bash: --command` error)

---

## [0.7.3] - 2026-04-28

### Fixed
- CI workflow YAML indentation error preventing all jobs from running

---

## [0.7.2] - 2026-04-28

### Fixed
- Clippy lint violations: collapsible `if let` chains, `&PathBuf` → `&Path`, missing `.truncate(true)` on file create
- Code formatting across workspace

### Changed
- CI workflow: corrected YAML indentation for `fmt` job, changed format check from `cargo check` to `cargo fmt --check --all`

---

## [0.7.1] - 2026-04-28

### Fixed
- CI build failure: add `protobuf` package to Arch Linux container dependencies for `prost_build` compilation

---

## [0.7.0] - 2026-04-28

### Added
- Runtime profiles (`--profile`, `V2RAY_RS_PROFILE` env) with isolated storage for production, development, test, and custom profiles
- Per-directory path overrides via CLI flags (`--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`) and matching env vars
- Instance compatibility stamp (`state_dir/instance.json`) — refuses to start when profile, App ID, or schema version mismatch the running build
- Single-instance lock per profile (`flock` on `runtime_dir/v2ray-rs.lock`) — second instance of same profile exits with code 75
- `--reset-instance` flag to wipe a profile's data (production requires `--i-understand`)
- `--install-icons` flag for non-production profiles

### Changed
- PID file moved from `data_dir/backend.pid` to `runtime_dir/backend.pid`
- Generated backend configs moved from `data_dir/generated/` to `runtime_dir/generated/`
- Geodata files moved from `data_dir/geodata/` to `cache_dir/geodata/`
- Latency snapshot moved from `data_dir/latency_snapshot.json` to `state_dir/latency_snapshot.json`
- Legacy files are automatically relocated on first launch (best-effort, logged)
- `V2RAY_RS_DEV` env var is deprecated; use `V2RAY_RS_PROFILE=development` instead
- `AppPaths::from_paths()` deprecated; use `AppPaths::for_profile_in(profile, root)` in tests
- `AppPaths` now exposes `cache_dir()`, `runtime_dir()`, `state_dir()` alongside existing `config_dir()` and `data_dir()`
- Tray icon installation skipped for non-production profiles unless `--install-icons` is set

---

## [0.6.2] - 2026-02-27

### Fixed
- LICENSE file restored to canonical Apache 2.0 text for correct GitHub license detection

---

## [0.6.1] - 2026-02-27

### Changed
- License changed from MIT to Apache 2.0

### Documentation
- CLAUDE.md: documented `models/dns.rs`, `models/resolve.rs`, `config/` module, `AppPaths::new_dev()`, corrected `app.rs` layout description (Paned + Preferences dialog), updated preset list and field descriptions
- OpenSpec specs actualized against implementation: routing-rules, process-lifecycle, dns-configuration, dns-provider-presets, subscription-update, main-window, system-tray
- README.md: updated license badge
- CONTRIBUTING.md: corrected branch name (`master`), project structure (all 5 crates), license reference

---

## [0.6.0] - 2026-02-26

### Changed
- `DnsProtocol` now exposes `default_port()` — eliminates duplicate port table across `singbox.rs`, `preferences.rs`, and `dns.rs`
- `builtin_dns_presets()` refactored via `standard_preset()` helper, reducing ~170 lines to ~30
- `apply_dns_preset()` now drops DNS rules whose `server_tag` no longer exists after the preset replaces servers

### Fixed
- Dead variables `_is_singbox` and `_servers` removed from `preferences.rs`
- DNS providers dialog now closes automatically after a preset is successfully applied
- `DnsValidationError` re-exported from `models` public API

### Tests
- `test_builtin_dns_presets_count` strengthened to `assert_eq!(8)` from `>= 8`
- Added `test_apply_dns_preset_clears_orphaned_rules` to verify preset application passes `validate()`

---

## [0.5.3] - 2026-02-22

### Fixed
- CI formatting: stray blank line in `resolve.rs` after `Default` impl removal

---

## [0.5.2] - 2026-02-22

### Fixed
- Clippy `collapsible_if` in `xray.rs`: collapse nested `if let` into a single `&&` chain
- Clippy `derivable_impls` in `resolve.rs`: replace manual `Default` impl with `#[derive(Default)]`
- Clippy `clone_on_ref_ptr` in `app.rs`: replace `&[node.clone()]` with `std::slice::from_ref`

---

## [0.5.1] - 2026-02-22

### Fixed
- CI formatting check failure: apply `cargo fmt` to all crates

---

## [0.5.0] - 2026-02-22

### Added
- DNS configuration model (`DnsConfig`, `DnsServer`, `DnsProtocol`) with DoH and plain DNS support
- DNS config generation for all backends (v2ray, xray, sing-box) — enabled via Settings when `dns.enabled = true`
- Routing rule groups: rules carry an optional `group` name (set automatically when applying a preset)
- Group-based routing rules UI: rules grouped by preset name in Preferences → Routing, with per-group "Remove" button
- `geodata_dir` forwarded to `ProcessManager` and exported as `V2RAY_LOCATION_ASSET` / `XRAY_LOCATION_ASSET` env vars on process spawn
- Log viewer buffer cap: trims to 10,000 lines to prevent memory growth on long-running sessions

### Changed
- "RU Direct" preset renamed to "RU Bypass" and expanded with private CIDRs and Russian GeoSite categories (media, retail, gov, mail, entertainment, e-commerce, etc.)
- "Popular AI", "Social Networks", and "Bypass LAN" presets merged into single "Proxy Popular" preset
- Country code validation now accepts extended GeoIP tags (GOOGLE, FACEBOOK, NETFLIX, TELEGRAM, TWITTER, etc.) in addition to ISO 3166-1 codes
- GeoSite category validation no longer requires membership in a hardcoded allowlist — accepts any valid lowercase hyphenated string
- Subscription operations (toggle, rename, move, delete, add) now use atomic whole-file saves via `save_subscriptions` instead of granular per-item persistence
- Xray: removed `security: "xtls"` override — xray-core v1.8+ uses `security: "tls"` with the `flow` field for XTLS vision

---

## [0.4.0] - 2026-02-16

### Added
- Connection auto-resolve: automatically select proxy nodes using configurable strategies (list order, lowest latency, random, last successful, geo-aware)
- Connection status bar in main window showing active node, latency, backend, and strategy
- Auto-resolve strategy selector in preferences (Network → Connection)
- Tray tooltip showing connection details (node, latency, backend, strategy, uptime)
- Tray menu label showing active node name when connected
- Latency snapshot persistence (`latency_snapshot.json`) — ping results survive restarts
- Connection state persistence (`connection_state.json`) — tracks active connection metadata
- `ConnectionPlanner` for ordered candidate resolution with fallback through all enabled nodes
- Last-successful-node tracking for reconnection preference

### Changed
- Connect flow iterates through ranked candidates instead of sending all nodes at once
- Process state events now carry `ConnectionMetadata` through the broadcast channel
- Reconnect on settings change only triggers when auto-resolve strategy changes (not on every save)

---

## [0.3.11] - 2026-02-15

### Fixed
- System tray icons missing on some status notifier hosts: install symbolic icons into the user hicolor theme and update icon cache
- App icon missing in some compositors: set window icon name and install icon cache on startup after registering resources

---

## [0.3.7] - 2026-02-12

### Fixed
- App icon not displayed: resized PNG to match 256x256 icon theme directory
- App icon not displayed: update icon cache after runtime install
- App icon not associated with window: added `StartupWMClass` to desktop entry
- Port settings changes had no effect until manual reconnect: auto-reconnect on settings change

---

## [0.3.6] - 2026-02-12

### Fixed
- AUR build failure: disable LTO to fix `ring` assembly linking (`ring_core_0_17_14__*` undefined symbols)

---

## [0.3.5] - 2026-02-12

### Fixed
- Runtime panic "No provider set": install ring crypto provider before reqwest client creation

---

## [0.3.4] - 2026-02-12

### Fixed
- AUR build failure: replaced `aws-lc-rs` crypto backend with `ring` (fixes linker errors in sandboxed builds)

---

## [0.3.3] - 2026-02-12

### Fixed
- CI test failure: ETXTBSY race on overlayfs (retry spawn on `ExecutableFileBusy`)
- AUR publish failure: `github-actions-deploy-aur@v4` tag doesn't exist (pinned to `v4.1.1`)

---

## [0.3.2] - 2026-02-12

### Fixed
- CI test failure: ETXTBSY race in `crash_detection` test (sync script file before exec)
- AUR publish not triggering after release (GITHUB_TOKEN events don't trigger other workflows)

### Changed
- AUR publish workflow now called directly from release via `workflow_call`

---

## [0.3.1] - 2026-02-12

### Fixed
- CI build failure: added `base-devel` to Arch container packages (fixes `glib-sys` compilation)
- Release build failure: committed `Cargo.lock` for reproducible `--locked` builds

---

## [0.3.0] - 2026-02-12

### Changed
- CI/Release builds inside Arch Linux container (rolling glib >= 2.84 for GNOME 48)
- Updated GitHub Actions: actions/checkout v4 -> v6
- Bumped toml 0.8 -> 1, nix 0.29 -> 0.31, resvg 0.45 -> 0.47, reqwest 0.12 -> 0.13
- Added --locked flag to release build
- Removed .deb packaging from release workflow

### Added
- AUR publishing workflow (auto-publishes PKGBUILD on GitHub release)

### Fixed
- CI failure from cargo fmt violations
- Release build failure (glib 2.84 not available on Ubuntu 24.04)
- All clippy warnings (derivable_impls, collapsible_if, needless_borrows, manual_ok)

---

## [0.2.0] - 2026-02-12

### Added
- GTK4/Relm4 GUI application with libadwaita (`v2ray-rs-ui`)
  - Main window with ViewSwitcher: Subscriptions, Routing, Logs, Settings
  - Connect/Disconnect button with process state tracking
  - Subscription management: add, rename, delete, toggle, reorder
  - Node latency testing (TCP ping) with sort-by-latency
  - Routing rule editor with drag-and-drop reordering and preset dialogs
  - Settings editor for backend, ports, update intervals, language
  - Process log viewer
  - First-run onboarding wizard
  - Toast notifications for status messages
  - i18n support via gettext
- System tray integration via ksni (`v2ray-rs-tray`)
  - FreeDesktop symbolic icons (connected/disconnected/error)
  - Connect/Disconnect toggle, status label, Open Main Window, Quit
  - Desktop notifications on state changes
- Workspace expanded to five crates: core, subscription, process, tray, ui
- Arch Linux PKGBUILD packaging
- Desktop entry file (`v2ray-rs.desktop`)
- Shared config test fixtures (`config/test_fixtures.rs`)
- Shared `outbound_tag()` helper (`config/common.rs`)
- Bounded concurrent TCP pings via `Semaphore` (max 50)
- PID reuse protection via `/proc/PID/cmdline` verification
- Signal-aware exit classification (SIGINT/SIGTERM/SIGKILL vs real crashes)
- Tracked log capture tasks with abort-on-stop

### Changed
- `geodata::needs_update()` accepts `Duration` instead of raw `u64`
- `subscription::update::reconcile_nodes()` delegates to `reconcile_with_counts()`
- Extracted `parse_url_transport()` and `parse_url_tls()` from duplicated parser code
- HTTP user-agent now uses `CARGO_PKG_VERSION` instead of hardcoded `"v2ray-rs/0.1"`
- Replaced 30+ `let _ = persist()` sites with `if let Err(e)` + `log::error!`
- Replaced `Mutex::lock().unwrap()` with `if let Ok(guard)` in UI statics
- Added `#[serde(skip_serializing_if = "Option::is_none")]` to all `Option<T>` proxy fields
- Changed `SubscriptionNode::last_latency_ms` from `#[serde(skip)]` to `#[serde(skip_serializing, default)]`
- Extracted named constants for timeouts, window size, channel capacity, retry limits, and ruleset URLs
- sing-box config uses `rule_set` (remote binary `.srs`) instead of deprecated `geoip`/`geosite` databases

### Fixed
- Panic on `target.parent().unwrap()` in geodata download path
- Potential UTF-8 panic in tray error message truncation
- PID file `kill(pid, 0)` race condition on PID reuse
- Untracked `tokio::spawn` tasks for log capture leaked on process stop
- All signal exits (130/137/143) incorrectly counted toward crash threshold

### Removed
- Dead `status_bar.rs` module (127 lines, unused)
- Duplicate `outbound_tag()` from `v2ray.rs` and `singbox.rs`
- Duplicate test fixture functions from individual config test modules
- Legacy PNG tray icons (replaced by symbolic SVGs)

## [0.1.0] - 2026-02-11

### Added
- Initial release
- Core architecture with Clean Layered Design
- Workspace with three crates:
  - `v2ray-rs-core`: Domain models, persistence, backend detection, routing, config generation
  - `v2ray-rs-subscription`: Subscription fetching and URI parsing
  - `v2ray-rs-process`: Tokio-based process lifecycle management
- Initial project structure and workspace configuration
- Core domain models for proxy protocols (VLESS, VMess, Shadowsocks, Trojan)
- Subscription management with URL and file sources
- Routing rule engine with GeoIP, GeoSite, domain pattern, and IP CIDR matching
- XDG-compliant file persistence with atomic writes
- Backend detection for v2ray, xray, and sing-box
- Geodata manager for GeoIP/GeoSite database updates
- Backend config generation for v2ray, xray, and sing-box
- Async process lifecycle management with crash recovery
- Circular log buffer for process output capture
- Graceful shutdown with SIGTERM/SIGKILL handling
- Built-in routing presets (RU Direct, CN Direct, Block Ads)
- Validation framework for country codes, IP CIDRs, and domain patterns
- Comprehensive test coverage with isolated environments

### Features

**Core (`v2ray-rs-core`)**
- Domain models: `ProxyNode`, `Subscription`, `RoutingRuleSet`, `AppSettings`
- Transport and TLS settings for proxy nodes
- Rule matching: GeoIP, GeoSite, domain pattern, IP CIDR
- Rule actions: Proxy, Direct, Block
- XDG-compliant persistence (settings.toml, subscriptions.json, routing.json)
- Atomic file writes via `tempfile::NamedTempFile`
- Backend binary detection (v2ray, xray, sing-box)
- Backend version extraction and validation
- Geodata management with GitHub release fetching
- Config generation for all three backends
- Routing manager with CRUD operations and rule validation
- Built-in routing presets

**Subscription (`v2ray-rs-subscription`)**
- HTTP subscription fetching with rustls-tls
- Local file subscription reading
- Base64-encoded content decoding
- Multi-line subscription parsing
- URI parsers for VLESS, VMess, Shadowsocks, Trojan

**Process (`v2ray-rs-process`)**
- Async process spawning via `tokio::process::Command`
- Process state machine (Stopped, Starting, Running, Stopping, Error)
- State change events via tokio broadcast channel
- Circular log buffer (10K lines)
- Async stdout/stderr capture and buffering
- PID file lifecycle management
- Orphaned process detection and cleanup
- Graceful stop with configurable timeout
- Crash recovery with backoff (max 3 crashes/minute)

**Testing**
- Comprehensive test coverage
- Isolated temp directory testing for persistence
- Table-driven tests for validation
- Process lifecycle tests
- URI parsing tests

**Infrastructure**
- Rust Edition 2024
- Workspace dependencies management
- Makefile for build automation
- GitHub Actions CI configuration
- CLAUDE.md development guidelines
- OpenSpec workflow setup

---

[Unreleased]: https://github.com/victorzhuk/v2ray-rs/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.8.1...v0.9.0
[0.8.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.4...v0.8.0
[0.7.4]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.2...v0.7.0
[0.6.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.11...v0.4.0
[0.3.11]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.7...v0.3.11
[0.3.7]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/victorzhuk/v2ray-rs/releases/tag/v0.1.0
