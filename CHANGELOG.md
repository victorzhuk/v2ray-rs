# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

---

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

[Unreleased]: https://github.com/victorzhuk/v2ray-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/victorzhuk/v2ray-rs/releases/tag/v0.1.0
