# Proposal: Core Data Models & Persistence Layer

## Why

The application needs foundational data structures to represent proxy nodes, subscriptions, routing rules, user preferences, and backend configurations. Without these core models, no other feature (subscription import, config generation, routing) can be built. These models serve as the shared vocabulary across all modules.

## What Changes

- Define Rust structs for proxy protocols: VLESS, VMess, Shadowsocks, Trojan
- Define subscription model (URL, nodes list, last updated, auto-update interval)
- Define routing rule model (GeoIP, GeoSite, domain/IP-based rules with actions)
- Define backend configuration model (selected backend, binary path, config output path)
- Define application settings/preferences model
- Implement serde serialization/deserialization for persistent storage (JSON/TOML)
- Establish XDG-compliant data/config directory structure

## Capabilities

### New Capabilities
- `proxy-node-models`: Rust types for all supported proxy protocols (VLESS, VMess, Shadowsocks, Trojan) with validation
- `app-persistence`: XDG-compliant config/data storage with serde-based serialization

### Modified Capabilities
(none)

## Impact

- New crate/module: `models` or `core`
- Dependencies: serde, serde_json, toml, dirs/directories
- All other changes depend on these data models
