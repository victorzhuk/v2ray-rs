## Context

- Scope: what a stored DNS entry means when it is read back or emitted — the model half of the DNS honesty work, with no generator-rule or UI coupling.
- `DnsConfig` is `#[serde(from = "DnsConfigWire")]` (`crates/core/src/models/dns.rs`). The wire struct declares `enabled: bool` with no default and `servers` with `#[serde(default)]` (i.e. an empty vector), and `From<DnsConfigWire>` substitutes `DnsConfig::default().servers` whenever `servers` is empty and the legacy `remote`/`domestic` fields are absent.
- `load_settings` maps a deserialize error to `CorruptConfig`, and `load_settings_or_default` answers with `AppSettings::default()` (`crates/core/src/persistence/settings.rs`).
- `dns_server_address_for_backend` calls `effective_protocol.server_address(&server.address, server.port)`, where `effective_protocol` is `server.protocol.effective_for_backend(backend_type)` (`crates/core/src/config/v2ray.rs`). `DnsProtocol::server_address` omits the port only when it equals the effective protocol's default (`dns.rs`), so a fallback protocol inherits the original protocol's explicit port.
- The DNS dialog stores `port = None` when the spin equals the selected protocol's default (`crates/ui/src/preferences/dns.rs`), so a leftover explicit port comes from a protocol change made after the port was set, or from a hand-edited settings file.
- `DnsConfig::serialization` always writes `servers`, so a saved file round-trips through the new wire shape.

## Goals / Non-Goals

**Goals:**
- A partially written `[dns]` table loads with everything else in the file intact, and the user's explicit server list is preserved exactly as written.
- A generated DNS address names the port the effective protocol actually dials.

**Non-Goals:**
- Rejecting a config saved with DNS enabled and zero servers: `DnsConfig::validate` keeps reporting `NoServers` for that state.
- Changing which protocols downgrade to which, or the downgrade warning.

## Decisions

- **`enabled` gets `#[serde(default)]`; `servers` becomes `Option<Vec<DnsServerConfig>>`.** `Some(v)` is taken as written, including an empty list; `None` runs the legacy migration and falls back to the defaults. This is the only shape that distinguishes an absent key from an empty one with serde alone.
- **Downgrade drops the port.** When `effective_for_backend` differs from the configured protocol, format with `port = None` so `server_address` emits the effective protocol's default port (or omits it). The existing downgrade warning stays.

## Risks / Trade-offs

- [Existing files carrying `servers = []` written while the old migration was in effect] → they now load empty instead of with the defaults; with DNS disabled nothing changes at runtime besides the TUN derived plane, which does not use user servers.
- [A hand-edited file relying on an empty `servers` key to mean "give me the defaults"] → the key now means what it says; the defaults remain one absent key away.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, lenses **spec, quality**, estimate **1.5 h** (5 open tasks).
Floor: `make lint && TEST_TIMEOUT=10m make test`. No red stage anywhere: this is a Rust change and the run has no
test-writer agent for Rust, so each chunk closes against a stated `NO-RED-WAIVER` and the tests are the coder's first
`codeTasks`. Both chunks run in parallel worktrees; their file sets are disjoint.

### dns-wire — tasks 1.1, 1.2 — shard `wire` — coder `rust-coder`
Files: `crates/core/src/models/dns.rs`, `crates/core/src/persistence/settings.rs`, `CHANGELOG.md`.

| site | anchor | change |
|---|---|---|
| `models/dns.rs` — `DnsConfigWire` | `struct DnsConfigWire {` | `#[serde(default)]` on `enabled`; `servers: Option<Vec<DnsServerConfig>>` |
| `models/dns.rs` — `From<DnsConfigWire> for DnsConfig` | `impl From<DnsConfigWire> for DnsConfig {` | `Some(v)` taken as written; legacy migration or defaults only on `None` |
| `models/dns.rs` — tests | `fn test_backward_compat_migration_from_legacy() {` | add the three wire tests |
| `persistence/settings.rs` — tests | `fn test_corrupt_config_falls_back() {` | add `test_load_settings_partial_dns_preserves_other_sections` |
| `CHANGELOG.md` | `## [Unreleased]` | user-facing entry for the partial `[dns]` load |

Contract: six wire states (`enabled-absent-false`, `enabled-as-written`, `servers-empty-as-written`,
`servers-nonempty-as-written`, `servers-legacy-migrated`, `servers-defaults-substituted`) and two loader states
(`loaded-preserved`, `corrupt-config`); every state seeded only through `toml::from_str::<DnsConfig>` (wire) or a
written `settings.toml` read back through `load_settings` (loader). Forbidden: enabling DNS from an absent key,
substituting defaults when a `servers` key is present in any form, and any `skip_serializing_if` on
`DnsConfig.servers` — `DnsConfig` derives `Serialize` directly, so an omitted key would reload as the defaults.
The loader test must not go through `load_settings_or_default`, which hides `CorruptConfig`.

