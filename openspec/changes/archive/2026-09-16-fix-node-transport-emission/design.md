## Context

- WebSocket: `build_ws_settings` (`crates/core/src/config/v2ray.rs:392-400`) emits `headers` when non-empty, and falls back to `{"Host": host}` only when headers are empty — a set host is dropped whenever other headers exist. sing-box `build_ws_transport` (`crates/core/src/config/singbox.rs:335-350`) inserts `Host` with `entry("Host").or_insert_with(host)`, which is case-sensitive. xray's `migrate_ws_host` (`crates/core/src/config/xray.rs:144-169`) moves a case-insensitive `host` header to `wsSettings.host`, so fixing the base builder fixes xray too.
- v2ray feature emission: `apply_stream_settings` writes `security: "reality"` + `realitySettings` (`v2ray.rs:350-367`) and `network: "xhttp"` + `xhttpSettings` (`:343-346`) for both family backends. `V2rayGenerator::generate` only checks for empty nodes (`v2ray.rs:20-35`). xray already refuses nodes it cannot express (`UnsupportedTlsVerification`, `xray.rs:25-30`); sing-box refuses XHTTP with `ConfigError::UnsupportedTransport` (`singbox.rs:325-330`, `crates/core/src/config/mod.rs:44-45`), and its probe drops such nodes with `filter_map(... .ok())` (`crates/core/src/config/probe.rs:60-70`). `V2rayProbeGenerator` maps every node through `build_family_outbound` (`probe.rs:150-161`).
- `VlessConfig.flow` is emitted by the shared VLESS builder for v2ray too (`v2ray.rs:246-248`).
- sing-box drops `GrpcSettings.multi_mode` (`singbox.rs:352-357`) and `TlsSettings.spider_x` (`singbox.rs:384-393`). The node editor offers both: `grpc_multi_mode` switch (`crates/ui/src/nodes.rs:201-204`), `spider_x` entry (`nodes.rs:342-345`); the editor has no backend in scope.
- Backend support facts: nothing in the repo records v2fly v5 transport or security support, nor the sing-box gRPC/REALITY option set. REALITY and XHTTP are introduced by Xray-core under those names; that is the only basis used here.

## Goals / Non-Goals

**Goals:**
- Same node, same Host header on every backend.
- A v2ray candidate that cannot work fails at generation with a message naming the node.

**Non-Goals:**
- Refusing VLESS `flow` on v2ray; not established which flows v2fly accepts.
- Emitting gRPC multi mode or spider X for sing-box; no verified option exists.
- Making the node editor backend-aware.

## Decisions

- **Merge host into headers case-insensitively in the shared builder.** Add `Host` only when no header key equals `host` ignoring case; this also tightens sing-box, whose `entry("Host")` would add a second `Host` next to a lowercase `host`. Alternative rejected: emitting xray `host` directly from the base builder — v2ray has no such field, and `migrate_ws_host` already does it for xray.
- **Refuse REALITY and XHTTP in `V2rayGenerator::generate`, before building.** XHTTP reuses `ConfigError::UnsupportedTransport { backend: V2ray, node }`, matching sing-box. REALITY gets a new `ConfigError::UnsupportedSecurity { backend, node, feature }` (name indicative) whose message names the feature. Scope limited to the two Xray-core-named features; assumption recorded in Context. Alternative rejected: silently falling back to plain TLS — connects to a REALITY server with the wrong handshake and hides why.
- **Probe skips refused nodes.** Share a predicate (`v2ray_supports(node)`) between generator and `V2rayProbeGenerator`; skipped nodes keep their `probe_tag(i)` gap, as sing-box does.
- **Static editor labels, not backend gating.** The editor would need the active backend threaded in, and nodes persist across backend switches, so a sensitivity tied to the current backend would mislead after a switch. `SwitchRow` gets a subtitle; `EntryRow` has no subtitle, so the spider X title becomes "Reality Spider X (not used by sing-box)". Alternative rejected: making the rows insensitive on sing-box.

## Risks / Trade-offs

- [A v2fly build does support one of these features] → the user sees an explicit error instead of a working connection; switching the backend to xray is the stated fix in the message. Revisit if a verified v2fly capability surfaces.
- [Nodes with both a lowercase `host` header and a node host previously sent both on sing-box] → now only the header value, which is what the header author asked for.

## Open Questions

- Does v2fly v5 accept `xtls-rprx-*` flows? If not, a follow-up adds flow to the refusal list without changing this design.

## Implementation plan

Tier standard, mode existing-service-strict, base `ce4666f`. Rust stack: every seam carries `NO-RED-WAIVER:` / `NO-TESTER-WAIVER:` — tests are the first code task of each chunk and chunks close by their verify command. Existing tests outside a chunk's named sites are read-only.

Chunks, in dispatch order:

1. **ws-host-base** (1.1, 1.2), integration worktree, rust-coder. `build_ws_settings` in `crates/core/src/config/v2ray.rs` (anchor `} else if let Some(host) = &ws.host {`): clone headers, insert `Host` = node host unless a key equals `host` ignoring ASCII case. Tests: `test_ws_host_merges_into_custom_headers`, `test_ws_lowercase_host_header_wins_over_node_host` (v2ray.rs); `test_ws_host_with_custom_headers_moves_to_dedicated_field`, `test_ws_lowercase_host_header_wins_over_node_host` (xray.rs, via `ws_vless_with_host_header`). `migrate_ws_host` is unchanged.
2. **ws-host-singbox** (1.3, 1.4), prev ws-host-base. `build_ws_transport` in singbox.rs (anchor `.entry("Host".to_string())`) uses the same check; test `test_singbox_ws_lowercase_host_header_wins_over_node_host`. `pinned_node_and_ws_transport_options_pass_xray_test` in `crates/core/tests/xray_check.rs` gets a WS node with a `User-Agent` header and host.
3. **v2ray-refusal-gen** (2.1, 2.2), prev ws-host-singbox. New `ConfigError::UnsupportedSecurity { backend: BackendType, node: String, feature: &'static str }` after `UnsupportedTransport`, `#[error("security {feature} not supported by backend {backend} for node '{node}'; use xray")]`. `pub(crate) fn v2ray_refusal(node: &ProxyNode) -> Option<ConfigError>` in v2ray.rs: `TransportSettings::Xhttp` → `UnsupportedTransport { backend: V2ray, node }` first, then `tls.reality` → `UnsupportedSecurity { feature: "REALITY" }`; node label `remark().unwrap_or(address())`. `V2rayGenerator::generate` returns the first refusal after the `NoNodes` check. Tests: `test_v2ray_reality_unsupported_security` (fields + exact Display), `test_v2ray_xhttp_unsupported_transport` (replaces `test_xhttp_transport`; `xhttp_node()` is XHTTP+REALITY, so it also proves precedence), `test_write_config_rejects_reality_for_v2ray`. Only `v2ray_refusal` lands here.
4. **v2ray-refusal-probe** (2.3, 2.4), prev v2ray-refusal-gen. `pub(crate) fn v2ray_supports(node: &ProxyNode) -> bool` in v2ray.rs; `V2rayProbeGenerator` filters with it, keeping `probe_tag(original index)`. Test `v2ray_probe_skips_refused_nodes`: [REALITY, TLS] → one outbound tagged `probe-1`.
5. **editor-labels** (3.1), parallel, shard `ui`, rust-coder. `crates/ui/src/nodes.rs`: `.title("gRPC Multi Mode")` row gets `.subtitle("Not used by sing-box")`; `.title("Reality Spider X")` becomes `"Reality Spider X (not used by sing-box)"`. New test module, each test skips when `gtk::init()` fails: `grpc_multi_mode_row_notes_sing_box_and_keeps_value`, `spider_x_row_notes_sing_box_and_keeps_value`.
6. **verification** (4.1, 4.2), after the ui shard merges. 4.1 is the floor; 4.2 is a manual live check reported by the operator.

Verify (core chunks): `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`. ui chunk: same with `v2ray-rs-ui`.

Floor: `timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`.

Lenses: `spec` (two capabilities touched), `quality` (standard tier). No sec/perf/arch triggers.

Risks: `xray_check.rs` and the GTK tests skip without an xray binary or a display — report run vs skip. `WsSettings.headers` is a `HashMap`; assert by key.

Plan review: zarchitect, 2 rounds, pass. Round 1 blocker — the XHTTP scenario required the error to name the transport while the design reuses `UnsupportedTransport` — resolved by aligning the spec wording to that decision.

