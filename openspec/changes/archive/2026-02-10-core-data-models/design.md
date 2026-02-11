# Design: Core Data Models & Persistence Layer

## Context

This is a greenfield Rust project. No existing code — these models are the first code written. All subsequent features (subscription import, config generation, routing, process management, UI) depend on these types. The app targets Linux desktops.

## Goals / Non-Goals

**Goals:**
- Define a clean, extensible type hierarchy for proxy protocols
- Use Rust enums for protocol variants (type-safe, exhaustive matching)
- Serde-based serialization for both config persistence (TOML) and data storage (JSON)
- XDG Base Directory compliance for config (~/.config/v2ray-rs/) and data (~/.local/share/v2ray-rs/)
- Validation at construction time (newtype patterns, builder where needed)

**Non-Goals:**
- No protocol implementation or network code
- No UI types or view models (those belong to the UI layer)
- No config generation logic (separate change)

## Decisions

1. **Single `core` crate vs workspace**: Use a Cargo workspace with a `core` library crate for models. This allows clean dependency boundaries as the project grows. Other crates (subscription, config, ui) depend on `core`.

2. **Enum-based proxy node model**: Use `enum ProxyNode { Vless(VlessConfig), Vmess(VmessConfig), Shadowsocks(SsConfig), Trojan(TrojanConfig) }` with serde tagged union. This gives exhaustive match checking and clear separation of protocol-specific fields.

3. **TOML for app config, JSON for data**: App settings (backend choice, proxy ports, UI prefs) stored as TOML in config dir. Subscriptions and node data stored as JSON in data dir. TOML is more human-readable for settings; JSON better for structured data.

4. **`directories` crate for XDG paths**: Use the `directories` crate (mature, well-maintained) rather than hand-rolling XDG logic.

5. **UUIDs for entity IDs**: Use `uuid` crate for subscription and rule IDs to avoid ordering/collision issues.

## Risks / Trade-offs

- [Enum extensibility] Adding new proxy protocols requires modifying the enum → Mitigated by serde's `#[serde(other)]` for forward-compatible deserialization
- [Schema evolution] Changing stored data format breaks existing user configs → Mitigated by versioning the config format with a `version` field
