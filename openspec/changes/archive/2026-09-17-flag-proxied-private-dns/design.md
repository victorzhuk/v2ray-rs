## Context

- Scope: what the DNS page and the app log tell the user the active backend will do with a configured server.
- Detour semantics differ per backend. sing-box emits `detour: <first proxy>` for any detour other than `"direct"` and omits the key otherwise (`crates/core/src/config/singbox.rs`); a detour-less sing-box server dials directly. xray tags a server direct only under TUN (`crates/core/src/config/v2ray.rs`, `direct_detour`); without TUN, unmatched built-in resolver traffic goes to the first outbound. The DNS dialog stores `"proxy"` or `"direct"` for sing-box and xray (`crates/ui/src/preferences/dns.rs`).
- `DnsConfig::validate` (`crates/core/src/models/dns.rs`) checks tags, client subnet, rule targets and FakeIP; it takes no backend. It runs in the writer only when DNS is enabled (`crates/core/src/config/writer.rs`) and in the UI through `validate_dns_settings_for_backend` (`crates/ui/src/preferences/dns.rs`).
- Strategy mapping: `query_strategy_str` maps `PreferIpv4 | Ipv4Only` → `UseIPv4` and `PreferIpv6 | Ipv6Only` → `UseIPv6`, for both xray and v2ray (`crates/core/src/config/v2ray.rs`); `DnsStrategy::ipv4` documents the collapse (`crates/core/src/models/dns.rs`). UI labels are "Prefer IPv4", "Prefer IPv6", "IPv4 Only", "IPv6 Only" (`crates/ui/src/preferences/dns.rs`).
- Whether Xray-core's DNS `queryStrategy` accepts a preference-with-fallback value is not recorded anywhere in the repo (see Open Questions).

## Goals / Non-Goals

**Goals:**
- A server whose resolver sits behind the proxy is visible as such while it is edited, while it is listed, and in the app log when the config is generated.
- The strategy selector states what the Prefer options do on the active backend.

**Non-Goals:**
- Emitting a different xray/v2ray `queryStrategy` for the Prefer options — no verified backend support.
- Flagging hostname-addressed servers (that would need resolution at edit time) or the v2ray backend (no detour model; the routing of its resolver traffic is not established here).
- Renaming servers to `remote`/`domestic` automatically.

## Decisions

- **Warn, do not reject, private servers behind the proxy.** A predicate on `DnsServerConfig` (name indicative: `resolves_via_proxy_private`) takes the backend and TUN state and applies the backend-specific "goes through proxy" test from Context. Generators log one `log::warn!` per flagged server; the UI shows a warning label in the dialog next to the existing downgrade warning and the same text on the server row. Alternative rejected: a `DnsValidationError` — it would block a resolver on the proxy server's own loopback or private network, which is a valid topology, and would fail every connection for users upgrading with such a server.
- **Explain Prefer on xray and v2ray instead of changing values.** A subtitle on the strategy row, driven by the active backend and refreshed by the existing settings observers. Alternative rejected: emitting a fallback-capable strategy — no verified `queryStrategy` value for it.

## Risks / Trade-offs

- [The warning is ignored and lookups still fail] → the warning names the fix (detour `direct`) in the dialog and on the row, and the log line lands in the app log for diagnostics.
- [The predicate and the generators drift apart] → both call the same `DnsServerConfig` predicate; the generator test asserts through it rather than through log capture.

## Open Questions

- Does Xray-core accept a preference-with-fallback `queryStrategy`? If a later check finds one, the note can be replaced by emitting it without changing these specs' intent.

## Implementation plan

Rust workspace, mode `existing-service-strict`, tier `standard`, plan base SHA `551b4781`. Four chunks; lenses `spec` and `quality`.

There is no test-writer agent for Rust, so no chunk has a red stage: each seam carries `NO-RED-WAIVER:` and `NO-TESTER-WAIVER:`, the test functions are named in the chunk's `codeTasks`, and the coder writes them. A chunk closes by waiver; the run's floor and each chunk's own verify command are the gate.

### Naming rulings the tests assert

- `DnsServerConfig::resolves_via_proxy_private(&self, backend: BackendType, tun_enabled: bool) -> bool` in `crates/core/src/models/dns.rs`. `BackendType` comes from `crates/core/src/models/settings.rs`.
- Rule: sing-box flags iff `detour.is_some() && !detours_direct()`; xray flags iff `!(tun_enabled && detours_direct())`; v2ray never flags. `DnsConfig::validate` is unchanged — warn, never reject.
- One string is the warning: `PRIVATE_DNS_WARNING`, produced by a pure helper in `crates/ui/src/preferences/dns.rs`; the dialog, the server row and the primary row all render it.
- Strategy note: a pure helper returning `Option<&'static str>`, `Some` for xray and v2ray containing "preferred address family", `None` for sing-box, set on the strategy row's subtitle.

### Chunks

**c1-predicate** — task 1.1, seam `private-dns-predicate`, integration worktree.
`crates/core/src/models/dns.rs`: the predicate and its two range checks. Tests in the file's `#[cfg(test)] mod tests`: `test_resolves_via_proxy_private_matrix` (the ten addresses × sing-box × xray TUN on/off × v2ray, matching all four dns-configuration scenarios) and `test_resolves_via_proxy_private_address_edges` (`::ffff:127.0.0.1` flagged, `0.0.0.0` not, `fe80::1%eth0` not, `" 10.0.0.1 "` trims and is flagged, invalid and empty do not panic).
Verify: `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`.

**c2-generator-warnings** — task 2.1, seam `generator-private-dns-warning`, prev c1, shared package `v2ray-rs-core`, shard `gen`.
`crates/core/src/config/singbox.rs` (`build_dns`) and `crates/core/src/config/v2ray.rs` (the dns-server entry path): one `log::warn!` per flagged server, naming tag and address. Tests: `test_singbox_generation_flags_private_proxy_detoured_server`, `test_xray_generation_flags_private_proxy_detoured_server`, `test_v2ray_family_never_flags_private_server` — asserted through the predicate, not log capture, with generation still returning `Ok`.
Verify: `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`.