## Plan appendix

```json
{
  "v": 2,
  "change": "fix-node-transport-emission",
  "baseSha": "ce4666f82fa95329e3651adc022d2beae6571292",
  "generatedAt": "2026-09-16T18:35:27.522Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 3.3,
  "chunks": [
    {
      "id": "ws-host-base",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "ws-host",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_ws_settings",
          "anchor": "} else if let Some(host) = &ws.host {",
          "change": "fn at v2ray.rs:391-399. Today: non-empty headers emitted as-is and ws.host dropped; host only used when headers empty. Change: clone headers, insert \"Host\"=ws.host unless any key eq_ignore_ascii_case(\"host\"); emit headers when non-empty. Shared by v2ray and xray (called from v2ray.rs:332)."
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "tests::test_xhttp_transport",
          "anchor": "fn test_xhttp_transport() {",
          "change": "add tests near here (tests mod v2ray.rs:961, uses test_fixtures::fixtures::*): headers {User-Agent:x}+host -> both keys; lowercase host header + node host -> header value kept, no \"Host\" key."
        },
        {
          "task": "1.2",
          "file": "crates/core/src/config/xray.rs",
          "symbol": "tests::test_ws_host_migration_keeps_other_headers",
          "anchor": "fn test_ws_host_migration_keeps_other_headers() {",
          "change": "add sibling test using ws_vless_with_host_header(&[(\"User-Agent\",\"x\")]) (helper xray.rs:217, host cdn.example.com): assert wsSettings.host==cdn.example.com, headers.User-Agent present, no Host. migrate_ws_host (xray.rs:144) already case-insensitive; needs no change."
        }
      ],
      "contract": {
        "states": [
          "ws.host: Option<String> (WsSettings.host, crates/core/src/models/proxy.rs:244)",
          "ws.headers: HashMap<String,String> (WsSettings.headers, proxy.rs:246)",
          "header_has_host = ws.headers.keys().any(|k| k.eq_ignore_ascii_case(\"host\"))",
          "emitted v2ray: wsSettings.headers; xray: wsSettings.host + wsSettings.headers; sing-box: transport.headers"
        ],
        "transitions": [
          {
            "input": "host=None, headers empty",
            "state": "v2ray/xray wsSettings has no headers key; xray no host key; sing-box no headers key",
            "effect": "no-op",
            "evidence": "v2ray.rs:391-399, singbox.rs:346"
          },
          {
            "input": "host=Some(h), headers empty",
            "state": "v2ray headers={Host:h}; xray host=h, headers key removed; sing-box headers={Host:h}",
            "effect": "set",
            "evidence": "v2ray.rs:395-396 (unchanged), xray.rs:241-253 test_ws_host_header_moves_to_dedicated_field"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={User-Agent:x}",
            "state": "v2ray headers={User-Agent:x, Host:cdn.example.com}; xray host=cdn.example.com, headers={User-Agent:x} without Host; sing-box headers={User-Agent:x, Host:cdn.example.com}",
            "effect": "set",
            "evidence": "spec config-generator 'Headers without Host on v2ray' / 'on xray'; bug at v2ray.rs:393-394"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={host: front.example.com}",
            "state": "v2ray headers={host: front.example.com} only (single entry, no added Host); xray host=front.example.com, headers key removed; sing-box headers={host: front.example.com} only",
            "effect": "no-op",
            "evidence": "spec 'Explicit Host header wins'; design Decision 1; sing-box case-sensitive entry at singbox.rs:342-344 is the bug"
          },
          {
            "input": "host=Some(h), headers={Host: front.example.com}",
            "state": "Host=front.example.com kept, node host ignored on all backends",
            "effect": "no-op",
            "evidence": "spec 'a Host entry already present SHALL win'; xray.rs:256-266 test_ws_host_migration_keeps_other_headers"
          },
          {
            "input": "host=None, headers={User-Agent:x}",
            "state": "headers emitted as-is, no Host",
            "effect": "no-op",
            "evidence": "v2ray.rs:393-394"
          }
        ],
        "forbidden": [
          "two header keys that both equal `host` ignoring case in emitted headers (any backend)",
          "node host overriding a header-supplied Host of any case",
          "xray wsSettings.headers containing a Host key of any case after migrate_ws_host",
          "v2ray output containing wsSettings.host (v2ray has no such field; migration stays xray-only in xray.rs:144-169)"
        ],
        "seeding": [
          "v2ray: V2rayGenerator.generate(&[node], &[], &default_settings()) with a ProxyNode::Vless built inline (TransportSettings::Ws(WsSettings{path, host, headers})) in v2ray.rs tests",
          "xray: reuse ws_vless_with_host_header(&[(k,v)]) helper at xray.rs:217-238 (host fixed to cdn.example.com) + XrayGenerator.generate",
          "sing-box: SingboxGenerator.generate(&[node], &[], &default_settings()) with inline Ws node, read config[\"outbounds\"][0][\"transport\"][\"headers\"]",
          "xray_check.rs: extend ws_node-style node inside pinned_node_and_ws_transport_options_pass_xray_test with headers {\"User-Agent\": \"x\"} and host cdn.example.com (add a local node or helper; do not change ws_node() used elsewhere unless all callers stay valid)"
        ],
        "budgets": [],
        "names": {
          "fn": "build_ws_settings(ws: &WsSettings) -> Value (v2ray.rs, signature unchanged); build_ws_transport(ws: &WsSettings) -> Value (singbox.rs, signature unchanged)",
          "check": "keys().any(|k| k.eq_ignore_ascii_case(\"host\")) — same idiom as xray.rs:155-158; a tiny shared helper is optional, not required",
          "tests": {
            "1.1": [
              "crates/core/src/config/v2ray.rs: test_ws_host_merges_into_custom_headers",
              "crates/core/src/config/v2ray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.2": [
              "crates/core/src/config/xray.rs: test_ws_host_with_custom_headers_moves_to_dedicated_field",
              "crates/core/src/config/xray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.3": [
              "crates/core/src/config/singbox.rs: test_singbox_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.4": [
              "crates/core/tests/xray_check.rs: pinned_node_and_ws_transport_options_pass_xray_test (extended, no rename)"
            ]
          }
        },
        "refusals": []
      },
      "redTasks": [],
      "codeTasks": [
        "1.1",
        "1.2"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command."
    },
    {
      "id": "ws-host-singbox",
      "taskIds": [
        "1.3",
        "1.4"
      ],
      "prev": "ws-host-base",
      "sharedPkg": "crates/core",
      "parallel": false,
      "seam": "ws-host",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "build_ws_transport",
          "anchor": ".entry(\"Host\".to_string())",
          "change": "singbox.rs:335-350 uses case-sensitive entry(\"Host\").or_insert; lowercase `host` header yields two entries. Replace with any-key eq_ignore_ascii_case check. Test in tests mod (singbox.rs:854) near test_singbox_vless_with_ws_tls (singbox.rs:1140)."
        },
        {
          "task": "1.4",
          "file": "crates/core/tests/xray_check.rs",
          "symbol": "ws_node / pinned_node_and_ws_transport_options_pass_xray_test",
          "anchor": "headers: Default::default(),",
          "change": "ws_node (xray_check.rs:228) builds WsSettings with empty headers + host cdn.example.com; add a header (e.g. User-Agent) or a second ws node with headers passed at \"pinned-node-with-ws-heartbeat\" call (xray_check.rs:286-291). Test skips when xray absent."
        }
      ],
      "contract": {
        "states": [
          "ws.host: Option<String> (WsSettings.host, crates/core/src/models/proxy.rs:244)",
          "ws.headers: HashMap<String,String> (WsSettings.headers, proxy.rs:246)",
          "header_has_host = ws.headers.keys().any(|k| k.eq_ignore_ascii_case(\"host\"))",
          "emitted v2ray: wsSettings.headers; xray: wsSettings.host + wsSettings.headers; sing-box: transport.headers"
        ],
        "transitions": [
          {
            "input": "host=None, headers empty",
            "state": "v2ray/xray wsSettings has no headers key; xray no host key; sing-box no headers key",
            "effect": "no-op",
            "evidence": "v2ray.rs:391-399, singbox.rs:346"
          },
          {
            "input": "host=Some(h), headers empty",
            "state": "v2ray headers={Host:h}; xray host=h, headers key removed; sing-box headers={Host:h}",
            "effect": "set",
            "evidence": "v2ray.rs:395-396 (unchanged), xray.rs:241-253 test_ws_host_header_moves_to_dedicated_field"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={User-Agent:x}",
            "state": "v2ray headers={User-Agent:x, Host:cdn.example.com}; xray host=cdn.example.com, headers={User-Agent:x} without Host; sing-box headers={User-Agent:x, Host:cdn.example.com}",
            "effect": "set",
            "evidence": "spec config-generator 'Headers without Host on v2ray' / 'on xray'; bug at v2ray.rs:393-394"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={host: front.example.com}",
            "state": "v2ray headers={host: front.example.com} only (single entry, no added Host); xray host=front.example.com, headers key removed; sing-box headers={host: front.example.com} only",
            "effect": "no-op",
            "evidence": "spec 'Explicit Host header wins'; design Decision 1; sing-box case-sensitive entry at singbox.rs:342-344 is the bug"
          },
          {
            "input": "host=Some(h), headers={Host: front.example.com}",
            "state": "Host=front.example.com kept, node host ignored on all backends",
            "effect": "no-op",
            "evidence": "spec 'a Host entry already present SHALL win'; xray.rs:256-266 test_ws_host_migration_keeps_other_headers"
          },
          {
            "input": "host=None, headers={User-Agent:x}",
            "state": "headers emitted as-is, no Host",
            "effect": "no-op",
            "evidence": "v2ray.rs:393-394"
          }
        ],
        "forbidden": [
          "two header keys that both equal `host` ignoring case in emitted headers (any backend)",
          "node host overriding a header-supplied Host of any case",
          "xray wsSettings.headers containing a Host key of any case after migrate_ws_host",
          "v2ray output containing wsSettings.host (v2ray has no such field; migration stays xray-only in xray.rs:144-169)"
        ],
        "seeding": [
          "v2ray: V2rayGenerator.generate(&[node], &[], &default_settings()) with a ProxyNode::Vless built inline (TransportSettings::Ws(WsSettings{path, host, headers})) in v2ray.rs tests",
          "xray: reuse ws_vless_with_host_header(&[(k,v)]) helper at xray.rs:217-238 (host fixed to cdn.example.com) + XrayGenerator.generate",
          "sing-box: SingboxGenerator.generate(&[node], &[], &default_settings()) with inline Ws node, read config[\"outbounds\"][0][\"transport\"][\"headers\"]",
          "xray_check.rs: extend ws_node-style node inside pinned_node_and_ws_transport_options_pass_xray_test with headers {\"User-Agent\": \"x\"} and host cdn.example.com (add a local node or helper; do not change ws_node() used elsewhere unless all callers stay valid)"
        ],
        "budgets": [],
        "names": {
          "fn": "build_ws_settings(ws: &WsSettings) -> Value (v2ray.rs, signature unchanged); build_ws_transport(ws: &WsSettings) -> Value (singbox.rs, signature unchanged)",
          "check": "keys().any(|k| k.eq_ignore_ascii_case(\"host\")) — same idiom as xray.rs:155-158; a tiny shared helper is optional, not required",
          "tests": {
            "1.1": [
              "crates/core/src/config/v2ray.rs: test_ws_host_merges_into_custom_headers",
              "crates/core/src/config/v2ray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.2": [
              "crates/core/src/config/xray.rs: test_ws_host_with_custom_headers_moves_to_dedicated_field",
              "crates/core/src/config/xray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.3": [
              "crates/core/src/config/singbox.rs: test_singbox_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.4": [
              "crates/core/tests/xray_check.rs: pinned_node_and_ws_transport_options_pass_xray_test (extended, no rename)"
            ]
          }
        },
        "refusals": []
      },
      "redTasks": [],
      "codeTasks": [
        "1.3",
        "1.4"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command."
    },
    {
      "id": "v2ray-refusal-gen",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "ws-host-singbox",
      "sharedPkg": "crates/core",
      "parallel": false,
      "seam": "v2ray-refusal",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/core/src/config/mod.rs",
          "symbol": "ConfigError",
          "anchor": "UnsupportedTransport { backend: BackendType, node: String },",
          "change": "add variant UnsupportedSecurity { backend: BackendType, node: String, feature: &'static str } directly after UnsupportedTransport with #[error(\"security {feature} not supported by backend {backend} for node '{node}'; use xray\")]. Display assertion lives in v2ray.rs test_v2ray_reality_unsupported_security; no test module in mod.rs."
        },
        {
          "task": "2.2",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "V2rayGenerator::generate",
          "anchor": "Ok(generate_v2ray_family_config(",
          "change": "v2ray.rs:19-36: after NoNodes check, find node with tls.reality -> new variant (backend V2ray); node with TransportSettings::Xhttp -> UnsupportedTransport{backend: BackendType::V2ray, node}. Node name idiom: node.remark().unwrap_or(node.address()).to_string() (xray.rs:28). Existing test_xhttp_transport (v2ray.rs:1716) asserts V2rayGenerator generates xhttp+reality -> must be rewritten; test_xray_xhttp_transport (v2ray.rs:1731) stays."
        },
        {
          "task": "2.2",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "tests::test_singbox_xhttp_unsupported_transport",
          "anchor": "fn test_singbox_xhttp_unsupported_transport() {",
          "change": "template to mirror (singbox.rs:1311-1322): match Err(ConfigError::UnsupportedTransport{backend,node}) assert backend + node==\"Test XHTTP\"; panic!(\"expected ..., got {other:?}\")."
        }
      ],
      "contract": {
        "states": [
          "uses_xhttp = transport is TransportSettings::Xhttp(_) (Vless/Vmess/Trojan `transport` field; Shadowsocks has none)",
          "uses_reality = tls is Some(t) with t.reality == true (TlsSettings.reality: bool, proxy.rs:289; Vless/Vmess/Trojan `tls: Option<TlsSettings>`; Shadowsocks has none)",
          "refusal: Option<ConfigError> per node"
        ],
        "transitions": [
          {
            "input": "V2rayGenerator.generate, nodes empty",
            "state": "Err(ConfigError::NoNodes)",
            "effect": "no-op",
            "evidence": "v2ray.rs:26-28 — empty check stays first"
          },
          {
            "input": "V2rayGenerator.generate, a node with TransportSettings::Xhttp (tls any)",
            "state": "Err(ConfigError::UnsupportedTransport { backend: BackendType::V2ray, node })",
            "effect": "forced",
            "evidence": "spec 'XHTTP node on v2ray'; design Decision 2; mirrors singbox.rs:325-330"
          },
          {
            "input": "V2rayGenerator.generate, a non-XHTTP node with tls.reality == true",
            "state": "Err(ConfigError::UnsupportedSecurity { backend: BackendType::V2ray, node, feature: \"REALITY\" }); no config built, so no realitySettings",
            "effect": "forced",
            "evidence": "spec 'REALITY node on v2ray'; design Decision 2"
          },
          {
            "input": "node with both XHTTP and REALITY (e.g. test_fixtures::xhttp_node)",
            "state": "UnsupportedTransport wins (transport checked before security)",
            "effect": "forced",
            "evidence": "plan decision; keeps parity with sing-box, whose test_singbox_xhttp_unsupported_transport uses the same REALITY+XHTTP fixture"
          },
          {
            "input": "several nodes, first refused node at index k (including via_node targets at nodes[1..])",
            "state": "error names nodes[k]; scan order = slice order",
            "effect": "forced",
            "evidence": "xray.rs:25-30 pattern (nodes.iter().find)"
          },
          {
            "input": "node label",
            "state": "node = node.remark().unwrap_or(node.address()).to_string()",
            "effect": "set",
            "evidence": "xray.rs:28, singbox.rs:328"
          },
          {
            "input": "tls Some, reality false (plain TLS), any non-XHTTP transport; Shadowsocks",
            "state": "generated as before",
            "effect": "no-op",
            "evidence": "v2ray.rs:368-385"
          },
          {
            "input": "XrayGenerator / build_xray_outbound with REALITY or XHTTP",
            "state": "generated as before (network xhttp, security reality)",
            "effect": "no-op",
            "evidence": "spec 'xray keeps REALITY and XHTTP'; refusal lives only in V2rayGenerator::generate, not in generate_v2ray_family_config/build_family_outbound"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch [REALITY node, plain TLS node]",
            "state": "outbounds = [one outbound tagged probe_tag(1)]",
            "effect": "clear",
            "evidence": "spec 'Probe batch with a REALITY node on v2ray'; probe.rs:60-70 sing-box filter_map pattern"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch of all-supported nodes",
            "state": "unchanged, tags probe-0..n",
            "effect": "no-op",
            "evidence": "probe.rs:335-380 v2ray_probe_config_shape"
          },
          {
            "input": "ConnectionService candidate loop gets generation Err",
            "state": "record_failure + continue to next candidate",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:263-281 (existing, no code change)"
          }
        ],
        "forbidden": [
          "refusal check inside generate_v2ray_family_config or build_family_outbound (would break xray, which shares them)",
          "V2rayGenerator returning Ok for a node with tls.reality or TransportSettings::Xhttp",
          "V2rayProbeGenerator renumbering tags after skipping (tag must stay probe_tag(original index))",
          "silent fallback of REALITY to plain tls on v2ray",
          "refusing VLESS flow on v2ray (non-goal)"
        ],
        "seeding": [
          "REALITY-only node: inline ProxyNode::Vless in v2ray.rs tests with transport TransportSettings::Tcp, tls Some(TlsSettings{reality: true, public_key: Some(\"pbk\".into()), ..Default::default()}), remark Some(\"Test REALITY\".into())",
          "XHTTP node: test_fixtures::fixtures::xhttp_node() (remark \"Test XHTTP\", also REALITY) — asserts UnsupportedTransport, which also proves precedence",
          "XHTTP without REALITY (optional precedence-independent case): xhttp_node() with tls set to Some(TlsSettings::default())",
          "probe batch: SubscriptionNode::new(ProxyNode::Vless{... tls reality true ...}) followed by SubscriptionNode::new of a plain TLS node (e.g. mixed_nodes()[0] clone) in probe.rs tests",
          "xray unchanged: existing test_xray_xhttp_transport (v2ray.rs:1731) and test_xray_reality_ignores_verify_flag (xray.rs:459)"
        ],
        "budgets": [],
        "names": {
          "error_variant": "ConfigError::UnsupportedSecurity { backend: BackendType, node: String, feature: &'static str }",
          "error_attr": "#[error(\"security {feature} not supported by backend {backend} for node '{node}'; use xray\")]",
          "error_display_example": "security REALITY not supported by backend v2ray for node 'Test REALITY'; use xray",
          "feature_literal": "\"REALITY\"",
          "variant_placement": "crates/core/src/config/mod.rs, directly after UnsupportedTransport (mod.rs:44-45)",
          "predicate_module": "crates/core/src/config/v2ray.rs",
          "predicate_fns": [
            "pub(crate) fn v2ray_refusal(node: &ProxyNode) -> Option<ConfigError>  — XHTTP checked first, then REALITY; None for Shadowsocks",
            "pub(crate) fn v2ray_supports(node: &ProxyNode) -> bool { v2ray_refusal(node).is_none() }  — design-named predicate used by probe.rs"
          ],
          "generator_use": "if let Some(err) = nodes.iter().find_map(v2ray_refusal) { return Err(err); } after the NoNodes check in V2rayGenerator::generate",
          "probe_use": "V2rayProbeGenerator: .enumerate().filter(|(_, n)| crate::config::v2ray::v2ray_supports(&n.node)).map(|(i, n)| build_family_outbound(&n.node, &probe_tag(i), V2rayFamilyBackend::V2ray))",
          "tests": {
            "2.1": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (asserts variant fields and exact to_string text)"
            ],
            "2.2": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (shared with 2.1)",
              "crates/core/src/config/v2ray.rs: test_v2ray_xhttp_unsupported_transport (replaces test_xhttp_transport at v2ray.rs:1716, which asserts V2rayGenerator emits xhttp and will fail)",
              "crates/core/src/config/v2ray.rs: test_write_config_rejects_reality_for_v2ray (ConfigWriter::with_dir, backend V2ray, output_path absent; mirrors xray.rs test_write_config_rejects_disabled_tls_verification_for_xray)",
              "existing: crates/core/src/config/v2ray.rs test_xray_xhttp_transport, crates/core/src/config/xray.rs test_xray_reality_ignores_verify_flag"
            ],
            "2.3": [
              "crates/core/src/config/probe.rs: v2ray_probe_skips_refused_nodes"
            ]
          }
        },
        "refusals": [
          "ConfigError::UnsupportedTransport { backend: V2ray } — refused by V2rayGenerator::generate (core config layer) before any outbound is built; surfaced by ConfigWriter::write_config, no file written",
          "ConfigError::UnsupportedSecurity { backend: V2ray, feature: \"REALITY\" } — same layer and moment",
          "probe: no error; V2rayProbeGenerator silently omits the node at config build time"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1",
        "2.2"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "notes": [
        "add only v2ray_refusal here; v2ray_supports lands in v2ray-refusal-probe (clippy dead_code)",
        "rewrite test_xhttp_transport into test_v2ray_xhttp_unsupported_transport in this chunk"
      ]
    },
    {
      "id": "v2ray-refusal-probe",
      "taskIds": [
        "2.3",
        "2.4"
      ],
      "prev": "v2ray-refusal-gen",
      "sharedPkg": "crates/core",
      "parallel": false,
      "seam": "v2ray-refusal",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/core/src/config/probe.rs",
          "symbol": "V2rayProbeGenerator::generate",
          "anchor": "crate::config::v2ray::build_family_outbound(",
          "change": "probe.rs:151-161 maps every node; switch to enumerate().filter(|(_,n)| !refused(&n.node)).map(... probe_tag(i)) keeping original index, same as SingboxProbeGenerator filter_map (probe.rs:64-70). Shared predicate should live in v2ray.rs, pub(crate)."
        },
        {
          "task": "2.3",
          "file": "crates/core/src/config/probe.rs",
          "symbol": "tests::v2ray_probe_config_shape",
          "anchor": "fn v2ray_probe_config_shape() {",
          "change": "add test: batch [REALITY node, TLS node] via SubscriptionNode::new -> outbounds.len()==1, outbounds[0][\"tag\"]==probe_tag(1)."
        }
      ],
      "contract": {
        "states": [
          "uses_xhttp = transport is TransportSettings::Xhttp(_) (Vless/Vmess/Trojan `transport` field; Shadowsocks has none)",
          "uses_reality = tls is Some(t) with t.reality == true (TlsSettings.reality: bool, proxy.rs:289; Vless/Vmess/Trojan `tls: Option<TlsSettings>`; Shadowsocks has none)",
          "refusal: Option<ConfigError> per node"
        ],
        "transitions": [
          {
            "input": "V2rayGenerator.generate, nodes empty",
            "state": "Err(ConfigError::NoNodes)",
            "effect": "no-op",
            "evidence": "v2ray.rs:26-28 — empty check stays first"
          },
          {
            "input": "V2rayGenerator.generate, a node with TransportSettings::Xhttp (tls any)",
            "state": "Err(ConfigError::UnsupportedTransport { backend: BackendType::V2ray, node })",
            "effect": "forced",
            "evidence": "spec 'XHTTP node on v2ray'; design Decision 2; mirrors singbox.rs:325-330"
          },
          {
            "input": "V2rayGenerator.generate, a non-XHTTP node with tls.reality == true",
            "state": "Err(ConfigError::UnsupportedSecurity { backend: BackendType::V2ray, node, feature: \"REALITY\" }); no config built, so no realitySettings",
            "effect": "forced",
            "evidence": "spec 'REALITY node on v2ray'; design Decision 2"
          },
          {
            "input": "node with both XHTTP and REALITY (e.g. test_fixtures::xhttp_node)",
            "state": "UnsupportedTransport wins (transport checked before security)",
            "effect": "forced",
            "evidence": "plan decision; keeps parity with sing-box, whose test_singbox_xhttp_unsupported_transport uses the same REALITY+XHTTP fixture"
          },
          {
            "input": "several nodes, first refused node at index k (including via_node targets at nodes[1..])",
            "state": "error names nodes[k]; scan order = slice order",
            "effect": "forced",
            "evidence": "xray.rs:25-30 pattern (nodes.iter().find)"
          },
          {
            "input": "node label",
            "state": "node = node.remark().unwrap_or(node.address()).to_string()",
            "effect": "set",
            "evidence": "xray.rs:28, singbox.rs:328"
          },
          {
            "input": "tls Some, reality false (plain TLS), any non-XHTTP transport; Shadowsocks",
            "state": "generated as before",
            "effect": "no-op",
            "evidence": "v2ray.rs:368-385"
          },
          {
            "input": "XrayGenerator / build_xray_outbound with REALITY or XHTTP",
            "state": "generated as before (network xhttp, security reality)",
            "effect": "no-op",
            "evidence": "spec 'xray keeps REALITY and XHTTP'; refusal lives only in V2rayGenerator::generate, not in generate_v2ray_family_config/build_family_outbound"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch [REALITY node, plain TLS node]",
            "state": "outbounds = [one outbound tagged probe_tag(1)]",
            "effect": "clear",
            "evidence": "spec 'Probe batch with a REALITY node on v2ray'; probe.rs:60-70 sing-box filter_map pattern"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch of all-supported nodes",
            "state": "unchanged, tags probe-0..n",
            "effect": "no-op",
            "evidence": "probe.rs:335-380 v2ray_probe_config_shape"
          },
          {
            "input": "ConnectionService candidate loop gets generation Err",
            "state": "record_failure + continue to next candidate",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:263-281 (existing, no code change)"
          }
        ],
        "forbidden": [
          "refusal check inside generate_v2ray_family_config or build_family_outbound (would break xray, which shares them)",
          "V2rayGenerator returning Ok for a node with tls.reality or TransportSettings::Xhttp",
          "V2rayProbeGenerator renumbering tags after skipping (tag must stay probe_tag(original index))",
          "silent fallback of REALITY to plain tls on v2ray",
          "refusing VLESS flow on v2ray (non-goal)"
        ],
        "seeding": [
          "REALITY-only node: inline ProxyNode::Vless in v2ray.rs tests with transport TransportSettings::Tcp, tls Some(TlsSettings{reality: true, public_key: Some(\"pbk\".into()), ..Default::default()}), remark Some(\"Test REALITY\".into())",
          "XHTTP node: test_fixtures::fixtures::xhttp_node() (remark \"Test XHTTP\", also REALITY) — asserts UnsupportedTransport, which also proves precedence",
          "XHTTP without REALITY (optional precedence-independent case): xhttp_node() with tls set to Some(TlsSettings::default())",
          "probe batch: SubscriptionNode::new(ProxyNode::Vless{... tls reality true ...}) followed by SubscriptionNode::new of a plain TLS node (e.g. mixed_nodes()[0] clone) in probe.rs tests",
          "xray unchanged: existing test_xray_xhttp_transport (v2ray.rs:1731) and test_xray_reality_ignores_verify_flag (xray.rs:459)"
        ],
        "budgets": [],
        "names": {
          "error_variant": "ConfigError::UnsupportedSecurity { backend: BackendType, node: String, feature: &'static str }",
          "error_attr": "#[error(\"security {feature} not supported by backend {backend} for node '{node}'; use xray\")]",
          "error_display_example": "security REALITY not supported by backend v2ray for node 'Test REALITY'; use xray",
          "feature_literal": "\"REALITY\"",
          "variant_placement": "crates/core/src/config/mod.rs, directly after UnsupportedTransport (mod.rs:44-45)",
          "predicate_module": "crates/core/src/config/v2ray.rs",
          "predicate_fns": [
            "pub(crate) fn v2ray_refusal(node: &ProxyNode) -> Option<ConfigError>  — XHTTP checked first, then REALITY; None for Shadowsocks",
            "pub(crate) fn v2ray_supports(node: &ProxyNode) -> bool { v2ray_refusal(node).is_none() }  — design-named predicate used by probe.rs"
          ],
          "generator_use": "if let Some(err) = nodes.iter().find_map(v2ray_refusal) { return Err(err); } after the NoNodes check in V2rayGenerator::generate",
          "probe_use": "V2rayProbeGenerator: .enumerate().filter(|(_, n)| crate::config::v2ray::v2ray_supports(&n.node)).map(|(i, n)| build_family_outbound(&n.node, &probe_tag(i), V2rayFamilyBackend::V2ray))",
          "tests": {
            "2.1": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (asserts variant fields and exact to_string text)"
            ],
            "2.2": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (shared with 2.1)",
              "crates/core/src/config/v2ray.rs: test_v2ray_xhttp_unsupported_transport (replaces test_xhttp_transport at v2ray.rs:1716, which asserts V2rayGenerator emits xhttp and will fail)",
              "crates/core/src/config/v2ray.rs: test_write_config_rejects_reality_for_v2ray (ConfigWriter::with_dir, backend V2ray, output_path absent; mirrors xray.rs test_write_config_rejects_disabled_tls_verification_for_xray)",
              "existing: crates/core/src/config/v2ray.rs test_xray_xhttp_transport, crates/core/src/config/xray.rs test_xray_reality_ignores_verify_flag"
            ],
            "2.3": [
              "crates/core/src/config/probe.rs: v2ray_probe_skips_refused_nodes"
            ]
          }
        },
        "refusals": [
          "ConfigError::UnsupportedTransport { backend: V2ray } — refused by V2rayGenerator::generate (core config layer) before any outbound is built; surfaced by ConfigWriter::write_config, no file written",
          "ConfigError::UnsupportedSecurity { backend: V2ray, feature: \"REALITY\" } — same layer and moment",
          "probe: no error; V2rayProbeGenerator silently omits the node at config build time"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.3",
        "2.4"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "notes": [
        "add pub(crate) fn v2ray_supports(node: &ProxyNode) -> bool { v2ray_refusal(node).is_none() } in v2ray.rs and use it in V2rayProbeGenerator"
      ]
    },
    {
      "id": "editor-labels",
      "taskIds": [
        "3.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "editor-labels",
      "shard": "ui",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/nodes.rs",
          "symbol": "TransportRows::new grpc_multi_mode",
          "anchor": ".title(\"gRPC Multi Mode\")",
          "change": "nodes.rs:201-204 SwitchRow builder: add .subtitle(\"Not used by sing-box\")."
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/nodes.rs",
          "symbol": "TlsRows::new spider_x",
          "anchor": ".title(\"Reality Spider X\")",
          "change": "nodes.rs:342-345: title -> \"Reality Spider X (not used by sing-box)\". Round-trip: TransportRows::value (nodes.rs:265, multi_mode from grpc_multi_mode.is_active() :274), TlsRows::value (nodes.rs:381, spider_x via trimmed_optional_text :393)."
        }
      ],
      "contract": {
        "states": [
          "TransportRows.grpc_multi_mode: adw::SwitchRow (nodes.rs:140, built :201-204)",
          "TlsRows.spider_x: adw::EntryRow (built nodes.rs:342-345)"
        ],
        "transitions": [
          {
            "input": "TransportRows::new(&TransportSettings::Grpc(GrpcSettings{service_name, multi_mode: true}))",
            "state": "grpc_multi_mode.title()==\"gRPC Multi Mode\", subtitle()==\"Not used by sing-box\", is_active(); value() == Grpc{multi_mode: true}",
            "effect": "set",
            "evidence": "spec ui-lists 'gRPC multi mode label'; nodes.rs:265-275"
          },
          {
            "input": "TlsRows::new(Some(&TlsSettings{reality: true, public_key: Some(..), spider_x: Some(\"/spx\"), ..}))",
            "state": "spider_x.title()==\"Reality Spider X (not used by sing-box)\"; value().unwrap().spider_x == Some(\"/spx\")",
            "effect": "set",
            "evidence": "spec 'Spider X label'; nodes.rs:381-395"
          },
          {
            "input": "any active backend",
            "state": "labels identical; rows stay sensitive/editable",
            "effect": "no-op",
            "evidence": "design Decision 4 (static labels, no backend gating)"
          }
        ],
        "forbidden": [
          "set_sensitive(false) or visibility tied to backend on these rows",
          "threading BackendType into TransportRows/TlsRows"
        ],
        "seeding": [
          "new #[cfg(test)] mod tests in crates/ui/src/nodes.rs (none exists); each test starts with `if gtk::init().is_err() { eprintln!(\"no display, skipping\"); return; }` per crates/ui/src/preferences/dns.rs:1806-1811; build rows via TransportRows::new / TlsRows::new (private, reachable from child mod via use super::*)"
        ],
        "budgets": [],
        "names": {
          "strings": [
            "subtitle \"Not used by sing-box\"",
            "title \"Reality Spider X (not used by sing-box)\"",
            "unchanged title \"gRPC Multi Mode\""
          ],
          "tests": {
            "3.1": [
              "crates/ui/src/nodes.rs: grpc_multi_mode_row_notes_sing_box_and_keeps_value",
              "crates/ui/src/nodes.rs: spider_x_row_notes_sing_box_and_keeps_value"
            ]
          }
        },
        "refusals": []
      },
      "redTasks": [],
      "codeTasks": [
        "3.1"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command."
    },
    {
      "id": "verification",
      "taskIds": [
        "4.1",
        "4.2"
      ],
      "prev": "v2ray-refusal-probe",
      "sharedPkg": "workspace",
      "parallel": false,
      "seam": "verification",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [],
      "contract": {
        "states": [
          "workspace test/fmt/clippy green",
          "manual: live xray WS node connects with wsSettings.host in generated xray.json; v2ray REALITY direct-connect toast names node + REALITY, auto-connect advances"
        ],
        "transitions": [
          {
            "input": "timeout 10m cargo test --workspace -- --test-threads=4",
            "state": "all pass",
            "effect": "set",
            "evidence": "tasks.md 4.1"
          },
          {
            "input": "manual live run",
            "state": "operator-confirmed",
            "effect": "set",
            "evidence": "tasks.md 4.2; live configs under /run/user/<uid>/…, not data_dir copies"
          }
        ],
        "forbidden": [],
        "seeding": [
          "n/a"
        ],
        "budgets": [
          "unit runs bounded by timeout 5m, workspace 10m, 4 test threads"
        ],
        "names": {},
        "refusals": []
      },
      "redTasks": [],
      "codeTasks": [
        "4.1",
        "4.2"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings",
      "coder": "zpatcher",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. 4.1 closes by the floor; 4.2 is a manual live check on xray/v2ray binaries reported by the operator.",
      "notes": [
        "runs after the ui shard (editor-labels) merged into the integration worktree",
        "4.2 manual, operator-reported"
      ]
    }
  ],
  "seams": [
    {
      "id": "ws-host",
      "tasks": [
        "1.1",
        "1.2",
        "1.3",
        "1.4"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. WebSocket node host is merged into headers as `Host` unless a header key equals `host` ignoring ASCII case; applies in v2ray.rs build_ws_settings (v2ray + xray via migrate_ws_host) and singbox.rs build_ws_transport.",
      "contract": {
        "states": [
          "ws.host: Option<String> (WsSettings.host, crates/core/src/models/proxy.rs:244)",
          "ws.headers: HashMap<String,String> (WsSettings.headers, proxy.rs:246)",
          "header_has_host = ws.headers.keys().any(|k| k.eq_ignore_ascii_case(\"host\"))",
          "emitted v2ray: wsSettings.headers; xray: wsSettings.host + wsSettings.headers; sing-box: transport.headers"
        ],
        "transitions": [
          {
            "input": "host=None, headers empty",
            "state": "v2ray/xray wsSettings has no headers key; xray no host key; sing-box no headers key",
            "effect": "no-op",
            "evidence": "v2ray.rs:391-399, singbox.rs:346"
          },
          {
            "input": "host=Some(h), headers empty",
            "state": "v2ray headers={Host:h}; xray host=h, headers key removed; sing-box headers={Host:h}",
            "effect": "set",
            "evidence": "v2ray.rs:395-396 (unchanged), xray.rs:241-253 test_ws_host_header_moves_to_dedicated_field"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={User-Agent:x}",
            "state": "v2ray headers={User-Agent:x, Host:cdn.example.com}; xray host=cdn.example.com, headers={User-Agent:x} without Host; sing-box headers={User-Agent:x, Host:cdn.example.com}",
            "effect": "set",
            "evidence": "spec config-generator 'Headers without Host on v2ray' / 'on xray'; bug at v2ray.rs:393-394"
          },
          {
            "input": "host=Some(\"cdn.example.com\"), headers={host: front.example.com}",
            "state": "v2ray headers={host: front.example.com} only (single entry, no added Host); xray host=front.example.com, headers key removed; sing-box headers={host: front.example.com} only",
            "effect": "no-op",
            "evidence": "spec 'Explicit Host header wins'; design Decision 1; sing-box case-sensitive entry at singbox.rs:342-344 is the bug"
          },
          {
            "input": "host=Some(h), headers={Host: front.example.com}",
            "state": "Host=front.example.com kept, node host ignored on all backends",
            "effect": "no-op",
            "evidence": "spec 'a Host entry already present SHALL win'; xray.rs:256-266 test_ws_host_migration_keeps_other_headers"
          },
          {
            "input": "host=None, headers={User-Agent:x}",
            "state": "headers emitted as-is, no Host",
            "effect": "no-op",
            "evidence": "v2ray.rs:393-394"
          }
        ],
        "forbidden": [
          "two header keys that both equal `host` ignoring case in emitted headers (any backend)",
          "node host overriding a header-supplied Host of any case",
          "xray wsSettings.headers containing a Host key of any case after migrate_ws_host",
          "v2ray output containing wsSettings.host (v2ray has no such field; migration stays xray-only in xray.rs:144-169)"
        ],
        "seeding": [
          "v2ray: V2rayGenerator.generate(&[node], &[], &default_settings()) with a ProxyNode::Vless built inline (TransportSettings::Ws(WsSettings{path, host, headers})) in v2ray.rs tests",
          "xray: reuse ws_vless_with_host_header(&[(k,v)]) helper at xray.rs:217-238 (host fixed to cdn.example.com) + XrayGenerator.generate",
          "sing-box: SingboxGenerator.generate(&[node], &[], &default_settings()) with inline Ws node, read config[\"outbounds\"][0][\"transport\"][\"headers\"]",
          "xray_check.rs: extend ws_node-style node inside pinned_node_and_ws_transport_options_pass_xray_test with headers {\"User-Agent\": \"x\"} and host cdn.example.com (add a local node or helper; do not change ws_node() used elsewhere unless all callers stay valid)"
        ],
        "budgets": [],
        "names": {
          "fn": "build_ws_settings(ws: &WsSettings) -> Value (v2ray.rs, signature unchanged); build_ws_transport(ws: &WsSettings) -> Value (singbox.rs, signature unchanged)",
          "check": "keys().any(|k| k.eq_ignore_ascii_case(\"host\")) — same idiom as xray.rs:155-158; a tiny shared helper is optional, not required",
          "tests": {
            "1.1": [
              "crates/core/src/config/v2ray.rs: test_ws_host_merges_into_custom_headers",
              "crates/core/src/config/v2ray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.2": [
              "crates/core/src/config/xray.rs: test_ws_host_with_custom_headers_moves_to_dedicated_field",
              "crates/core/src/config/xray.rs: test_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.3": [
              "crates/core/src/config/singbox.rs: test_singbox_ws_lowercase_host_header_wins_over_node_host"
            ],
            "1.4": [
              "crates/core/tests/xray_check.rs: pinned_node_and_ws_transport_options_pass_xray_test (extended, no rename)"
            ]
          }
        },
        "refusals": []
      },
      "codeTasks": [
        "1.1",
        "1.2",
        "1.3",
        "1.4"
      ]
    },
    {
      "id": "v2ray-refusal",
      "tasks": [
        "2.1",
        "2.2",
        "2.3",
        "2.4"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. V2rayGenerator::generate refuses any node using XHTTP transport (UnsupportedTransport) or REALITY security (new UnsupportedSecurity) before building; V2rayProbeGenerator filters the same nodes out keeping probe_tag(i) gaps; xray and sing-box unchanged.",
      "contract": {
        "states": [
          "uses_xhttp = transport is TransportSettings::Xhttp(_) (Vless/Vmess/Trojan `transport` field; Shadowsocks has none)",
          "uses_reality = tls is Some(t) with t.reality == true (TlsSettings.reality: bool, proxy.rs:289; Vless/Vmess/Trojan `tls: Option<TlsSettings>`; Shadowsocks has none)",
          "refusal: Option<ConfigError> per node"
        ],
        "transitions": [
          {
            "input": "V2rayGenerator.generate, nodes empty",
            "state": "Err(ConfigError::NoNodes)",
            "effect": "no-op",
            "evidence": "v2ray.rs:26-28 — empty check stays first"
          },
          {
            "input": "V2rayGenerator.generate, a node with TransportSettings::Xhttp (tls any)",
            "state": "Err(ConfigError::UnsupportedTransport { backend: BackendType::V2ray, node })",
            "effect": "forced",
            "evidence": "spec 'XHTTP node on v2ray'; design Decision 2; mirrors singbox.rs:325-330"
          },
          {
            "input": "V2rayGenerator.generate, a non-XHTTP node with tls.reality == true",
            "state": "Err(ConfigError::UnsupportedSecurity { backend: BackendType::V2ray, node, feature: \"REALITY\" }); no config built, so no realitySettings",
            "effect": "forced",
            "evidence": "spec 'REALITY node on v2ray'; design Decision 2"
          },
          {
            "input": "node with both XHTTP and REALITY (e.g. test_fixtures::xhttp_node)",
            "state": "UnsupportedTransport wins (transport checked before security)",
            "effect": "forced",
            "evidence": "plan decision; keeps parity with sing-box, whose test_singbox_xhttp_unsupported_transport uses the same REALITY+XHTTP fixture"
          },
          {
            "input": "several nodes, first refused node at index k (including via_node targets at nodes[1..])",
            "state": "error names nodes[k]; scan order = slice order",
            "effect": "forced",
            "evidence": "xray.rs:25-30 pattern (nodes.iter().find)"
          },
          {
            "input": "node label",
            "state": "node = node.remark().unwrap_or(node.address()).to_string()",
            "effect": "set",
            "evidence": "xray.rs:28, singbox.rs:328"
          },
          {
            "input": "tls Some, reality false (plain TLS), any non-XHTTP transport; Shadowsocks",
            "state": "generated as before",
            "effect": "no-op",
            "evidence": "v2ray.rs:368-385"
          },
          {
            "input": "XrayGenerator / build_xray_outbound with REALITY or XHTTP",
            "state": "generated as before (network xhttp, security reality)",
            "effect": "no-op",
            "evidence": "spec 'xray keeps REALITY and XHTTP'; refusal lives only in V2rayGenerator::generate, not in generate_v2ray_family_config/build_family_outbound"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch [REALITY node, plain TLS node]",
            "state": "outbounds = [one outbound tagged probe_tag(1)]",
            "effect": "clear",
            "evidence": "spec 'Probe batch with a REALITY node on v2ray'; probe.rs:60-70 sing-box filter_map pattern"
          },
          {
            "input": "V2rayProbeGenerator.generate, batch of all-supported nodes",
            "state": "unchanged, tags probe-0..n",
            "effect": "no-op",
            "evidence": "probe.rs:335-380 v2ray_probe_config_shape"
          },
          {
            "input": "ConnectionService candidate loop gets generation Err",
            "state": "record_failure + continue to next candidate",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:263-281 (existing, no code change)"
          }
        ],
        "forbidden": [
          "refusal check inside generate_v2ray_family_config or build_family_outbound (would break xray, which shares them)",
          "V2rayGenerator returning Ok for a node with tls.reality or TransportSettings::Xhttp",
          "V2rayProbeGenerator renumbering tags after skipping (tag must stay probe_tag(original index))",
          "silent fallback of REALITY to plain tls on v2ray",
          "refusing VLESS flow on v2ray (non-goal)"
        ],
        "seeding": [
          "REALITY-only node: inline ProxyNode::Vless in v2ray.rs tests with transport TransportSettings::Tcp, tls Some(TlsSettings{reality: true, public_key: Some(\"pbk\".into()), ..Default::default()}), remark Some(\"Test REALITY\".into())",
          "XHTTP node: test_fixtures::fixtures::xhttp_node() (remark \"Test XHTTP\", also REALITY) — asserts UnsupportedTransport, which also proves precedence",
          "XHTTP without REALITY (optional precedence-independent case): xhttp_node() with tls set to Some(TlsSettings::default())",
          "probe batch: SubscriptionNode::new(ProxyNode::Vless{... tls reality true ...}) followed by SubscriptionNode::new of a plain TLS node (e.g. mixed_nodes()[0] clone) in probe.rs tests",
          "xray unchanged: existing test_xray_xhttp_transport (v2ray.rs:1731) and test_xray_reality_ignores_verify_flag (xray.rs:459)"
        ],
        "budgets": [],
        "names": {
          "error_variant": "ConfigError::UnsupportedSecurity { backend: BackendType, node: String, feature: &'static str }",
          "error_attr": "#[error(\"security {feature} not supported by backend {backend} for node '{node}'; use xray\")]",
          "error_display_example": "security REALITY not supported by backend v2ray for node 'Test REALITY'; use xray",
          "feature_literal": "\"REALITY\"",
          "variant_placement": "crates/core/src/config/mod.rs, directly after UnsupportedTransport (mod.rs:44-45)",
          "predicate_module": "crates/core/src/config/v2ray.rs",
          "predicate_fns": [
            "pub(crate) fn v2ray_refusal(node: &ProxyNode) -> Option<ConfigError>  — XHTTP checked first, then REALITY; None for Shadowsocks",
            "pub(crate) fn v2ray_supports(node: &ProxyNode) -> bool { v2ray_refusal(node).is_none() }  — design-named predicate used by probe.rs"
          ],
          "generator_use": "if let Some(err) = nodes.iter().find_map(v2ray_refusal) { return Err(err); } after the NoNodes check in V2rayGenerator::generate",
          "probe_use": "V2rayProbeGenerator: .enumerate().filter(|(_, n)| crate::config::v2ray::v2ray_supports(&n.node)).map(|(i, n)| build_family_outbound(&n.node, &probe_tag(i), V2rayFamilyBackend::V2ray))",
          "tests": {
            "2.1": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (asserts variant fields and exact to_string text)"
            ],
            "2.2": [
              "crates/core/src/config/v2ray.rs: test_v2ray_reality_unsupported_security (shared with 2.1)",
              "crates/core/src/config/v2ray.rs: test_v2ray_xhttp_unsupported_transport (replaces test_xhttp_transport at v2ray.rs:1716, which asserts V2rayGenerator emits xhttp and will fail)",
              "crates/core/src/config/v2ray.rs: test_write_config_rejects_reality_for_v2ray (ConfigWriter::with_dir, backend V2ray, output_path absent; mirrors xray.rs test_write_config_rejects_disabled_tls_verification_for_xray)",
              "existing: crates/core/src/config/v2ray.rs test_xray_xhttp_transport, crates/core/src/config/xray.rs test_xray_reality_ignores_verify_flag"
            ],
            "2.3": [
              "crates/core/src/config/probe.rs: v2ray_probe_skips_refused_nodes"
            ]
          }
        },
        "refusals": [
          "ConfigError::UnsupportedTransport { backend: V2ray } — refused by V2rayGenerator::generate (core config layer) before any outbound is built; surfaced by ConfigWriter::write_config, no file written",
          "ConfigError::UnsupportedSecurity { backend: V2ray, feature: \"REALITY\" } — same layer and moment",
          "probe: no error; V2rayProbeGenerator silently omits the node at config build time"
        ]
      },
      "codeTasks": [
        "2.1",
        "2.2",
        "2.3",
        "2.4"
      ]
    },
    {
      "id": "editor-labels",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. Node editor gRPC multi mode SwitchRow gets subtitle \"Not used by sing-box\"; spider X EntryRow title becomes \"Reality Spider X (not used by sing-box)\"; values still round-trip.",
      "contract": {
        "states": [
          "TransportRows.grpc_multi_mode: adw::SwitchRow (nodes.rs:140, built :201-204)",
          "TlsRows.spider_x: adw::EntryRow (built nodes.rs:342-345)"
        ],
        "transitions": [
          {
            "input": "TransportRows::new(&TransportSettings::Grpc(GrpcSettings{service_name, multi_mode: true}))",
            "state": "grpc_multi_mode.title()==\"gRPC Multi Mode\", subtitle()==\"Not used by sing-box\", is_active(); value() == Grpc{multi_mode: true}",
            "effect": "set",
            "evidence": "spec ui-lists 'gRPC multi mode label'; nodes.rs:265-275"
          },
          {
            "input": "TlsRows::new(Some(&TlsSettings{reality: true, public_key: Some(..), spider_x: Some(\"/spx\"), ..}))",
            "state": "spider_x.title()==\"Reality Spider X (not used by sing-box)\"; value().unwrap().spider_x == Some(\"/spx\")",
            "effect": "set",
            "evidence": "spec 'Spider X label'; nodes.rs:381-395"
          },
          {
            "input": "any active backend",
            "state": "labels identical; rows stay sensitive/editable",
            "effect": "no-op",
            "evidence": "design Decision 4 (static labels, no backend gating)"
          }
        ],
        "forbidden": [
          "set_sensitive(false) or visibility tied to backend on these rows",
          "threading BackendType into TransportRows/TlsRows"
        ],
        "seeding": [
          "new #[cfg(test)] mod tests in crates/ui/src/nodes.rs (none exists); each test starts with `if gtk::init().is_err() { eprintln!(\"no display, skipping\"); return; }` per crates/ui/src/preferences/dns.rs:1806-1811; build rows via TransportRows::new / TlsRows::new (private, reachable from child mod via use super::*)"
        ],
        "budgets": [],
        "names": {
          "strings": [
            "subtitle \"Not used by sing-box\"",
            "title \"Reality Spider X (not used by sing-box)\"",
            "unchanged title \"gRPC Multi Mode\""
          ],
          "tests": {
            "3.1": [
              "crates/ui/src/nodes.rs: grpc_multi_mode_row_notes_sing_box_and_keeps_value",
              "crates/ui/src/nodes.rs: spider_x_row_notes_sing_box_and_keeps_value"
            ]
          }
        },
        "refusals": []
      },
      "codeTasks": [
        "3.1"
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "4.1",
        "4.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. 4.1 = workspace floor; 4.2 is a manual live check on real xray/v2ray binaries (MANUAL-WAIVER: not automatable in CI, reported by operator) — no observable contract beyond seams above.",
      "contract": {
        "states": [
          "workspace test/fmt/clippy green",
          "manual: live xray WS node connects with wsSettings.host in generated xray.json; v2ray REALITY direct-connect toast names node + REALITY, auto-connect advances"
        ],
        "transitions": [
          {
            "input": "timeout 10m cargo test --workspace -- --test-threads=4",
            "state": "all pass",
            "effect": "set",
            "evidence": "tasks.md 4.1"
          },
          {
            "input": "manual live run",
            "state": "operator-confirmed",
            "effect": "set",
            "evidence": "tasks.md 4.2; live configs under /run/user/<uid>/…, not data_dir copies"
          }
        ],
        "forbidden": [],
        "seeding": [
          "n/a"
        ],
        "budgets": [
          "unit runs bounded by timeout 5m, workspace 10m, 4 test threads"
        ],
        "names": {},
        "refusals": []
      },
      "codeTasks": [
        "4.1",
        "4.2"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "every generator SHALL send the node's host as the HTTP `Host` of the upgrade request",
      "tests": [
        "v2ray.rs test_ws_host_merges_into_custom_headers",
        "xray.rs test_ws_host_with_custom_headers_moves_to_dedicated_field",
        "singbox.rs test_singbox_ws_lowercase_host_header_wins_over_node_host",
        "xray_check.rs pinned_node_and_ws_transport_options_pass_xray_test"
      ]
    },
    {
      "shall": "`wsSettings.headers` SHALL contain `\"Host\": \"cdn.example.com\"` and `\"User-Agent\": \"x\"`",
      "tests": [
        "v2ray.rs test_ws_host_merges_into_custom_headers"
      ]
    },
    {
      "shall": "`wsSettings.host` SHALL be `cdn.example.com` and `wsSettings.headers` SHALL contain",
      "tests": [
        "xray.rs test_ws_host_with_custom_headers_moves_to_dedicated_field",
        "xray_check.rs pinned_node_and_ws_transport_options_pass_xray_test"
      ]
    },
    {
      "shall": "every backend SHALL send `front.example.com` as the Host",
      "tests": [
        "v2ray.rs test_ws_lowercase_host_header_wins_over_node_host",
        "xray.rs test_ws_lowercase_host_header_wins_over_node_host",
        "singbox.rs test_singbox_ws_lowercase_host_header_wins_over_node_host"
      ]
    },
    {
      "shall": "config generation SHALL fail with an error naming the node and REALITY",
      "tests": [
        "v2ray.rs test_v2ray_reality_unsupported_security",
        "v2ray.rs test_write_config_rejects_reality_for_v2ray"
      ]
    },
    {
      "shall": "config generation SHALL fail with an error naming the node and stating that the backend does not support its transport",
      "tests": [
        "v2ray.rs test_v2ray_xhttp_unsupported_transport"
      ]
    },
    {
      "shall": "the probe config SHALL contain an outbound for the TLS node only",
      "tests": [
        "probe.rs v2ray_probe_skips_refused_nodes"
      ]
    },
    {
      "shall": "the config SHALL be generated as before",
      "tests": [
        "v2ray.rs test_xray_xhttp_transport",
        "xray.rs test_xray_reality_ignores_verify_flag",
        "singbox.rs test_singbox_xhttp_unsupported_transport"
      ]
    },
    {
      "shall": "When the backend is v2ray, config generation for a candidate SHALL fail",
      "tests": [
        "v2ray.rs test_v2ray_reality_unsupported_security",
        "v2ray.rs test_v2ray_xhttp_unsupported_transport",
        "probe.rs v2ray_probe_skips_refused_nodes"
      ]
    },
    {
      "shall": "The manual node editor SHALL label the gRPC multi mode switch",
      "tests": [
        "nodes.rs grpc_multi_mode_row_notes_sing_box_and_keeps_value",
        "nodes.rs spider_x_row_notes_sing_box_and_keeps_value"
      ]
    },
    {
      "shall": "the multi mode row SHALL state that sing-box does not use it",
      "tests": [
        "nodes.rs grpc_multi_mode_row_notes_sing_box_and_keeps_value"
      ]
    },
    {
      "shall": "the spider X row SHALL state that sing-box does not use it",
      "tests": [
        "nodes.rs spider_x_row_notes_sing_box_and_keeps_value"
      ]
    }
  ],
  "testHarness": [
    "fixtures::xhttp_node — crates/core/src/config/test_fixtures.rs:67 — VLESS Xhttp /xhttp host xhttp.example.com, tls.reality=true, remark \"Test XHTTP\"",
    "fixtures::vless_node — crates/core/src/config/test_fixtures.rs:9 — VLESS ws /ws host example.com, empty headers, TLS, remark \"Test VLESS\"",
    "fixtures::default_settings — crates/core/src/config/test_fixtures.rs:5 — AppSettings::default()",
    "ws_vless_with_host_header(&[(&str,&str)]) — crates/core/src/config/xray.rs:217 — VLESS ws /ws host cdn.example.com with given headers, TLS",
    "xray_vless_with_xtls — crates/core/src/config/xray.rs:200 — VLESS tcp TLS xtls flow",
    "mixed_nodes — crates/core/src/config/probe.rs:207 — [VLESS tls, VMess, Trojan tls, SS] SubscriptionNodes",
    "ws_node / reality_node / check_with_nodes — crates/core/tests/xray_check.rs:228 / :185 / :53 — WS VLESS node, REALITY VLESS node, runs `xray run -test` and fails on deprecation lines; skipped when xray_available() false (:11)"
  ],
  "floor": "timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings",
  "risks": [
    "Existing test_xhttp_transport (crates/core/src/config/v2ray.rs:1716-1729) calls V2rayGenerator.generate on xhttp_node and unwraps → breaks under 2.2; replace it with test_v2ray_xhttp_unsupported_transport (xhttp builder coverage remains in test_xray_xhttp_transport).",
    "xray_check.rs test silently skips without xray on PATH (see memory note on singbox_check); 1.4 proof requires xray installed locally — report whether it ran or skipped.",
    "UI label tests skip without a display (gtk::init err); in headless CI they pass vacuously — report run vs skip.",
    "WsSettings.headers is a HashMap: assert header entries by key, never by serialized order or array index.",
    "Precedence choice (XHTTP before REALITY) is a plan decision, not from design.md; xhttp_node fixture has both, so test_v2ray_xhttp_unsupported_transport doubles as the precedence proof.",
    "HAZARD: fixture xhttp_node() (config/test_fixtures.rs:67-87) is BOTH Xhttp transport AND tls.reality=true -> v2ray refusal order decides which variant test sees; test_xhttp_transport (v2ray.rs:1716) currently asserts V2rayGenerator succeeds with it and will break.",
    "Spec XHTTP wording aligned in plan review round 1 to the reused UnsupportedTransport text (states the backend does not support the node transport), per design Decision 2.",
    "xray_check.rs and GTK label tests skip without xray binary / display; report run vs skip."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
