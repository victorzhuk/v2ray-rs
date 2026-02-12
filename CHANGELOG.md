# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/victorzhuk/v2ray-rs/releases/tag/v0.1.0