**c3-dialog-and-rows** — tasks 3.1 and 3.2, seam `ui-private-dns-warning`, prev c1, shard `ui`.
`crates/ui/src/preferences/dns.rs`: the warning helper, the dialog's `private_warning_label` below the existing downgrade `warning_label`, the refresh hooks on address/detour/protocol/backend change, the two row-subtitle helpers, and the `last_tun` guard in the settings observer. Tests: `test_private_dns_warning_text`, `test_flagged_private_server_still_validates` (the Save path's own validator still accepts the flagged server), `test_server_row_subtitle_appends_private_warning`, `test_primary_row_subtitle_appends_private_warning`.
Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4`.

**c4-strategy-note** — task 3.3, seam `strategy-family-note`, prev c3, shared package `v2ray-rs-ui`, integration worktree.
`crates/ui/src/preferences/dns.rs`: the note helper and `strategy_row.set_subtitle` inside `sync_dns_ui`. Test: `test_strategy_family_note_per_backend`.
Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4`.

Tasks 3.4 and 4.1 are the per-chunk verify commands and the floor, so no chunk owns them; the seam `verification-coverage` records that ownership.

### Floor

`make lint && make test`.

### Lenses

`spec` and `quality`. The diff touches user-facing warning text, two config generators and one preferences page; it has no auth, SQL, secret, crypto or concurrency path, adds no package and moves no boundary, so `sec`, `arch` and `perf` are not triggered.

### Manual acceptance

No agent can run these; they stay manual in the change's record.

- 4.2 — with `domestic` = `udp 127.0.0.1` detour `proxy` on xray with TUN: the DNS page shows the warning on the Domestic row, connecting still works, the app log carries one warning naming `domestic`; switching the detour to `direct` clears the warning and the regenerated xray config tags that server `dns-direct`.
- 4.3 — switching the backend to sing-box and back on the DNS page: the strategy note appears for xray and v2ray and is absent for sing-box.

### Plan review