Verify: `make test-core && timeout 3m cargo clippy -p v2ray-rs-core --all-targets -- -D warnings`

### dns-port — task 1.3 — shard `addr` — coder `rust-coder`
Files: `crates/core/src/config/v2ray.rs`.

| site | anchor | change |
|---|---|---|
| `config/v2ray.rs` — `dns_server_address_for_backend` | `fn dns_server_address_for_backend(server: &DnsServerConfig, backend: V2rayFamilyBackend) -> String {` | pass `None` where the port goes when `effective_protocol != server.protocol`; keep the `log::warn!` branch |
| `config/v2ray.rs` — tests | `fn test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols() {` | add the downgrade and native-DoH port tests |

Contract: `address-doh-default-port`, `address-explicit-port-kept`, `native-protocol-unchanged`. Forbidden: a
downgraded address carrying the original protocol's port, and a native protocol losing an explicit non-default port
(xray DoT `dns.google:8530` stays `tls://dns.google:8530`). `attach_exclude_domains` reads its target address
through the same function, so it needs no separate edit.

Verify: `make test-core && timeout 3m cargo clippy -p v2ray-rs-core --all-targets -- -D warnings`

### run-verification — tasks 2.1, 2.2 — no chunk
Task 2.1 is the floor above, run once after both chunks merge. Task 2.2 (dev-profile copy of `settings.toml` with
`enabled` removed from `[dns]`, app started, DNS page showing DNS disabled) is a manual acceptance step recorded in
the run report; nothing in the run can fail on it.

## Plan appendix