`zarchitect`, one round, `merge_ready` with no blockers and six warnings: xray with TUN off flags every private-literal server (upheld as faithful to the requirement's wording — without TUN nothing is directly routed); the `log::warn!` emission itself is only proven live, since log capture under a shared logger is hostile; the requirements map credits the predicate tests for the "SHALL NOT reject" and log clauses, whose coverage actually sits in c2's generation assertions, c3's validator test and live 4.2; GTK widget wiring is untested headless, matching the repo's `gtk_test::run` skip-without-display pattern; the "four options and stored values" clause is guarded by the seam's forbidden list rather than a test; and `make test` caps the workspace run at `5m` while task 4.1 names `10m`.

## Plan appendix

```json
{
  "v": 2,
  "change": "flag-proxied-private-dns",
  "baseSha": "551b47811d8f86cc0dd6b1f8895abda4c229ae26",
  "generatedAt": "2026-09-17T16:12:11.422Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "c1-predicate",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "private-dns-predicate",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "DnsServerConfig",
          "lines": "81-98",
          "anchor": "    pub fn detours_direct(&self) -> bool {\n        self.detour.as_deref() == Some(\"direct\")\n    }",
          "change": "add private-server predicate taking backend and tun_enabled to identify loopback/private/link-local/unique-local IP literals sent through proxy"
        }
      ],
      "contract": {
        "states": [
          "flagged",
          "not-flagged"
        ],
        "transitions": [
          {
            "input": "sing-box, private literal in any of the four range families, detour Some(non-\"direct\") incl. legacy sentinel \"proxy-0\"",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:463-465"
          },
          {
            "input": "sing-box, private literal, detour None",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/singbox.rs:461-462 (a server with no detour is not dispatched through the proxy chain)"
          },
          {
            "input": "sing-box, private literal, detour \"direct\"",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/singbox.rs:463 + existing test test_dns_server_direct_detour_is_not_emitted at singbox.rs:2003-2023"
          },
          {
            "input": "xray, private literal, tun on, detour \"direct\"",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/v2ray.rs:827-829 (direct_detour true -> entry tagged dns-direct, v2ray.rs:848-849)"
          },
          {
            "input": "xray, private literal, tun on, detour \"proxy\" or detour None",
            "state": "flagged",
            "effect": "set",
            "evidence": "delta spec dns-configuration scenario 'Loopback server without direct routing on xray' + v2ray.rs:524-538"
          },
          {
            "input": "xray, private literal, tun off, any detour including \"direct\"",
            "state": "flagged",
            "effect": "set",
            "evidence": "v2ray.rs:85, 116-117, 524-538, 827-829 — nothing expressible as directly routed without TUN; ruled flagged"
          },
          {
            "input": "v2ray, any address, any detour, any tun state",
            "state": "not-flagged",
            "effect": "forced",
            "evidence": "delta spec dns-configuration requirement 1 last sentence: 'Hostname-addressed servers and the v2ray backend SHALL NOT be flagged'"
          },
          {
            "input": "any backend, public literal e.g. 1.1.1.1, detour proxy",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "delta spec dns-configuration scenario 'Public server is not flagged'"
          },
          {
            "input": "any backend, hostname e.g. dns.google",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "delta spec dns-configuration requirement 1 last sentence"
          },
          {
            "input": "address ::ffff:127.0.0.1 (IPv4-mapped) with sing-box proxy detour",
            "state": "flagged",
            "effect": "set",
            "evidence": "ruling: mapped v4 normalized via to_ipv4_mapped and classified by the v4 ranges"
          },
          {
            "input": "address 0.0.0.0 with sing-box proxy detour",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: parses but matches none of the listed ranges"
          },
          {
            "input": "address fe80::1%eth0 (zone id) with sing-box proxy detour",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: std IpAddr parse rejects zone ids -> treated as non-literal"
          },
          {
            "input": "invalid address (999.1.1.1, empty, whitespace-padded trimmed to a valid private IP)",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: parse failure -> not flagged, no panic; trimmed input parsed"
          }
        ],
        "forbidden": [
          "flagging a hostname-addressed server",
          "flagging any server under BackendType::V2ray",
          "flagging a public IP literal",
          "panic or unwrap on an unparseable address"
        ],
        "seeding": [
          "construct DnsServerConfig { tag: \"domestic\", protocol: DnsProtocol::Udp, address, port: None, detour } with literal strings directly in the test function and call resolves_via_proxy_private(BackendType::SingBox|Xray|V2ray, true|false); no persistence, no config generation, no GTK"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "crates/core/src/models/dns.rs #[cfg(test)] (suite: timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4): fn test_resolves_via_proxy_private_matrix — table over 127.0.0.1, ::1, 10.1.2.3, 172.20.0.1, 192.168.1.1, 169.254.1.1, fe80::1, fd00::1 (flagged) and 1.1.1.1, dns.google (not flagged) x SingBox (proxy/direct/no detour) x Xray (tun on/off x proxy/direct/no detour) x V2ray (always false), matching all four dns-configuration scenarios; asserts the flagged/not-flagged expectation per row",
        "same module: fn test_resolves_via_proxy_private_address_edges — ::ffff:127.0.0.1 with SingBox proxy detour is flagged (mapped loopback), 0.0.0.0 is not, fe80::1%eth0 is not, \" 10.0.0.1 \" trims and is flagged, \"999.1.1.1\" and \"\" are not and do not panic",
        "implement resolves_via_proxy_private + private_dns_ip exactly per the summary ruling; no changes to DnsConfig::validate (warn, never reject)"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust — no test-writer agent exists for this stack"
    },
    {
      "id": "c2-generator-warnings",
      "taskIds": [
        "2.1"
      ],
      "prev": "c1-predicate",
      "sharedPkg": "v2ray-rs-core",
      "parallel": true,
      "seam": "generator-private-dns-warning",
      "shard": "gen",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "SingboxGenerator::build_dns",
          "lines": "458-466",
          "anchor": "        // Only a proxy detour is expressible. `detour: \"direct\"` names the\n        // empty direct outbound, which sing-box refuses to start against\n        // (\"detour to an empty direct outbound makes no sense\") while\n        // `sing-box check` accepts it; a server with no detour is not\n        // dispatched through the proxy chain in the first place.\n        if server_cfg.detour.is_some() && !server_cfg.detours_direct() {\n            server[\"detour\"] = json!(first_proxy_tag);\n        }",
          "change": "log one warning naming tag and address for flagged private DNS server during config generation; also applied in crates/core/src/config/v2ray.rs"
        }
      ],
      "contract": {
        "states": [
          "flagged",
          "not-flagged"
        ],
        "transitions": [
          {
            "input": "generate() on sing-box with a flagged server in settings.dns.servers",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:463-465 + delta spec dns-configuration scenario 'Loopback server with proxy detour on sing-box' (warning naming domestic and 127.0.0.1, generation succeeds)"
          },
          {
            "input": "generate on xray with TUN on and a flagged server (detour proxy or None)",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:853 + 827-829"
          },
          {
            "input": "generate on the v2ray family backend with a private server",
            "state": "not-flagged",
            "effect": "forced",
            "evidence": "delta spec dns-configuration requirement 1 last sentence"
          },
          {
            "input": "generate with no flagged servers (public, hostname, or direct detour)",
            "state": "not-flagged",
            "effect": "no-op",
            "evidence": "task 2.1: one warning per flagged server only"
          }
        ],
        "forbidden": [
          "generation returns Err or alters the emitted config because a server is flagged",
          "more than one warning line per flagged server per generate call",
          "a warning for a hostname-addressed or public server"
        ],
        "seeding": [
          "per generator test module: the existing default_settings() fixture plus one ProxyNode (pattern at singbox.rs:1617-1621 and v2ray.rs:1989-1990); set settings.dns.enabled = true and settings.dns.servers = vec![DnsServerConfig { tag: \"domestic\", protocol: DnsProtocol::Udp, address: \"127.0.0.1\", port: None, detour: Some(\"proxy\") }]; the xray case additionally sets settings.tun.enabled = true"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "crates/core/src/config/singbox.rs tests (suite: timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4): fn test_singbox_generation_flags_private_proxy_detoured_server — the domestic server reports resolves_via_proxy_private(BackendType::SingBox, false) == true, SingboxGenerator.generate(...) returns Ok, and the emitted server entry carries detour == the first proxy tag (generation unaffected)",
        "crates/core/src/config/v2ray.rs tests (same suite): fn test_xray_generation_flags_private_proxy_detoured_server — same server with settings.tun.enabled = true; predicate true with detour Some(\"proxy\") and with detour None, and generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::Xray) still returns a config containing the server entry",
        "crates/core/src/config/v2ray.rs tests (same suite): fn test_v2ray_family_never_flags_private_server — the same server under BackendType::V2ray is never flagged, and under BackendType::Xray with detour Some(\"direct\") and TUN on is not flagged (it is tagged dns-direct, v2ray.rs:848-849)",
        "add the two log::warn! loops per the summary; all assertions go through the predicate call, not log capture (task 2.1)"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust — no test-writer agent exists for this stack"
    },
    {
      "id": "c3-dialog-and-rows",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "c1-predicate",
      "sharedPkg": null,
      "parallel": true,
      "seam": "ui-private-dns-warning",
      "shard": "ui",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "show_dns_server_dialog",
          "lines": "1290-1318",
          "anchor": "    let warning_label = gtk::Label::builder()\n        .label(\"\")\n        .wrap(true)\n        .xalign(0.0)\n        .halign(gtk::Align::Start)\n        .visible(false)\n        .build();\n\n    content.append(&group);\n    content.append(&error_label);\n    content.append(&warning_label);",
          "change": "show private-server warning below downgrade warning in server dialog, refreshed on address, detour, protocol, and backend changes while keeping Save enabled"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "render_dns_servers",
          "lines": "938-948",
          "anchor": "        if let Some(fallback) = server.protocol.fallback_protocol_for_backend(backend) {\n            subtitle.push_str(&format!(\n                \"\\nDowngraded to {} on {}\",\n                protocol_display_name(fallback),\n                backend_display_name(backend)\n            ));\n        }\n\n        let row = adw::ActionRow::builder()\n            .title(&server.tag)\n            .subtitle(&subtitle)\n            .build();",
          "change": "append private-server warning to subtitle in render_dns_servers and render_primary_dns_servers for flagged servers"
        }
      ],
      "contract": {
        "states": [
          "warning-shown",
          "warning-absent"
        ],
        "transitions": [
          {
            "input": "dialog inputs: private IP literal + detour proxy (or no detour entry on xray TUN) on sing-box",
            "state": "warning-shown",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Warning while editing' (Save stays enabled)"
          },
          {
            "input": "detour switched to direct, or address becomes public/hostname, or backend becomes v2ray",
            "state": "warning-absent",
            "effect": "clear",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Warning clears on direct detour'"
          },
          {
            "input": "saved flagged server rendered in the servers list or the primary section",
            "state": "warning-shown",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Saved flagged server marked'"
          },
          {
            "input": "protocol combo changed",
            "state": "warning-shown",
            "effect": "no-op",
            "evidence": "task 3.1 requires refresh on protocol change; predicate ignores protocol, recompute yields the same state"
          },
          {
            "input": "backend or TUN changed while the DNS page is open",
            "state": "warning-shown",
            "effect": "no-op",
            "evidence": "observer re-render on backend diff exists at dns.rs:503-507; extended by last_tun so rows and note never go stale"
          }
        ],
        "forbidden": [
          "the private-server warning disabling the Save response (dialog.set_response_enabled(false))",
          "row or primary text differing from PRIVATE_DNS_WARNING",
          "a warning rendered under the v2ray backend"
        ],
        "seeding": [
          "pure-helper tests construct DnsServerConfig literals plus BackendType and bool flags directly — no widgets, no gtk_test::run, plain #[test] in the existing tests mod (dns.rs:1821+); the live dialog surface is exercised by manual acceptance 4.2"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "crates/ui/src/preferences/dns.rs tests (suite: timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4, plain #[test]): fn test_private_dns_warning_text — flagged input (127.0.0.1, Some(\"proxy\"), BackendType::SingBox) returns Some whose text contains \"proxy server's network\" and \"direct\"; unflagged inputs (detour Some(\"direct\"), address 1.1.1.1, address dns.google, BackendType::V2ray) return None",
        "same module: fn test_flagged_private_server_still_validates — AppSettings holding the flagged domestic server passes validate_dns_settings_for_backend (the exact fn the Save path calls at dns.rs:1351), proving saving remains allowed",
        "same module: fn test_server_row_subtitle_appends_private_warning — server_row_subtitle contains the scheme://address:port base and appends PRIVATE_DNS_WARNING for a flagged server, omits it for detour direct",
        "same module: fn test_primary_row_subtitle_appends_private_warning — same assertions for primary_dns_subtitle",
        "wiring per the summary: private_warning_label after warning_label, update_private_warning closure + hooks, the two subtitle helpers consumed by render_dns_servers/render_primary_dns_servers, last_tun guard in the settings observer"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust — no test-writer agent exists for this stack"
    },
    {
      "id": "c4-strategy-note",
      "taskIds": [
        "3.3"
      ],
      "prev": "c3-dialog-and-rows",
      "sharedPkg": "v2ray-rs-ui",
      "parallel": false,
      "seam": "strategy-family-note",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "sync_dns_ui",
          "lines": "495-508",
          "anchor": "        let sync_dns_ui_observer = sync_dns_ui.clone();\n        let ctx = dns_ctx.clone();\n        let last_backend = Rc::new(RefCell::new(ctx.state.borrow().backend.backend_type));\n        subscribe_settings(settings_observers, move |settings| {\n            let backend = settings.backend.backend_type;\n            sync_dns_ui_observer(settings);\n            if backend != *last_backend.borrow() {\n                *last_backend.borrow_mut() = backend;\n                render_dns_servers(&ctx);\n                render_primary_dns_servers(&ctx);\n            }\n        });",
          "change": "set strategy row subtitle for xray/v2ray noting Prefer options query only preferred family, none for sing-box, refreshed on backend change"
        }
      ],
      "contract": {
        "states": [
          "note-visible",
          "note-hidden"
        ],
        "transitions": [
          {
            "input": "active backend set to xray",
            "state": "note-visible",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 3 scenario 'xray strategy note'"
          },
          {
            "input": "active backend set to v2ray",
            "state": "note-visible",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 3 ('when the active backend is xray or v2ray')"
          },
          {
            "input": "active backend set to sing-box",
            "state": "note-hidden",
            "effect": "clear",
            "evidence": "delta spec dns-preferences-ui requirement 3 scenario 'sing-box shows no note'"
          },
          {
            "input": "strategy selection changed within a backend",
            "state": "note-visible",
            "effect": "no-op",
            "evidence": "requirement 3: four options and stored values unchanged; note depends only on backend"
          }
        ],
        "forbidden": [
          "the four strategy options, their order, or the stored DnsStrategy value changing",
          "any note shown for BackendType::SingBox"
        ],
        "seeding": [
          "call strategy_family_note(BackendType::Xray | V2ray | SingBox) directly in a plain #[test]; no widgets — the live refresh path is exercised by manual acceptance 4.3"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "crates/ui/src/preferences/dns.rs tests (suite: timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4, plain #[test]): fn test_strategy_family_note_per_backend — Xray and V2ray return Some containing \"preferred address family\", SingBox returns None",
        "strategy_row.set_subtitle wiring in sync_dns_ui per the summary, refreshed by the existing subscribe_settings observer"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust — no test-writer agent exists for this stack"
    }
  ],
  "seams": [
    {
      "id": "private-dns-predicate",
      "tasks": [
        "1.1"
      ],
      "summary": "Predicate ruling: pub fn resolves_via_proxy_private(&self, backend: BackendType, tun_enabled: bool) -> bool on DnsServerConfig (crates/core/src/models/dns.rs:81), a method impl beside detours_direct (dns.rs:91-97). BackendType is the crate's existing enum at crates/core/src/models/settings.rs:15 (variants V2ray, Xray, SingBox), already imported into dns.rs at line 5. Body: first gate on the address — self.address.trim() must parse as IpAddr and fall in 127.0.0.0/8, ::1, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, fe80::/10 or fc00::/7, via a private helper fn private_dns_ip(s: &str) -> Option<IpAddr> that normalizes IPv4-mapped IPv6 through to_ipv4_mapped() and classifies the mapped v4 by the v4 ranges (std methods is_loopback/is_private/is_link_local for v4, two bit-mask checks for fe80::/10 and fc00::/7 — no per-call string parsing of nets; ipnet is already a core dep, dns.rs:1, if a const-net form is preferred). Then match backend: V2ray => false; SingBox => self.detour.is_some() && !self.detours_direct() — exactly the condition singbox.rs:463 uses to emit detour: <first proxy>; Xray => !(tun_enabled && self.detours_direct()) — exactly direct_detour at v2ray.rs:827-829. xray TUN-off ruling: without TUN no server can be expressed as directly routed — dns.tag and the dns-internal routing rule are emitted only when backend==Xray && tun.enabled (v2ray.rs:85, 116-117, 524-538), there is no per-server detour field (v2ray.rs:824-825 comment), and untagged built-in-resolver traffic falls through to the first outbound (the proxy), so the emitted config makes all servers indistinguishable and every private-literal one is flagged. Address-parsing ruling: IP literal = whatever std str::parse::<IpAddr>() accepts on the trimmed string; ::ffff:127.0.0.1 counts and is classified as loopback via the mapped v4; 0.0.0.0 parses but matches no listed range -> not flagged; fe80::1%eth0 fails std parsing (zone ids unsupported) -> treated as non-literal -> not flagged (conservative false negative); hostnames and bare invalid strings -> not flagged, never panic. NO-RED-WAIVER: Rust — no test-writer agent exists for this stack. NO-TESTER-WAIVER: Rust — the coder writes the tests and runs the crate suite. Suite: v2ray-rs-core.",
      "contract": {
        "states": [
          "flagged",
          "not-flagged"
        ],
        "transitions": [
          {
            "input": "sing-box, private literal in any of the four range families, detour Some(non-\"direct\") incl. legacy sentinel \"proxy-0\"",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:463-465"
          },
          {
            "input": "sing-box, private literal, detour None",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/singbox.rs:461-462 (a server with no detour is not dispatched through the proxy chain)"
          },
          {
            "input": "sing-box, private literal, detour \"direct\"",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/singbox.rs:463 + existing test test_dns_server_direct_detour_is_not_emitted at singbox.rs:2003-2023"
          },
          {
            "input": "xray, private literal, tun on, detour \"direct\"",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "crates/core/src/config/v2ray.rs:827-829 (direct_detour true -> entry tagged dns-direct, v2ray.rs:848-849)"
          },
          {
            "input": "xray, private literal, tun on, detour \"proxy\" or detour None",
            "state": "flagged",
            "effect": "set",
            "evidence": "delta spec dns-configuration scenario 'Loopback server without direct routing on xray' + v2ray.rs:524-538"
          },
          {
            "input": "xray, private literal, tun off, any detour including \"direct\"",
            "state": "flagged",
            "effect": "set",
            "evidence": "v2ray.rs:85, 116-117, 524-538, 827-829 — nothing expressible as directly routed without TUN; ruled flagged"
          },
          {
            "input": "v2ray, any address, any detour, any tun state",
            "state": "not-flagged",
            "effect": "forced",
            "evidence": "delta spec dns-configuration requirement 1 last sentence: 'Hostname-addressed servers and the v2ray backend SHALL NOT be flagged'"
          },
          {
            "input": "any backend, public literal e.g. 1.1.1.1, detour proxy",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "delta spec dns-configuration scenario 'Public server is not flagged'"
          },
          {
            "input": "any backend, hostname e.g. dns.google",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "delta spec dns-configuration requirement 1 last sentence"
          },
          {
            "input": "address ::ffff:127.0.0.1 (IPv4-mapped) with sing-box proxy detour",
            "state": "flagged",
            "effect": "set",
            "evidence": "ruling: mapped v4 normalized via to_ipv4_mapped and classified by the v4 ranges"
          },
          {
            "input": "address 0.0.0.0 with sing-box proxy detour",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: parses but matches none of the listed ranges"
          },
          {
            "input": "address fe80::1%eth0 (zone id) with sing-box proxy detour",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: std IpAddr parse rejects zone ids -> treated as non-literal"
          },
          {
            "input": "invalid address (999.1.1.1, empty, whitespace-padded trimmed to a valid private IP)",
            "state": "not-flagged",
            "effect": "clear",
            "evidence": "ruling: parse failure -> not flagged, no panic; trimmed input parsed"
          }
        ],
        "forbidden": [
          "flagging a hostname-addressed server",
          "flagging any server under BackendType::V2ray",
          "flagging a public IP literal",
          "panic or unwrap on an unparseable address"
        ],
        "seeding": [
          "construct DnsServerConfig { tag: \"domestic\", protocol: DnsProtocol::Udp, address, port: None, detour } with literal strings directly in the test function and call resolves_via_proxy_private(BackendType::SingBox|Xray|V2ray, true|false); no persistence, no config generation, no GTK"
        ]
      },
      "codeTasks": [
        "crates/core/src/models/dns.rs #[cfg(test)] (suite: timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4): fn test_resolves_via_proxy_private_matrix — table over 127.0.0.1, ::1, 10.1.2.3, 172.20.0.1, 192.168.1.1, 169.254.1.1, fe80::1, fd00::1 (flagged) and 1.1.1.1, dns.google (not flagged) x SingBox (proxy/direct/no detour) x Xray (tun on/off x proxy/direct/no detour) x V2ray (always false), matching all four dns-configuration scenarios; asserts the flagged/not-flagged expectation per row",
        "same module: fn test_resolves_via_proxy_private_address_edges — ::ffff:127.0.0.1 with SingBox proxy detour is flagged (mapped loopback), 0.0.0.0 is not, fe80::1%eth0 is not, \" 10.0.0.1 \" trims and is flagged, \"999.1.1.1\" and \"\" are not and do not panic",
        "implement resolves_via_proxy_private + private_dns_ip exactly per the summary ruling; no changes to DnsConfig::validate (warn, never reject)"
      ]
    },
    {
      "id": "generator-private-dns-warning",
      "tasks": [
        "2.1"
      ],
      "summary": "Both generators call the seam-1 predicate on every configured server and emit exactly one log::warn! per flagged server during generation, with an identical format string in both files: log::warn!(\"DNS server {} ({}) is private and routed through the proxy; its queries reach the proxy server's network - consider detour 'direct'\", server.tag, server.address) — names the tag and the address as scenario 1 requires. sing-box: inside the existing server loop of build_dns (crates/core/src/config/singbox.rs:402, loop body around 458-465), gated resolves_via_proxy_private(BackendType::SingBox, false). xray: in build_user_dns_servers (crates/core/src/config/v2ray.rs:853) only when backend == V2rayFamilyBackend::Xray, passing the same tun_xray the entry builder uses (v2ray.rs:85); the v2ray family backend never logs (predicate forced false). Precedent for log::warn! in generators: hosts_for_strategy at v2ray.rs:769-781. The generated JSON is byte-identical to today's — the warning is observational, generation never returns Err for a flagged server. NO-RED-WAIVER: Rust — no test-writer agent exists for this stack. NO-TESTER-WAIVER: Rust — the coder writes the tests and runs the crate suite. Suite: v2ray-rs-core.",
      "contract": {
        "states": [
          "flagged",
          "not-flagged"
        ],
        "transitions": [
          {
            "input": "generate() on sing-box with a flagged server in settings.dns.servers",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:463-465 + delta spec dns-configuration scenario 'Loopback server with proxy detour on sing-box' (warning naming domestic and 127.0.0.1, generation succeeds)"
          },
          {
            "input": "generate on xray with TUN on and a flagged server (detour proxy or None)",
            "state": "flagged",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:853 + 827-829"
          },
          {
            "input": "generate on the v2ray family backend with a private server",
            "state": "not-flagged",
            "effect": "forced",
            "evidence": "delta spec dns-configuration requirement 1 last sentence"
          },
          {
            "input": "generate with no flagged servers (public, hostname, or direct detour)",
            "state": "not-flagged",
            "effect": "no-op",
            "evidence": "task 2.1: one warning per flagged server only"
          }
        ],
        "forbidden": [
          "generation returns Err or alters the emitted config because a server is flagged",
          "more than one warning line per flagged server per generate call",
          "a warning for a hostname-addressed or public server"
        ],
        "seeding": [
          "per generator test module: the existing default_settings() fixture plus one ProxyNode (pattern at singbox.rs:1617-1621 and v2ray.rs:1989-1990); set settings.dns.enabled = true and settings.dns.servers = vec![DnsServerConfig { tag: \"domestic\", protocol: DnsProtocol::Udp, address: \"127.0.0.1\", port: None, detour: Some(\"proxy\") }]; the xray case additionally sets settings.tun.enabled = true"
        ]
      },
      "codeTasks": [
        "crates/core/src/config/singbox.rs tests (suite: timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4): fn test_singbox_generation_flags_private_proxy_detoured_server — the domestic server reports resolves_via_proxy_private(BackendType::SingBox, false) == true, SingboxGenerator.generate(...) returns Ok, and the emitted server entry carries detour == the first proxy tag (generation unaffected)",
        "crates/core/src/config/v2ray.rs tests (same suite): fn test_xray_generation_flags_private_proxy_detoured_server — same server with settings.tun.enabled = true; predicate true with detour Some(\"proxy\") and with detour None, and generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::Xray) still returns a config containing the server entry",
        "crates/core/src/config/v2ray.rs tests (same suite): fn test_v2ray_family_never_flags_private_server — the same server under BackendType::V2ray is never flagged, and under BackendType::Xray with detour Some(\"direct\") and TUN on is not flagged (it is tagged dns-direct, v2ray.rs:848-849)",
        "add the two log::warn! loops per the summary; all assertions go through the predicate call, not log capture (task 2.1)"
      ]
    },
    {
      "id": "ui-private-dns-warning",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "One pure function produces the string all three surfaces render: fn private_dns_warning(server: &DnsServerConfig, backend: BackendType, tun_enabled: bool) -> Option<&'static str> in crates/ui/src/preferences/dns.rs — returns Some(PRIVATE_DNS_WARNING) iff server.resolves_via_proxy_private(backend, tun_enabled), None otherwise. The const lives next to it in the same file: const PRIVATE_DNS_WARNING: &str = \"Queries to this private address go through the proxy and reach the proxy server's network, not yours. Set Detour to 'direct' if this resolver is on your network.\" Dialog (3.1): a new gtk::Label private_warning_label built like the existing downgrade warning_label and appended directly after it (ui dns.rs:1296-1305), driven by a new update_private_warning closure that builds a scratch DnsServerConfig from address_entry + detour_combo and re-reads backend and ctx.state.borrow().tun.enabled fresh on every call (covers backend change), hooked to address_entry.connect_changed, detour_combo.connect_selected_notify and protocol_combo.connect_selected_notify plus one initial call (existing hook pattern at dns.rs:1374-1390); it never touches dialog.set_response_enabled — Save stays governed solely by update_validation (dns.rs:1355-1372), so saving a flagged server remains allowed. Rows (3.2): subtitle construction moves into pure helpers server_row_subtitle(server, backend, tun_enabled) -> String (used by render_dns_servers, base built at dns.rs:936-941, downgrade append at 942-948) and primary_dns_subtitle(server, backend, tun_enabled) -> String (used by render_primary_dns_servers, dns.rs:1063-1107); each appends \"\\n\" + PRIVATE_DNS_WARNING when private_dns_warning is Some. The existing settings observer (the subscribe_settings callback at dns.rs:498-507 that diffs last_backend and re-renders rows at 505-506) gains a parallel last_tun guard so rows re-render on backend or TUN change. NO-RED-WAIVER: Rust — no test-writer agent exists for this stack. NO-TESTER-WAIVER: Rust — the coder writes the tests and runs the crate suite. Suite: v2ray-rs-ui.",
      "contract": {
        "states": [
          "warning-shown",
          "warning-absent"
        ],
        "transitions": [
          {
            "input": "dialog inputs: private IP literal + detour proxy (or no detour entry on xray TUN) on sing-box",
            "state": "warning-shown",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Warning while editing' (Save stays enabled)"
          },
          {
            "input": "detour switched to direct, or address becomes public/hostname, or backend becomes v2ray",
            "state": "warning-absent",
            "effect": "clear",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Warning clears on direct detour'"
          },
          {
            "input": "saved flagged server rendered in the servers list or the primary section",
            "state": "warning-shown",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 2 scenario 'Saved flagged server marked'"
          },
          {
            "input": "protocol combo changed",
            "state": "warning-shown",
            "effect": "no-op",
            "evidence": "task 3.1 requires refresh on protocol change; predicate ignores protocol, recompute yields the same state"
          },
          {
            "input": "backend or TUN changed while the DNS page is open",
            "state": "warning-shown",
            "effect": "no-op",
            "evidence": "observer re-render on backend diff exists at dns.rs:503-507; extended by last_tun so rows and note never go stale"
          }
        ],
        "forbidden": [
          "the private-server warning disabling the Save response (dialog.set_response_enabled(false))",
          "row or primary text differing from PRIVATE_DNS_WARNING",
          "a warning rendered under the v2ray backend"
        ],
        "seeding": [
          "pure-helper tests construct DnsServerConfig literals plus BackendType and bool flags directly — no widgets, no gtk_test::run, plain #[test] in the existing tests mod (dns.rs:1821+); the live dialog surface is exercised by manual acceptance 4.2"
        ]
      },
      "codeTasks": [
        "crates/ui/src/preferences/dns.rs tests (suite: timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4, plain #[test]): fn test_private_dns_warning_text — flagged input (127.0.0.1, Some(\"proxy\"), BackendType::SingBox) returns Some whose text contains \"proxy server's network\" and \"direct\"; unflagged inputs (detour Some(\"direct\"), address 1.1.1.1, address dns.google, BackendType::V2ray) return None",
        "same module: fn test_flagged_private_server_still_validates — AppSettings holding the flagged domestic server passes validate_dns_settings_for_backend (the exact fn the Save path calls at dns.rs:1351), proving saving remains allowed",
        "same module: fn test_server_row_subtitle_appends_private_warning — server_row_subtitle contains the scheme://address:port base and appends PRIVATE_DNS_WARNING for a flagged server, omits it for detour direct",
        "same module: fn test_primary_row_subtitle_appends_private_warning — same assertions for primary_dns_subtitle",
        "wiring per the summary: private_warning_label after warning_label, update_private_warning closure + hooks, the two subtitle helpers consumed by render_dns_servers/render_primary_dns_servers, last_tun guard in the settings observer"
      ]
    },
    {
      "id": "strategy-family-note",
      "tasks": [
        "3.3"
      ],
      "summary": "fn strategy_family_note(backend: BackendType) -> Option<&'static str> in crates/ui/src/preferences/dns.rs — depends only on the active backend: Some(\"Prefer IPv4 and Prefer IPv6 query only the preferred address family on this backend\") for BackendType::Xray and BackendType::V2ray, None for BackendType::SingBox. Applied as strategy_row.set_subtitle(...) inside the sync_dns_ui closure (ui dns.rs:469-493) — adw::ComboRow supports set_subtitle, precedent detour_combo.set_subtitle at dns.rs:1276-1278 — and the existing settings observer that invokes sync_dns_ui on every settings push (the subscribe_settings callback, dns.rs:498-507) is the refresh path on backend change, so no new observer is needed. The selector's model, the four option labels and the stored DnsStrategy values are untouched (strategy_to_index/index_to_strategy stay as-is, dns.rs:613-629). NO-RED-WAIVER: Rust — no test-writer agent exists for this stack. NO-TESTER-WAIVER: Rust — the coder writes the tests and runs the crate suite. Suite: v2ray-rs-ui.",
      "contract": {
        "states": [
          "note-visible",
          "note-hidden"
        ],
        "transitions": [
          {
            "input": "active backend set to xray",
            "state": "note-visible",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 3 scenario 'xray strategy note'"
          },
          {
            "input": "active backend set to v2ray",
            "state": "note-visible",
            "effect": "set",
            "evidence": "delta spec dns-preferences-ui requirement 3 ('when the active backend is xray or v2ray')"
          },
          {
            "input": "active backend set to sing-box",
            "state": "note-hidden",
            "effect": "clear",
            "evidence": "delta spec dns-preferences-ui requirement 3 scenario 'sing-box shows no note'"
          },
          {
            "input": "strategy selection changed within a backend",
            "state": "note-visible",
            "effect": "no-op",
            "evidence": "requirement 3: four options and stored values unchanged; note depends only on backend"
          }
        ],
        "forbidden": [
          "the four strategy options, their order, or the stored DnsStrategy value changing",
          "any note shown for BackendType::SingBox"
        ],
        "seeding": [
          "call strategy_family_note(BackendType::Xray | V2ray | SingBox) directly in a plain #[test]; no widgets — the live refresh path is exercised by manual acceptance 4.3"
        ]
      },
      "codeTasks": [
        "crates/ui/src/preferences/dns.rs tests (suite: timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4, plain #[test]): fn test_strategy_family_note_per_backend — Xray and V2ray return Some containing \"preferred address family\", SingBox returns None",
        "strategy_row.set_subtitle wiring in sync_dns_ui per the summary, refreshed by the existing subscribe_settings observer"
      ]
    },
    {
      "id": "verification-coverage",
      "tasks": [
        "3.4",
        "4.1",
        "4.2",
        "4.3"
      ],
      "summary": "Tasks 3.4 and 4.1 are satisfied by the run's own commands and need no chunk: 3.4 is the per-chunk ui verify (timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4) after c3 and c4, 4.1 is covered by the floor (make lint && make test) once after all chunks land. Tasks 4.2 and 4.3 are LIVE manual acceptance no agent can execute — kept out of chunks, steps named in risks. NO-RED-WAIVER: Rust — no test-writer agent exists for this stack. NO-TESTER-WAIVER: Rust — the coder writes the tests and runs the crate suite.",
      "contract": {
        "states": [
          "all-green"
        ],
        "transitions": [
          {
            "input": "make lint && make test after all chunks",
            "state": "all-green",
            "effect": "no-op",
            "evidence": "tasks.md 4.1 + repo Makefile (fmt --check, clippy -D warnings, timeout 5m cargo test --workspace --all-targets -- --test-threads=4)"
          },
          {
            "input": "manual 4.2 executed by the user",
            "state": "all-green",
            "effect": "no-op",
            "evidence": "tasks.md 4.2 — live GUI + real connection + app log inspection"
          },
          {
            "input": "manual 4.3 executed by the user",
            "state": "all-green",
            "effect": "no-op",
            "evidence": "tasks.md 4.3 — live backend switching on the DNS page"
          }
        ],
        "forbidden": [
          "a chunk marked done without its per-crate verify command green",
          "the floor run skipped or truncated"
        ],
        "seeding": [
          "clean checkout at baseSha 551b47811d8f with chunks c1-c4 applied in dependency order (c2, c3 behind c1; c4 behind c3)"
        ]
      }
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Private DNS servers routed through the proxy are flagged The system SHALL identify a DNS server whose address is an IP literal in a loopback (`127.0.0.0/8`, `::1`), private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local (`169.254.0.0/16`, `fe80::/10`), or unique-local (`fc00::/7`) range and whose queries the generated config sends through the proxy: on sing-box, a server with a detour other than \"direct\"; on xray, any server not expressed as directly routed (a \"direct\" detour under TUN). Such a server reaches the proxy server's network, not the user's. The system SHALL NOT reject such a server, because a resolver on the proxy server's own network is a valid setup; config generation SHALL log one warning per flagged server naming its tag and address. Hostname-addressed servers and the v2ray backend SHALL NOT be flagged.",
      "tests": [
        "test_resolves_via_proxy_private_matrix",
        "test_resolves_via_proxy_private_address_edges"
      ]
    },
    {
      "shall": "- **THEN** the server SHALL be flagged, config generation SHALL succeed, and a warning naming `domestic` and `127.0.0.1` SHALL be logged",
      "tests": [
        "test_resolves_via_proxy_private_matrix",
        "test_resolves_via_proxy_private_address_edges"
      ]
    },
    {
      "shall": "- **THEN** the server SHALL be flagged",
      "tests": [
        "test_resolves_via_proxy_private_matrix",
        "test_resolves_via_proxy_private_address_edges"
      ]
    },
    {
      "shall": "- **THEN** the server SHALL NOT be flagged",
      "tests": [
        "test_resolves_via_proxy_private_matrix",
        "test_resolves_via_proxy_private_address_edges"
      ]
    },
    {
      "shall": "- **THEN** the server SHALL NOT be flagged",
      "tests": [
        "test_resolves_via_proxy_private_matrix",
        "test_resolves_via_proxy_private_address_edges"
      ]
    },
    {
      "shall": "### Requirement: Warning for private DNS servers routed through the proxy The DNS server dialog SHALL show a non-blocking inline warning while the entered address, detour, and active backend make the server flagged as a private DNS server routed through the proxy, stating that queries will reach the proxy server's network and suggesting detour `direct`. The server's row in the list and in the primary section SHALL show the same warning. Saving SHALL remain allowed.",
      "tests": [
        "test_private_dns_warning_text",
        "test_flagged_private_server_still_validates",
        "test_server_row_subtitle_appends_private_warning",
        "test_primary_row_subtitle_appends_private_warning"
      ]
    },
    {
      "shall": "- **THEN** the dialog SHALL show the warning and the Save response SHALL stay enabled",
      "tests": [
        "test_private_dns_warning_text",
        "test_flagged_private_server_still_validates",
        "test_server_row_subtitle_appends_private_warning",
        "test_primary_row_subtitle_appends_private_warning"
      ]
    },
    {
      "shall": "- **THEN** the warning SHALL disappear",
      "tests": [
        "test_private_dns_warning_text",
        "test_flagged_private_server_still_validates",
        "test_server_row_subtitle_appends_private_warning",
        "test_primary_row_subtitle_appends_private_warning"
      ]
    },
    {
      "shall": "- **THEN** its row SHALL show the warning text",
      "tests": [
        "test_private_dns_warning_text",
        "test_flagged_private_server_still_validates",
        "test_server_row_subtitle_appends_private_warning",
        "test_primary_row_subtitle_appends_private_warning"
      ]
    },
    {
      "shall": "### Requirement: Strategy options reflect the backend The IP strategy selector SHALL keep its four options and stored values, and when the active backend is xray or v2ray SHALL show a note that \"Prefer IPv4\" and \"Prefer IPv6\" query only the preferred address family on that backend. For sing-box no note SHALL be shown.",
      "tests": [
        "test_strategy_family_note_per_backend"
      ]
    },
    {
      "shall": "- **THEN** the strategy row SHALL show the note that Prefer options use only the preferred family",
      "tests": [
        "test_strategy_family_note_per_backend"
      ]
    },
    {
      "shall": "- **THEN** the strategy row SHALL show no family note",
      "tests": [
        "test_strategy_family_note_per_backend"
      ]
    }
  ],
  "testHarness": [
    "default_settings — crates/core/src/config/test_fixtures.rs:5 — AppSettings with default values",
    "vless_node — crates/core/src/config/test_fixtures.rs:9 — ProxyNode::Vless sample node fixture",
    "vmess_node — crates/core/src/config/test_fixtures.rs:27 — ProxyNode::Vmess sample node fixture",
    "ss_node — crates/core/src/config/test_fixtures.rs:39 — ProxyNode::Shadowsocks sample node fixture",
    "trojan_node — crates/core/src/config/test_fixtures.rs:49 — ProxyNode::Trojan sample node fixture",
    "build_dns — crates/core/src/config/v2ray.rs:660 — builds v2ray DNS JSON Value fixture from rules and settings",
    "dns_server_with_tag — crates/core/src/config/v2ray.rs:2476 — finds first DNS server JSON object by tag in config",
    "dns_servers_with_tag — crates/core/src/config/v2ray.rs:2480 — finds all DNS server JSON objects by tag in config",
    "rule_index_with_outbound_tag — crates/core/src/config/v2ray.rs:2469 — finds index of routing rule matching outboundTag",
    "rule_index_with_inbound_tag — crates/core/src/config/v2ray.rs:2493 — finds index of routing rule matching inboundTag",
    "detour_note — crates/ui/src/preferences/dns.rs:700 — returns Option<&'static str> describing detour capabilities for BackendType",
    "without_handlers — crates/ui/src/preferences/dns.rs:607 — executes closure with signal suppression guard active to prevent widget re-entrancy",
    "run — crates/ui/src/gtk_test.rs:13 — executes closure on dedicated GTK thread, skipping gracefully if display unavailable"
  ],
  "floor": "make lint && make test",
  "estimateHours": 1.5,
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1
  }
}
```