```json
{
  "v": 2,
  "change": "load-partial-dns-settings",
  "baseSha": "331a0908b53388258b6aac80d393148d46acfacc",
  "generatedAt": "2026-09-17T14:52:10.463Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "dns-wire",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "dns-wire-partial-load",
      "shard": "wire",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "DnsConfigWire",
          "anchor": "struct DnsConfigWire {",
          "change": "Add #[serde(default)] to enabled: bool and change servers: Vec<DnsServerConfig> to servers: Option<Vec<DnsServerConfig>>"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "<DnsConfig as From<DnsConfigWire>>::from",
          "anchor": "impl From<DnsConfigWire> for DnsConfig {",
          "change": "Take Some(servers) as written including empty Vec, running legacy migration or defaulting only on None"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "tests",
          "anchor": "fn test_backward_compat_migration_from_legacy() {",
          "change": "NEW: Add unit tests verifying [dns] without enabled defaults enabled=false, servers=[] roundtrips empty, and absent servers gets defaults"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/persistence/settings.rs",
          "symbol": "tests",
          "anchor": "fn test_corrupt_config_falls_back() {",
          "change": "NEW: Add test_load_settings_partial_dns_preserves_other_sections verifying missing enabled in [dns] preserves other settings without CorruptConfig"
        },
        {
          "task": "1.1",
          "file": "CHANGELOG.md",
          "symbol": "[Unreleased]",
          "anchor": "## [Unreleased]",
          "change": "repo policy: add the user-facing entry for the partial [dns] table load and the downgraded DoH port (Fixed/Changed under [Unreleased])"
        }
      ],
      "contract": {
        "states": [
          "enabled-absent-false",
          "enabled-as-written",
          "servers-empty-as-written",
          "servers-nonempty-as-written",
          "servers-legacy-migrated",
          "servers-defaults-substituted",
          "loaded-preserved",
          "corrupt-config"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "spec delta 'Partial DNS settings load without data loss' scenario Missing enabled key; tasks.md 1.1; wire field today has no default at crates/core/src/models/dns.rs:201",
            "input": "TOML [dns] without `enabled` key (servers in any form)",
            "state": "enabled-absent-false"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/models/dns.rs:237 (enabled: wire.enabled carried verbatim)",
            "input": "TOML [dns] with `enabled = <bool>`",
            "state": "enabled-as-written"
          },
          {
            "effect": "set",
            "evidence": "design.md decision 'Some(v) is taken as written, including an empty list'; today empty is replaced at crates/core/src/models/dns.rs:237",
            "input": "`servers = []` present (even alongside legacy remote/domestic)",
            "state": "servers-empty-as-written"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/models/dns.rs:237-238 (current non-empty branch unchanged)",
            "input": "`servers` present and non-empty",
            "state": "servers-nonempty-as-written"
          },
          {
            "effect": "set",
            "evidence": "crates/core/src/models/dns.rs:239-245; guarded by test_backward_compat_migration_from_legacy (dns.rs:689) and base spec 'Backward-compatible deserialization with migration'",
            "input": "`servers` key absent AND both `remote` and `domestic` present",
            "state": "servers-legacy-migrated"
          },
          {
            "effect": "forced",
            "evidence": "crates/core/src/models/dns.rs:246-248; defaults are remote DoH 1.1.1.1 port None + domestic Udp 223.5.5.5 port None (dns.rs:365-390); spec delta scenario Missing servers key gets defaults",
            "input": "`servers` key absent AND no legacy pair",
            "state": "servers-defaults-substituted"
          },
          {
            "effect": "set",
            "evidence": "spec delta scenario Missing enabled key; tasks.md 1.2; refusal path today: deserialize error -> PersistenceError::CorruptConfig at crates/core/src/persistence/settings.rs:36",
            "input": "settings.toml with [dns] lacking enabled (servers present), non-DNS sections carrying non-default values",
            "state": "loaded-preserved"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/settings.rs:25 (unchanged guard, refuses at the toml parse layer)",
            "input": "settings.toml that is invalid TOML",
            "state": "corrupt-config"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/settings.rs:29-37 (unchanged guard, refuses at the deserialize layer)",
            "input": "settings.toml that is valid TOML but fails AppSettings deserialization for a reason other than [dns].enabled",
            "state": "corrupt-config"
          }
        ],
        "forbidden": [
          "dns.enabled == true after loading a [dns] table whose enabled key was absent",
          "default substitution or legacy migration when a servers key is present in any form, including servers = []",
          "servers.len() > 0 after loading servers = []",
          "adding skip_serializing_if (or any omission) on DnsConfig.servers serialization — DnsConfig always writes servers, and omitting it would make a saved empty list reload as the defaults",
          "load_settings returning Err(PersistenceError::CorruptConfig(..)) for a file whose only defect is a missing [dns].enabled",
          "any non-DNS field of the loaded AppSettings reverting to its default when the file's other sections parse",
          "asserting through load_settings_or_default — it maps CorruptConfig to AppSettings::default() (settings.rs:40-46) and would mask the failure the test exists to catch"
        ],
        "seeding": [
          "All states seeded only by toml::from_str::<DnsConfig>(...) on a hand-written TOML literal — the path the existing tests use (dns.rs:689-706, dns.rs:708-752); no struct-literal bypass of the wire",
          "enabled-absent-false: literal '[dns]' table with servers and no enabled line",
          "servers-empty-as-written: same literal plus a save/load round trip through toml::to_string + from_str to prove the empty list survives reload (DnsConfig derives Serialize directly; from = DnsConfigWire only affects Deserialize, dns.rs:184-185)",
          "servers-legacy-migrated: exact literal of test_backward_compat_migration_from_legacy (dns.rs:690-693)",
          "servers-defaults-substituted: literal with enabled = false and neither servers nor remote/domestic; assert equality with DnsConfig::default().servers (2 servers)",
          "let (_tmp, paths) = super::super::test_paths(); paths.ensure_dirs().unwrap(); fs::write(paths.settings_path(), toml) with a hand-written TOML string — the pattern of test_corrupt_config_falls_back (settings.rs:163-171)",
          "loaded-preserved: TOML with e.g. socks_port = 9999 and language = \"ru\" (non-defaults, mirroring test_settings_save_load_roundtrip at settings.rs:144-152) plus a [dns] table with servers and no enabled; assert load_settings(&paths).unwrap() returns those values and dns.enabled == false"
        ],
        "budgets": []
      },
      "redTasks": [],
      "codeTasks": [
        "1.1: crates/core/src/models/dns.rs — #[serde(default)] on DnsConfigWire.enabled (bool default false), servers: Option<Vec<DnsServerConfig>> (absent key deserializes to None without an extra attribute); From<DnsConfigWire> (dns.rs:235-259) becomes match wire.servers { Some(v) => v, None => legacy remote+domestic pair (dns.rs:240-245) else DnsConfig::default().servers }",
        "1.1 tests in dns.rs tests mod: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults; test_backward_compat_migration_from_legacy, test_new_format_direct_load, test_dns_config_roundtrip, test_dns_config_roundtrip_with_all_fields, test_dns_config_default must stay green unedited",
        "1.2 test only, no production code expected (proposal.md Impact): add test_load_settings_partial_dns_preserves_other_sections to crates/core/src/persistence/settings.rs tests mod; goes green only after dns-wire-partial-load merges — run this seam's verify after that seam, or fold both into one chunk if the runner cannot order them"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && timeout 3m cargo clippy -p v2ray-rs-core --all-targets -- -D warnings",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust change — this run has no test-writer agent for Rust and no tester stage for a non-go test command; the tests are the coder's first codeTasks, per this seam's summary."
    },
    {
      "id": "dns-port",
      "taskIds": [
        "1.3"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "downgraded-dns-port",
      "shard": "addr",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "dns_server_address_for_backend",
          "anchor": "fn dns_server_address_for_backend(server: &DnsServerConfig, backend: V2rayFamilyBackend) -> String {",
          "change": "Pass port = None to server_address when effective_protocol != server.protocol so downgraded servers use DoH default port"
        },
        {
          "task": "1.3",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "tests",
          "anchor": "fn test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols() {",
          "change": "NEW: Add test verifying v2ray DoT port 853 and xray H3 port 8443 downgrade to DoH without port, while native DoH keeps explicit port"
        }
      ],
      "contract": {
        "forbidden": [
          "a downgraded address carrying the original protocol's port (e.g. https://dns.google:853/dns-query, https://dns.google:8443/dns-query)",
          "a native-protocol server losing an explicit non-default port",
          "removing or rewording the log::warn! downgrade message at v2ray.rs:987-996 (design.md: the downgrade warning stays)"
        ],
        "seeding": [
          "Unit-level: call dns_server_address_for_backend(&server, V2rayFamilyBackend::V2ray | V2rayFamilyBackend::Xray) directly from the v2ray.rs tests mod (fn at v2ray.rs:980, enum at v2ray.rs:10, both in scope via use super::* at v2ray.rs:1005); server built as DnsServerConfig { tag, protocol, address, port, detour: None } literal — the pattern of test_dns_supported_protocol_addresses_preserved (v2ray.rs:2046-2068)",
          "Through-config for the v2ray row: default_settings() + settings.dns.enabled = true + one server, then build_dns(&[], &settings) (cfg(test) helper v2ray.rs:659-662) and read dns[\"servers\"][0] — pattern of v2ray.rs:2072-2097",
          "No state here is reachable through a saved settings file; the formatter is pure over its arguments"
        ],
        "states": [
          "address-doh-default-port",
          "address-explicit-port-kept",
          "native-protocol-unchanged"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "spec delta scenario DoT with explicit port on v2ray; fallback Dot->Doh for v2ray at dns.rs:72-74, DoH formatting at dns.rs:34-41; port must be passed as None at v2ray.rs:998 when effective != configured (v2ray.rs:985-996)",
            "input": "v2ray backend, server DnsProtocol::Dot 'dns.google' port Some(853)",
            "state": "address-doh-default-port"
          },
          {
            "effect": "set",
            "evidence": "tasks.md 1.3; fallback H3->Doh for xray at dns.rs:73-74",
            "input": "xray backend, server DnsProtocol::H3 'dns.google' port Some(8443)",
            "state": "address-doh-default-port"
          },
          {
            "effect": "no-op",
            "evidence": "spec delta scenario Native DoH keeps its port; dns.rs:38-41 (existing behavior, port preserved)",
            "input": "any backend, server DnsProtocol::Doh 'doh.example.com' port Some(8443)",
            "state": "address-explicit-port-kept"
          },
          {
            "effect": "no-op",
            "evidence": "dns.rs:71-74 fallback list — xray downgrades only H3; tls://dns.google:8530 stays; guard row so the fix does not over-drop ports for native protocols",
            "input": "xray backend, server DnsProtocol::Dot 'dns.google' port Some(8530) (no fallback)",
            "state": "native-protocol-unchanged"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.3: crates/core/src/config/v2ray.rs — in dns_server_address_for_backend (v2ray.rs:980-998) pass None instead of server.port when effective_protocol != server.protocol, keeping the warn! branch; attach_exclude_domains (v2ray.rs:955) computes its target_addr through the same fn so it stays consistent with no extra edit",
        "1.3 tests in v2ray.rs tests mod: test_dns_downgraded_server_uses_doh_default_port (v2ray DoT dns.google 853 -> https://dns.google/dns-query; xray H3 dns.google 8443 -> https://dns.google/dns-query), test_dns_native_doh_keeps_explicit_port (DoH doh.example.com 8443 -> https://doh.example.com:8443/dns-query); test_dns_supported_protocol_addresses_preserved and test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols (v2ray.rs:2046-2097, both port: None) must stay green unedited"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && timeout 3m cargo clippy -p v2ray-rs-core --all-targets -- -D warnings",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust change — this run has no test-writer agent for Rust and no tester stage for a non-go test command; the tests are the coder's first codeTasks, per this seam's summary."
    }
  ],
  "seams": [
    {
      "id": "dns-wire-partial-load",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust change with no test-writer agent and no tester stage for cargo test; NO-TESTER-WAIVER: no red run exists, tests are written by the coder as the first codeTasks. DnsConfigWire lenient [dns] load: enabled defaults false, servers Option<Vec> taken as written. Loader half: NO-RED-WAIVER: no Rust test-writer agent or tester stage in this run; NO-TESTER-WAIVER: the test is written by the coder as the first codeTask. load_settings keeps a partial-[dns] file intact instead of CorruptConfig; test-only seam, green only after dns-wire-partial-load lands.",
      "contract": {
        "states": [
          "enabled-absent-false",
          "enabled-as-written",
          "servers-empty-as-written",
          "servers-nonempty-as-written",
          "servers-legacy-migrated",
          "servers-defaults-substituted",
          "loaded-preserved",
          "corrupt-config"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "spec delta 'Partial DNS settings load without data loss' scenario Missing enabled key; tasks.md 1.1; wire field today has no default at crates/core/src/models/dns.rs:201",
            "input": "TOML [dns] without `enabled` key (servers in any form)",
            "state": "enabled-absent-false"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/models/dns.rs:237 (enabled: wire.enabled carried verbatim)",
            "input": "TOML [dns] with `enabled = <bool>`",
            "state": "enabled-as-written"
          },
          {
            "effect": "set",
            "evidence": "design.md decision 'Some(v) is taken as written, including an empty list'; today empty is replaced at crates/core/src/models/dns.rs:237",
            "input": "`servers = []` present (even alongside legacy remote/domestic)",
            "state": "servers-empty-as-written"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/models/dns.rs:237-238 (current non-empty branch unchanged)",
            "input": "`servers` present and non-empty",
            "state": "servers-nonempty-as-written"
          },
          {
            "effect": "set",
            "evidence": "crates/core/src/models/dns.rs:239-245; guarded by test_backward_compat_migration_from_legacy (dns.rs:689) and base spec 'Backward-compatible deserialization with migration'",
            "input": "`servers` key absent AND both `remote` and `domestic` present",
            "state": "servers-legacy-migrated"
          },
          {
            "effect": "forced",
            "evidence": "crates/core/src/models/dns.rs:246-248; defaults are remote DoH 1.1.1.1 port None + domestic Udp 223.5.5.5 port None (dns.rs:365-390); spec delta scenario Missing servers key gets defaults",
            "input": "`servers` key absent AND no legacy pair",
            "state": "servers-defaults-substituted"
          },
          {
            "effect": "set",
            "evidence": "spec delta scenario Missing enabled key; tasks.md 1.2; refusal path today: deserialize error -> PersistenceError::CorruptConfig at crates/core/src/persistence/settings.rs:36",
            "input": "settings.toml with [dns] lacking enabled (servers present), non-DNS sections carrying non-default values",
            "state": "loaded-preserved"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/settings.rs:25 (unchanged guard, refuses at the toml parse layer)",
            "input": "settings.toml that is invalid TOML",
            "state": "corrupt-config"
          },
          {
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/settings.rs:29-37 (unchanged guard, refuses at the deserialize layer)",
            "input": "settings.toml that is valid TOML but fails AppSettings deserialization for a reason other than [dns].enabled",
            "state": "corrupt-config"
          }
        ],
        "forbidden": [
          "dns.enabled == true after loading a [dns] table whose enabled key was absent",
          "default substitution or legacy migration when a servers key is present in any form, including servers = []",
          "servers.len() > 0 after loading servers = []",
          "adding skip_serializing_if (or any omission) on DnsConfig.servers serialization — DnsConfig always writes servers, and omitting it would make a saved empty list reload as the defaults",
          "load_settings returning Err(PersistenceError::CorruptConfig(..)) for a file whose only defect is a missing [dns].enabled",
          "any non-DNS field of the loaded AppSettings reverting to its default when the file's other sections parse",
          "asserting through load_settings_or_default — it maps CorruptConfig to AppSettings::default() (settings.rs:40-46) and would mask the failure the test exists to catch"
        ],
        "seeding": [
          "All states seeded only by toml::from_str::<DnsConfig>(...) on a hand-written TOML literal — the path the existing tests use (dns.rs:689-706, dns.rs:708-752); no struct-literal bypass of the wire",
          "enabled-absent-false: literal '[dns]' table with servers and no enabled line",
          "servers-empty-as-written: same literal plus a save/load round trip through toml::to_string + from_str to prove the empty list survives reload (DnsConfig derives Serialize directly; from = DnsConfigWire only affects Deserialize, dns.rs:184-185)",
          "servers-legacy-migrated: exact literal of test_backward_compat_migration_from_legacy (dns.rs:690-693)",
          "servers-defaults-substituted: literal with enabled = false and neither servers nor remote/domestic; assert equality with DnsConfig::default().servers (2 servers)",
          "let (_tmp, paths) = super::super::test_paths(); paths.ensure_dirs().unwrap(); fs::write(paths.settings_path(), toml) with a hand-written TOML string — the pattern of test_corrupt_config_falls_back (settings.rs:163-171)",
          "loaded-preserved: TOML with e.g. socks_port = 9999 and language = \"ru\" (non-defaults, mirroring test_settings_save_load_roundtrip at settings.rs:144-152) plus a [dns] table with servers and no enabled; assert load_settings(&paths).unwrap() returns those values and dns.enabled == false"
        ],
        "budgets": []
      },
      "codeTasks": [
        "1.1: crates/core/src/models/dns.rs — #[serde(default)] on DnsConfigWire.enabled (bool default false), servers: Option<Vec<DnsServerConfig>> (absent key deserializes to None without an extra attribute); From<DnsConfigWire> (dns.rs:235-259) becomes match wire.servers { Some(v) => v, None => legacy remote+domestic pair (dns.rs:240-245) else DnsConfig::default().servers }",
        "1.1 tests in dns.rs tests mod: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults; test_backward_compat_migration_from_legacy, test_new_format_direct_load, test_dns_config_roundtrip, test_dns_config_roundtrip_with_all_fields, test_dns_config_default must stay green unedited"
      ]
    },
    {
      "id": "downgraded-dns-port",
      "tasks": [
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: no Rust test-writer agent or tester stage in this run; NO-TESTER-WAIVER: tests written by the coder as the first codeTask. dns_server_address_for_backend drops the configured port on protocol downgrade so DoH's default port is emitted.",
      "contract": {
        "forbidden": [
          "a downgraded address carrying the original protocol's port (e.g. https://dns.google:853/dns-query, https://dns.google:8443/dns-query)",
          "a native-protocol server losing an explicit non-default port",
          "removing or rewording the log::warn! downgrade message at v2ray.rs:987-996 (design.md: the downgrade warning stays)"
        ],
        "seeding": [
          "Unit-level: call dns_server_address_for_backend(&server, V2rayFamilyBackend::V2ray | V2rayFamilyBackend::Xray) directly from the v2ray.rs tests mod (fn at v2ray.rs:980, enum at v2ray.rs:10, both in scope via use super::* at v2ray.rs:1005); server built as DnsServerConfig { tag, protocol, address, port, detour: None } literal — the pattern of test_dns_supported_protocol_addresses_preserved (v2ray.rs:2046-2068)",
          "Through-config for the v2ray row: default_settings() + settings.dns.enabled = true + one server, then build_dns(&[], &settings) (cfg(test) helper v2ray.rs:659-662) and read dns[\"servers\"][0] — pattern of v2ray.rs:2072-2097",
          "No state here is reachable through a saved settings file; the formatter is pure over its arguments"
        ],
        "states": [
          "address-doh-default-port",
          "address-explicit-port-kept",
          "native-protocol-unchanged"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "spec delta scenario DoT with explicit port on v2ray; fallback Dot->Doh for v2ray at dns.rs:72-74, DoH formatting at dns.rs:34-41; port must be passed as None at v2ray.rs:998 when effective != configured (v2ray.rs:985-996)",
            "input": "v2ray backend, server DnsProtocol::Dot 'dns.google' port Some(853)",
            "state": "address-doh-default-port"
          },
          {
            "effect": "set",
            "evidence": "tasks.md 1.3; fallback H3->Doh for xray at dns.rs:73-74",
            "input": "xray backend, server DnsProtocol::H3 'dns.google' port Some(8443)",
            "state": "address-doh-default-port"
          },
          {
            "effect": "no-op",
            "evidence": "spec delta scenario Native DoH keeps its port; dns.rs:38-41 (existing behavior, port preserved)",
            "input": "any backend, server DnsProtocol::Doh 'doh.example.com' port Some(8443)",
            "state": "address-explicit-port-kept"
          },
          {
            "effect": "no-op",
            "evidence": "dns.rs:71-74 fallback list — xray downgrades only H3; tls://dns.google:8530 stays; guard row so the fix does not over-drop ports for native protocols",
            "input": "xray backend, server DnsProtocol::Dot 'dns.google' port Some(8530) (no fallback)",
            "state": "native-protocol-unchanged"
          }
        ]
      },
      "codeTasks": [
        "1.3: crates/core/src/config/v2ray.rs — in dns_server_address_for_backend (v2ray.rs:980-998) pass None instead of server.port when effective_protocol != server.protocol, keeping the warn! branch; attach_exclude_domains (v2ray.rs:955) computes its target_addr through the same fn so it stays consistent with no extra edit",
        "1.3 tests in v2ray.rs tests mod: test_dns_downgraded_server_uses_doh_default_port (v2ray DoT dns.google 853 -> https://dns.google/dns-query; xray H3 dns.google 8443 -> https://dns.google/dns-query), test_dns_native_doh_keeps_explicit_port (DoH doh.example.com 8443 -> https://doh.example.com:8443/dns-query); test_dns_supported_protocol_addresses_preserved and test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols (v2ray.rs:2046-2097, both port: None) must stay green unedited"
      ]
    },
    {
      "id": "run-verification",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER / NO-TESTER-WAIVER: verification-only seam, owns no chunk and no code — task 2.1 is the whole-workspace command `timeout 10m cargo test --workspace -- --test-threads=4` (the fullFloor, run once after all three code seams merge), and task 2.2 is a manual acceptance step: copy a settings file under a dev profile (AppPaths::new_dev(), persistence.rs), delete `enabled` from its [dns], start the app, confirm every other setting is intact and the DNS page shows DNS disabled.",
      "contract": {
        "forbidden": [
          "running the whole-workspace floor before all three code seams are merged — a parallel worktree without task 1.1 makes settings-partial-dns-load's test fail and reports a phantom regression"
        ],
        "seeding": [
          "workspace-green seeded only by the merge of all three code seams; manual-acceptance-pending seeded by a dev-profile settings copy with the enabled line removed from [dns]"
        ],
        "states": [
          "workspace-green",
          "manual-acceptance-pending"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "tasks.md 2.1",
            "input": "fullFloor command after dns-wire-partial-load, settings-partial-dns-load and downgraded-dns-port are all merged",
            "state": "workspace-green"
          },
          {
            "effect": "set",
            "evidence": "tasks.md 2.2; recorded by the operator, not automated",
            "input": "manual dev-profile check per tasks.md 2.2",
            "state": "manual-acceptance-pending"
          }
        ]
      }
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Partial DNS settings load without data loss Loading `settings.toml` SHALL accept a `[dns]` table that omits `enabled`, treating it as `false`, so a partial table never makes the whole settings file unreadable. An explicitly present `servers` key SHALL be loaded as written, including an empty list. The two default servers SHALL be supplied only when the `servers` key is absent and no legacy `remote`/`domestic` fields are present.",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "- **THEN** settings SHALL load with `dns.enabled = false`, the configured servers, and every other section's values preserved",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "- **THEN** the loaded `dns.servers` SHALL be empty",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "- **THEN** the loaded `dns.servers` SHALL be the two default servers",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "### Requirement: Downgraded DNS servers use the DoH default port When a DNS server's protocol is downgraded to DoH for the selected backend, the generated address SHALL use DoH's default port and SHALL NOT carry a port set for the original protocol. A server configured as DoH keeps its explicit port.",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "- **THEN** the generated address SHALL be `https://dns.google/dns-query`",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    },
    {
      "shall": "- **THEN** the generated address SHALL be `https://doh.example.com:8443/dns-query`",
      "tests": [
        "crates/core/src/models/dns.rs tests: test_dns_config_missing_enabled_defaults_false, test_dns_config_empty_servers_list_roundtrip, test_dns_config_missing_servers_without_legacy_gets_defaults",
        "crates/core/src/persistence/settings.rs tests: test_load_settings_partial_dns_preserves_other_sections",
        "crates/core/src/config/v2ray.rs tests: test_dns_downgraded_server_uses_doh_default_port, test_dns_native_doh_keeps_explicit_port"
      ]
    }
  ],
  "testHarness": [
    "DnsConfig::default — crates/core/src/models/dns.rs:365 — builds default DnsConfig with enabled=false and two default servers (remote 1.1.1.1 DoH, domestic 223.5.5.5 UDP)",
    "FakeIpConfig::default — crates/core/src/models/dns.rs:146 — builds default FakeIpConfig with enabled=false and standard IPv4/IPv6 CIDRs",
    "builtin_dns_presets — crates/core/src/models/dns.rs:312 — returns Vec<DnsProviderPreset> with 8 predefined DNS preset configurations",
    "test_backward_compat_migration_from_legacy — crates/core/src/models/dns.rs:689 — existing test asserting DnsConfigWire deserialization and migration of legacy remote/domestic tables",
    "test_new_format_direct_load — crates/core/src/models/dns.rs:708 — existing test asserting DnsConfigWire deserialization of modern [[servers]] and [[rules]] TOML",
    "test_dns_config_roundtrip — crates/core/src/models/dns.rs:644 — existing test asserting DnsConfig::default serde JSON roundtrip via DnsConfigWire",
    "test_dns_config_roundtrip_with_all_fields — crates/core/src/models/dns.rs:652 — existing test asserting fully populated DnsConfig serde JSON roundtrip via DnsConfigWire",
    "test_paths — crates/core/src/persistence/mod.rs:463 — builds (tempfile::TempDir, AppPaths) fixture isolated for AppProfile::Test",
    "save_settings — crates/core/src/persistence/settings.rs:88 — serializes AppSettings into settings.toml under paths.config_dir()",
    "load_settings — crates/core/src/persistence/settings.rs:17 — deserializes settings.toml into Result<AppSettings, PersistenceError>",
    "load_settings_or_default — crates/core/src/persistence/settings.rs:40 — loads AppSettings from disk or falls back to AppSettings::default() on CorruptConfig",
    "test_corrupt_config_falls_back — crates/core/src/persistence/settings.rs:166 — existing test asserting CorruptConfig fallback to AppSettings::default()",
    "test_load_settings_missing_file_returns_default — crates/core/src/persistence/settings.rs:158 — existing test asserting missing settings.toml returns AppSettings::default()",
    "default_settings — crates/core/src/config/test_fixtures.rs:5 — builds default AppSettings fixture for generator testing",
    "vless_node — crates/core/src/config/test_fixtures.rs:9 — builds sample VLESS ProxyNode fixture for config generation",
    "vmess_node — crates/core/src/config/test_fixtures.rs:30 — builds sample VMess ProxyNode fixture for config generation",
    "ss_node — crates/core/src/config/test_fixtures.rs:44 — builds sample Shadowsocks ProxyNode fixture for config generation",
    "trojan_node — crates/core/src/config/test_fixtures.rs:54 — builds sample Trojan ProxyNode fixture for config generation",
    "xhttp_node — crates/core/src/config/test_fixtures.rs:67 — builds sample XHTTP ProxyNode fixture for config generation",
    "build_dns — crates/core/src/config/v2ray.rs:660 — builds test V2Ray DNS JSON Value from RoutingRules and AppSettings",
    "build_dns_for_backend — crates/core/src/config/v2ray.rs:664 — builds backend-specific V2Ray/Xray DNS JSON Value",
    "test_dns_supported_protocol_addresses_preserved — crates/core/src/config/v2ray.rs:2047 — existing test asserting supported DNS protocol addresses format properly",
    "test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols — crates/core/src/config/v2ray.rs:2073 — existing test asserting DoT, DoQ, and H3 fallback to DoH on V2Ray backend"
  ],
  "floor": "make lint && TEST_TIMEOUT=10m make test",
  "estimateHours": 1.5,
  "planRulings": [
    "Seams dns-wire-partial-load and settings-partial-dns-load are merged into one chunk (tasks 1.1 + 1.2): the loader test is red until the wire change lands, so the two run in one worktree instead of a shard whose merge order the graph would have to guarantee.",
    "dns-port runs in parallel — crates/core/src/config/v2ray.rs is disjoint from both files of the wire chunk.",
    "CHANGELOG.md carries an entry under [Unreleased] because the change is user-facing (a settings file that previously loaded as defaults now loads intact); it is outside the OpenSpec impact list and repo policy requires it.",
    "Tier standard, lenses spec + quality: the change is small but rewrites the deserialization contract of a persisted settings file, so the quality lens reads the serde shape.",
    "No red stage: this is a Rust change and the run has no test-writer agent for Rust; the kernel seals nothing (no pkgDirs, no redTasks) and each chunk closes by waiver.",
    "Floor strengthened after plan review: `make lint && TEST_TIMEOUT=10m make test` keeps the Makefile's own timeout/thread bounds and adds --all-targets, which the CI test job runs; the floor still cannot exercise the sing-box-dependent paths CI provisions (sing-box is not installed here)."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1,
    "warnings": [
      "evidence line drift in dns-wire contract: \"enabled: wire.enabled carried verbatim\" cited at crates/core/src/models/dns.rs:237 but sits at dns.rs:250 inside the same impl (anchor \"impl From<DnsConfigWire> for DnsConfig {\" greps exactly at 235); attach_exclude_domains cited at v2ray.rs:955 vs fn head at 943 — cosmetic, all sites anchors hit at HEAD 331a0908",
      "task 2.2 (live dev-profile acceptance: remove enabled from [dns], start app, DNS page shows DNS disabled) has no automated stage that can fail on it — seam run-verification records it as operator-manual per tasks.md; flagged per rubric",
      "plan floor \"make lint && timeout 10m cargo test --workspace -- --test-threads=4\" matches Makefile discipline (lint: fmt clippy exists; TEST_THREADS=4/TEST_TIMEOUT=5m) but is weaker than CI, which runs \"cargo test --workspace --all-targets\" after provisioning sing-box 1.13.14: --all-targets omitted and no sing-box in the floor, so bin-target tests and sing-box-dependent behavior go unexercised"
    ]
  }
}
```
