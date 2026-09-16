## Context

- `crates/core/src/config/v2ray.rs:189` emits `"autoOutboundsInterface": "auto"` in `build_xray_tun_inbound`; the canonical specs still require it (`openspec/specs/tun-mode/spec.md`, requirement `Outbound loop prevention`; `openspec/specs/config-generator/spec.md`, requirement `Generate xray TUN inbound`) until this change is archived.
- Xray-core v26.9.9, Linux: `proxy/tun/handler.go` `Start` → `InterfaceUpdater` + `internet.RegisterDialerController` calling `setinterface` = `unix.BindToDevice(fd, iface.Name)` for every dial except loopback; `findOutboundInterface` picks the lowest-metric default route whose link is not the TUN and is refreshed on netlink route/link updates (`monitorRouteChanges`). Active on Linux since v26.6.27.
- `crates/core/src/config/xray.rs:64-74` `apply_tun_fwmark` stamps `sockopt.mark = 255` on every outbound except `blackhole` and `dns`; netctl installs `fwmark 0xff lookup main` at pref 9000 (`crates/netctl/src/net.rs:42`). That alone keeps xray's sockets off table 2023.
- Host evidence 2026-09-15: `ip route get <vpn-dns-ip> mark 255` → `via 192.168.220.1 dev tun0` (correct), yet xray's `[tun-in -> direct]` query to `<vpn-dns-ip>:53` came back as a 4 ms forged `NXDOMAIN` with a bad cookie — the socket egressed `wlan0`.

## Goals / Non-Goals

**Goals:**
- xray's direct and proxy egress uses the same routes the host would use for that destination.

**Non-Goals:**
- sing-box: `auto_detect_interface` binds similarly, but sing-box TUN is not currently usable on the affected host (`fix-singbox-tun-without-ipv6`); verify and address separately.
- Changing which traffic is captured into the tunnel (see `route-xray-exclusions-outside-tun`).

## Decisions

- **Rely on the fwmark, drop the binding.** The mark already implements loop prevention through policy routing, and policy routing respects every route in `main`. Alternative rejected: `autoOutboundsInterface: "<fixed name>"` — still one interface, same failure. Alternative rejected: netctl rules steering VPN subnets around xray only — fixes captured DNS but not xray-routed direct traffic to those subnets.
- **Keep the mark on every dialing outbound as a hard invariant.** A test asserts each non-blackhole/non-dns outbound in an xray TUN config carries mark 255, since it becomes the only loop guard.

## Risks / Trade-offs

- [An xray dial path that bypasses `sockopt` (no mark) now loops into the TUN instead of being pinned to the uplink] → the invariant test covers generated outbounds; the built-in resolver's queries leave through tagged outbounds (`dns-internal`/`dns-direct` routing), which are marked. Live check: TUN session traffic flows and no `tun-in` lines show xray's own node address.
- [Default-route change mid-session (Wi-Fi roam) previously re-bound sockets automatically] → with policy routing, new sockets follow the new `main` default without any binding; existing connections break either way.

## Implementation plan

Tier `light`, mode `existing-service-strict`, lenses `spec`. Planned at `1d6a0fb8`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.


### `x1` — tasks 1.1 — seam `tun-inbound-shape`

- order: parallel, shard `generator`; coder `zpatcher`
- sites:
  - `crates/core/src/config/v2ray.rs` · `build_xray_tun_inbound` · anchor `"autoOutboundsInterface": "auto",` — Delete the key from the tun inbound `settings` json! block; `name`/`mtu`/`gateway` stay.
  - `crates/core/src/config/v2ray.rs` · `tests::test_xray_tun_inbound_emitted_when_enabled` · anchor `assert_eq!(tun["settings"]["autoOutboundsInterface"], "auto");` — Flip to asserting absence, e.g. assert!(tun["settings"].get("autoOutboundsInterface").is_none()).
  - `crates/core/src/config/xray.rs` · `tests::test_xray_generator_emits_tun_inbound_when_enabled` · anchor `assert_eq!(tun["settings"]["autoOutboundsInterface"], "auto");` — Same flip to absence assertion (this file's only hit, inside the tun-inbound test).
- work, in order:
  - Flip crates/core/src/config/v2ray.rs tests::test_xray_tun_inbound_emitted_when_enabled: replace `assert_eq!(tun["settings"]["autoOutboundsInterface"], "auto");` with `assert!(tun["settings"].get("autoOutboundsInterface").is_none());` and keep the name/mtu/gateway/sniffing assertions
  - Flip crates/core/src/config/xray.rs tests::test_xray_generator_emits_tun_inbound_when_enabled the same way; keep `assert_eq!(tun["sniffing"]["enabled"], true);`
  - Delete the line `"autoOutboundsInterface": "auto",` from the `settings` object of build_xray_tun_inbound in crates/core/src/config/v2ray.rs; `name`, `mtu`, `gateway` and the sniffing block are untouched
- verify: `make test-core && make lint   # test-core = timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4; lint = cargo fmt -- --check + cargo clippy --workspace --all-targets --all-features -- -D warnings`

### `x2` — tasks 1.2 — seam `fwmark-sole-guard-invariant`

- order: after `x1` (shared `crates/core/src/config`); coder `rust-coder`
- sites:
  - `crates/core/src/config/xray.rs` · `tests::test_xray_tun_marks_dialing_outbounds` · anchor `fn test_xray_tun_marks_dialing_outbounds() {` — Extend or add a sibling test building a multi-node + via_node-rule xray TUN config and asserting every outbound whose protocol is not blackhole/dns has streamSettings.sockopt.mark == 255 (covers node outbounds 0..n, via-node outbounds, `direct`).
- work, in order:
  - Add crates/core/src/config/xray.rs tests::test_xray_tun_marks_every_dialing_outbound (sibling of the existing test_xray_tun_marks_dialing_outbounds, which keeps its single-node coverage): build the multi-node + via-node-rule config per the seeding line, then iterate config["outbounds"] asserting mark == 255 for every outbound whose protocol is neither "blackhole" nor "dns", and is_null() for those two; include the outbound in the failure message as the neighbouring tests do
  - Add `use uuid::Uuid;` to the xray.rs tests mod
  - Assert the via-node tag actually resolved, locating the rule BY ITS MATCH and never by index: find the entry of config["routing"]["rules"] whose `domain` array contains the emitted, prefixed form `domain:api.example.com` (the generator renders RuleMatch::Domain { pattern } as `format!("domain:{pattern}")`, so a bare `api.example.com` never matches; no other rule in the tun-on fixture carries a `domain` array), and assert its `outboundTag` equals crate::config::common::outbound_tag(&nodes[1], 1). Index 0 is NOT the user rule — with backend xray and tun.enabled, build_routing prepends the optional dns-direct rule, the DNS_INTERNAL_TAG inboundTag rule, the exclusion rules and the udp/53 hijack ahead of every user rule.
- verify: `timeout 5m cargo test -p v2ray-rs-core marks_every_dialing_outbound -- --test-threads=4 && make test-core && make lint`

### `x3` — tasks 1.3 — seam `docs-loop-guard`

- order: parallel, shard `docs`; coder `zpatcher`
- sites:
  - `docs/ARCHITECTURE.md` · `netctl policy-rules list (pref 9000 bullet)` · anchor `9000 — `fwmark 0xff` → `main`, so xray's own sockets reach the real default` — State the fwmark + pref-9000 rule is the only xray loop guard; no interface binding on outbounds.
- work, in order:
  - In docs/ARCHITECTURE.md, extend the pref-9000 bullet under `xray-up` to state that the fwmark and this rule are the sole loop guard — xray outbounds are not bound to an interface, so they follow the whole `main` table including VPN routes
  - Read back the pref-9000 bullet and the `Bootstrap resolvers (xray)` paragraph to confirm they agree
- verify: `read back the pref-9000 bullet in docs/ARCHITECTURE.md; no command`

### `x4` — tasks 2.1, 2.2 — seam `verification`

- order: after `x2` (shared `crates/core/src/config`); coder `rust-coder`
- sites:
  - `crates/core/tests/xray_check.rs` · `xray_available` · anchor `fn xray_available` — Run the workspace floor; report whether xray was in PATH, since every test here silently skips when it is not.
- work, in order:
  - Run `make test TEST_TIMEOUT=10m`; report whether xray was in PATH (if not, state that crates/core/tests/xray_check.rs silently skipped)
  - Run `make lint` (cargo fmt -- --check, then cargo clippy --workspace --all-targets --all-features -- -D warnings)
  - Report 2.2 as MANUAL with the three checks spelled out for the operator; do not mark it done
- verify: `make test TEST_TIMEOUT=10m && make lint`

### Floor

make test TEST_TIMEOUT=10m && make lint   —   expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`, then `cargo fmt -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Task 2.2 stays MANUAL (needs a real xray backend, CAP_NET_ADMIN and a split-tunnel VPN) and is reported, never claimed green.


### Requirements map

- When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbo…
  - tests: `tests::test_xray_tun_inbound_emitted_when_enabled`; `tests::test_xray_generator_emits_tun_inbound_when_enabled`
- When `dns_hijack` is `Hijack`, the config SHALL additionally contain a `{"protocol": "dns", "tag": "dns-out"}` outbound and a routing rule `{"network": "udp", "…
  - tests: `tests::test_xray_tun_hijack_emits_dns_out_and_udp53_rule`; `tests::test_xray_tun_native_and_disabled_skip_hijack`
- - **THEN** the generated config inbounds SHALL include a `{ "protocol": "tun", "settings": { "name": "...", "mtu": 1500, "gateway": ["198.18.0.1/30"] } }` entry…
  - tests: `tests::test_xray_tun_inbound_emitted_when_enabled`; `tests::test_xray_generator_emits_tun_inbound_when_enabled`
- - **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound
  - tests: `tests::test_xray_no_tun_inbound_when_disabled`; `tests::test_v2ray_never_emits_tun_even_when_enabled`
- - **THEN** the config SHALL contain the `dns-out` outbound and the `udp/53 → dns-out` routing rule so TUN-captured plaintext DNS is answered by the built-in res…
  - tests: `tests::test_xray_tun_hijack_emits_dns_out_and_udp53_rule`
- - **THEN** the config SHALL contain neither the `dns-out` outbound nor the `udp/53` routing rule
  - tests: `tests::test_xray_tun_native_and_disabled_skip_hijack`
- The system SHALL configure each backend so the backend's own outbound traffic bypasses the TUN interface and does not loop. For xray, loop prevention SHALL rely…
  - tests: `tests::test_xray_tun_marks_every_dialing_outbound`; `tests::test_xray_tun_marks_dialing_outbounds`; `tests::test_xray_tun_inbound_emitted_when_enabled`
- - **THEN** the route section SHALL set `auto_detect_interface: true`
  - tests: `tests::test_singbox_tun_inbound_emitted_when_enabled`
- - **THEN** every outbound other than `blackhole` and `dns` SHALL set `streamSettings.sockopt.mark` to 255, and the tun inbound settings SHALL NOT contain `autoO…
  - tests: `tests::test_xray_tun_marks_every_dialing_outbound`; `tests::test_xray_tun_marks_dialing_outbounds`; `tests::test_xray_generator_emits_tun_inbound_when_enabled`
- - **THEN** that traffic SHALL leave through the VPN interface, not the default-route interface
  - tests: `MANUAL task 2.2: live split-tunnel VPN check — `ip route get <node-ip> mark 255` via the uplink, `dig @<vpn-dns-ip>` returns the internal record, no `tun-in` connection to the node address in backend.log`

### Plan review

`pass` by zarchitect, round 1. Blocker `x2-via-rule-index` (the via-node rule is never `rules[0]` — `build_routing` prepends the dns-direct, `DNS_INTERNAL_TAG`, exclusion and udp/53 rules ahead of every user rule) was fixed by locating the rule by its emitted `domain:api.example.com` value. Carried warning: the canonical specs keep mandating `autoOutboundsInterface` until this change is archived.

## Plan appendix

```json
{
  "v": 2,
  "change": "drop-xray-outbound-interface-binding",
  "baseSha": "1d6a0fb8eb02e73a65fc393adfdd743e2bba742f",
  "generatedAt": "2026-09-16T09:06:36.199091+00:00",
  "tier": "light",
  "mode": "existing-service-strict",
  "lenses": [
    "spec"
  ],
  "chunks": [
    {
      "id": "x1",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "tun-inbound-shape",
      "shard": "generator",
      "pkgDirs": [
        "crates/core/src/config"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_xray_tun_inbound",
          "anchor": "\"autoOutboundsInterface\": \"auto\",",
          "change": "Delete the key from the tun inbound `settings` json! block; `name`/`mtu`/`gateway` stay."
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "tests::test_xray_tun_inbound_emitted_when_enabled",
          "anchor": "assert_eq!(tun[\"settings\"][\"autoOutboundsInterface\"], \"auto\");",
          "change": "Flip to asserting absence, e.g. assert!(tun[\"settings\"].get(\"autoOutboundsInterface\").is_none())."
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/xray.rs",
          "symbol": "tests::test_xray_generator_emits_tun_inbound_when_enabled",
          "anchor": "assert_eq!(tun[\"settings\"][\"autoOutboundsInterface\"], \"auto\");",
          "change": "Same flip to absence assertion (this file's only hit, inside the tun-inbound test)."
        }
      ],
      "contract": {
        "states": [
          "xray-tun-on",
          "xray-tun-off",
          "v2ray-tun-on"
        ],
        "transitions": [
          {
            "input": "backend V2rayFamilyBackend::Xray, settings.tun.enabled = true -> tun inbound settings key autoOutboundsInterface",
            "state": "xray-tun-on",
            "effect": "clear",
            "evidence": "\"autoOutboundsInterface\": \"auto\","
          },
          {
            "input": "same config -> tun inbound settings keys name / mtu / gateway",
            "state": "xray-tun-on",
            "effect": "no-op",
            "evidence": "assert_eq!(tun[\"settings\"][\"gateway\"], json!([\"198.18.0.1/30\"]));"
          },
          {
            "input": "same config -> tun inbound sniffing block",
            "state": "xray-tun-on",
            "effect": "no-op",
            "evidence": "assert_eq!(tun[\"sniffing\"][\"enabled\"], true);"
          },
          {
            "input": "backend Xray, settings.tun.enabled = false -> inbounds contain no protocol == \"tun\" entry",
            "state": "xray-tun-off",
            "effect": "no-op",
            "evidence": "fn test_xray_no_tun_inbound_when_disabled"
          },
          {
            "input": "backend V2rayFamilyBackend::V2ray, settings.tun.enabled = true -> inbounds stay socks-in + http-in only",
            "state": "v2ray-tun-on",
            "effect": "no-op",
            "evidence": "fn test_v2ray_never_emits_tun_even_when_enabled"
          }
        ],
        "forbidden": [
          "any generated config carrying the string autoOutboundsInterface anywhere",
          "a tun inbound emitted under V2rayFamilyBackend::V2ray",
          "removing name / mtu / gateway / sniffing alongside the key",
          "asserting equality against Value::Null instead of asserting the key is absent (tun[\"settings\"].get(\"autoOutboundsInterface\").is_none())"
        ],
        "seeding": [
          "xray-tun-on (v2ray.rs tests): let mut settings = default_settings(); settings.tun.enabled = true; settings.tun.address_v4 = \"198.18.0.1/30\".to_string(); generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray); read via the existing helper find_tun_inbound",
          "xray-tun-on (xray.rs tests): let mut settings = AppSettings::default(); settings.tun.enabled = true; XrayGenerator.generate(&[xray_vless_with_xtls()], &[], &settings).unwrap()",
          "xray-tun-off: default_settings() / AppSettings::default() with tun untouched (TunConfig default is enabled: false)",
          "v2ray-tun-on: V2rayGenerator.generate(&[ss_node()], &[], &settings) with settings.tun.enabled = true",
          "never hand-build the inbound Value; always go through the generator"
        ],
        "budgets": [
          "crate test run: timeout 5m, --test-threads=4 (Makefile TEST_TIMEOUT=5m, TEST_THREADS=4)",
          "generator is pure and allocation-only; no timing assertion"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "Flip crates/core/src/config/v2ray.rs tests::test_xray_tun_inbound_emitted_when_enabled: replace `assert_eq!(tun[\"settings\"][\"autoOutboundsInterface\"], \"auto\");` with `assert!(tun[\"settings\"].get(\"autoOutboundsInterface\").is_none());` and keep the name/mtu/gateway/sniffing assertions",
        "Flip crates/core/src/config/xray.rs tests::test_xray_generator_emits_tun_inbound_when_enabled the same way; keep `assert_eq!(tun[\"sniffing\"][\"enabled\"], true);`",
        "Delete the line `\"autoOutboundsInterface\": \"auto\",` from the `settings` object of build_xray_tun_inbound in crates/core/src/config/v2ray.rs; `name`, `mtu`, `gateway` and the sniffing block are untouched"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && make lint   # test-core = timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4; lint = cargo fmt -- --check + cargo clippy --workspace --all-targets --all-features -- -D warnings",
      "coder": "zpatcher"
    },
    {
      "id": "x2",
      "taskIds": [
        "1.2"
      ],
      "prev": "x1",
      "sharedPkg": "crates/core/src/config",
      "parallel": false,
      "seam": "fwmark-sole-guard-invariant",
      "shard": "generator",
      "pkgDirs": [
        "crates/core/src/config"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/core/src/config/xray.rs",
          "symbol": "tests::test_xray_tun_marks_dialing_outbounds",
          "anchor": "fn test_xray_tun_marks_dialing_outbounds() {",
          "change": "Extend or add a sibling test building a multi-node + via_node-rule xray TUN config and asserting every outbound whose protocol is not blackhole/dns has streamSettings.sockopt.mark == 255 (covers node outbounds 0..n, via-node outbounds, `direct`)."
        }
      ],
      "contract": {
        "states": [
          "tun-on-multi-node",
          "tun-off"
        ],
        "transitions": [
          {
            "input": "node outbound (protocol vless / vmess / shadowsocks / trojan), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "outbound[\"streamSettings\"][\"sockopt\"][\"mark\"] = Value::from(XRAY_TUN_FWMARK);"
          },
          {
            "input": "via-node outbound named by a rule's via_node — it IS nodes[i+1]'s own outbound, not a separate object",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "pub(crate) fn via_outbound_tags("
          },
          {
            "input": "tag `direct` (protocol freedom), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "\"tag\": \"direct\","
          },
          {
            "input": "tag `block` (protocol blackhole), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "forced",
            "evidence": "if outbound[\"protocol\"] == \"blackhole\" || outbound[\"protocol\"] == \"dns\" {"
          },
          {
            "input": "tag `dns-out` (protocol dns, present because DnsHijackMode::Hijack is the TunConfig default), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "forced",
            "evidence": "\"tag\": \"dns-out\","
          },
          {
            "input": "any outbound, settings.tun.enabled = false",
            "state": "tun-off",
            "effect": "no-op",
            "evidence": "fn test_xray_no_fwmark_when_tun_disabled"
          }
        ],
        "forbidden": [
          "exempting outbounds by tag (\"block\" / \"dns-out\") instead of by protocol — apply_tun_fwmark skips on protocol, and a tag-keyed test fails on the dns-out hijack outbound",
          "asserting a mark on a blackhole or dns outbound",
          "a single-node fixture for this test: with one node there is no nodes[1], via_outbound_tags yields nothing and the via-node class is never exercised",
          "any outbound reachable in the tun-on config with protocol outside {blackhole, dns} and no streamSettings.sockopt.mark",
          "reintroducing an interface-binding key as a second guard"
        ],
        "seeding": [
          "tun-on-multi-node: let mut settings = AppSettings::default(); settings.tun.enabled = true; nodes = [xray_vless_with_xtls(), ws_vless_with_host_header(&[])]; rules = vec![ RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: \"api.example.com\".into() }, action: RuleAction::Proxy, enabled: true, group: None, via_node: Some(ConnectionNodeRef::Manual { node_id: Uuid::new_v4() }) } ]; XrayGenerator.generate(&nodes, &rules, &settings).unwrap()",
          "the xray.rs tests mod needs `use uuid::Uuid;` added — `use crate::models::*;` does not re-export Uuid (v2ray.rs tests import it explicitly)",
          "tun-off: AppSettings::default() (TunConfig default enabled: false), same nodes; covered by the existing test_xray_no_fwmark_when_tun_disabled",
          "dns-out is seeded only by leaving tun.dns_hijack at its DnsHijackMode::Hijack default; never inject the outbound by hand"
        ],
        "budgets": [
          "expected mark value: exactly 255 (XRAY_TUN_FWMARK, cross-asserted against netctl's XRAY_FWMARK in the netctl tests)",
          "crate test run: timeout 5m, --test-threads=4",
          "at least 2 proxy nodes in the fixture so the via-node class exists"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "Add crates/core/src/config/xray.rs tests::test_xray_tun_marks_every_dialing_outbound (sibling of the existing test_xray_tun_marks_dialing_outbounds, which keeps its single-node coverage): build the multi-node + via-node-rule config per the seeding line, then iterate config[\"outbounds\"] asserting mark == 255 for every outbound whose protocol is neither \"blackhole\" nor \"dns\", and is_null() for those two; include the outbound in the failure message as the neighbouring tests do",
        "Add `use uuid::Uuid;` to the xray.rs tests mod",
        "Assert the via-node tag actually resolved, locating the rule BY ITS MATCH and never by index: find the entry of config[\"routing\"][\"rules\"] whose `domain` array contains the emitted, prefixed form `domain:api.example.com` (the generator renders RuleMatch::Domain { pattern } as `format!(\"domain:{pattern}\")`, so a bare `api.example.com` never matches; no other rule in the tun-on fixture carries a `domain` array), and assert its `outboundTag` equals crate::config::common::outbound_tag(&nodes[1], 1). Index 0 is NOT the user rule — with backend xray and tun.enabled, build_routing prepends the optional dns-direct rule, the DNS_INTERNAL_TAG inboundTag rule, the exclusion rules and the udp/53 hijack ahead of every user rule."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core marks_every_dialing_outbound -- --test-threads=4 && make test-core && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "x3",
      "taskIds": [
        "1.3"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "docs-loop-guard",
      "shard": "docs",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.3",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "netctl policy-rules list (pref 9000 bullet)",
          "anchor": "9000 — `fwmark 0xff` → `main`, so xray's own sockets reach the real default",
          "change": "State the fwmark + pref-9000 rule is the only xray loop guard; no interface binding on outbounds."
        }
      ],
      "contract": {
        "states": [],
        "transitions": [
          {
            "input": "reader of the netctl policy-rules list looking for what stops xray's dials re-entering the tunnel",
            "state": "",
            "effect": "no-op",
            "evidence": "9000 — `fwmark 0xff` → `main`, so xray's own sockets reach the real default"
          }
        ],
        "forbidden": [
          "adding a mention of autoOutboundsInterface to docs/ (it has zero occurrences today — this is a rewording of the pref-9000 bullet, not a deletion)",
          "editing the bootstrap-resolver paragraph that already leans on the pref-9000 rule beyond leaving it consistent"
        ],
        "seeding": [],
        "budgets": []
      },
      "redTasks": [],
      "codeTasks": [
        "In docs/ARCHITECTURE.md, extend the pref-9000 bullet under `xray-up` to state that the fwmark and this rule are the sole loop guard — xray outbounds are not bound to an interface, so they follow the whole `main` table including VPN routes",
        "Read back the pref-9000 bullet and the `Bootstrap resolvers (xray)` paragraph to confirm they agree"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "read back the pref-9000 bullet in docs/ARCHITECTURE.md; no command",
      "coder": "zpatcher"
    },
    {
      "id": "x4",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "x2",
      "sharedPkg": "crates/core/src/config",
      "parallel": false,
      "seam": "verification",
      "shard": "",
      "pkgDirs": [
        "crates/core/src/config"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/core/tests/xray_check.rs",
          "symbol": "xray_available",
          "anchor": "fn xray_available",
          "change": "Run the workspace floor; report whether xray was in PATH, since every test here silently skips when it is not."
        }
      ],
      "contract": {
        "states": [
          "automated-floor",
          "manual-live"
        ],
        "transitions": [
          {
            "input": "workspace test run with xray present in PATH",
            "state": "automated-floor",
            "effect": "set",
            "evidence": "fn xray_available() -> bool"
          },
          {
            "input": "workspace test run with xray absent",
            "state": "automated-floor",
            "effect": "no-op",
            "evidence": "every test in crates/core/tests/xray_check.rs returns early when xray_available() is false — green does not prove the config still validates; say so in the report"
          },
          {
            "input": "live xray TUN session with a split-tunnel VPN up",
            "state": "manual-live",
            "effect": "forced",
            "evidence": "dig <internal-host> @<vpn-dns-ip> returns the internal A record with no COOKIE warning; ip route get <node-ip> mark 255 goes via the uplink; backend.log shows no tun-in connection to the node's own address"
          }
        ],
        "forbidden": [
          "running crates/core/tests/xray_check.rs concurrently with a live app instance — it binds real ports (39000+slot / 39400+slot) and creates xchk<slot> tun names",
          "reporting 2.1 green as evidence the generated config still validates when xray was not installed",
          "a bare `cargo test` with no timeout or thread cap"
        ],
        "seeding": [
          "automated-floor: `make test TEST_TIMEOUT=10m` from the repo root (expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`)",
          "manual-live: operator runs the app with xray TUN connected; not reachable from the test suite"
        ],
        "budgets": [
          "workspace run: timeout 10m, --test-threads=4",
          "per-crate run: timeout 5m, --test-threads=4"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "Run `make test TEST_TIMEOUT=10m`; report whether xray was in PATH (if not, state that crates/core/tests/xray_check.rs silently skipped)",
        "Run `make lint` (cargo fmt -- --check, then cargo clippy --workspace --all-targets --all-features -- -D warnings)",
        "Report 2.2 as MANUAL with the three checks spelled out for the operator; do not mark it done"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test TEST_TIMEOUT=10m && make lint",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "tun-inbound-shape",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent, so the coder writes the assertions itself. NO-TESTER-WAIVER: same — no test-writer agent exists for Rust; tests are the first codeTasks of this seam. Delete the `autoOutboundsInterface` key from the xray tun inbound in build_xray_tun_inbound and flip the two shape assertions to absence. Test files the coder may change: crates/core/src/config/v2ray.rs (mod tests), crates/core/src/config/xray.rs (mod tests). No new test file.",
      "contract": {
        "states": [
          "xray-tun-on",
          "xray-tun-off",
          "v2ray-tun-on"
        ],
        "transitions": [
          {
            "input": "backend V2rayFamilyBackend::Xray, settings.tun.enabled = true -> tun inbound settings key autoOutboundsInterface",
            "state": "xray-tun-on",
            "effect": "clear",
            "evidence": "\"autoOutboundsInterface\": \"auto\","
          },
          {
            "input": "same config -> tun inbound settings keys name / mtu / gateway",
            "state": "xray-tun-on",
            "effect": "no-op",
            "evidence": "assert_eq!(tun[\"settings\"][\"gateway\"], json!([\"198.18.0.1/30\"]));"
          },
          {
            "input": "same config -> tun inbound sniffing block",
            "state": "xray-tun-on",
            "effect": "no-op",
            "evidence": "assert_eq!(tun[\"sniffing\"][\"enabled\"], true);"
          },
          {
            "input": "backend Xray, settings.tun.enabled = false -> inbounds contain no protocol == \"tun\" entry",
            "state": "xray-tun-off",
            "effect": "no-op",
            "evidence": "fn test_xray_no_tun_inbound_when_disabled"
          },
          {
            "input": "backend V2rayFamilyBackend::V2ray, settings.tun.enabled = true -> inbounds stay socks-in + http-in only",
            "state": "v2ray-tun-on",
            "effect": "no-op",
            "evidence": "fn test_v2ray_never_emits_tun_even_when_enabled"
          }
        ],
        "forbidden": [
          "any generated config carrying the string autoOutboundsInterface anywhere",
          "a tun inbound emitted under V2rayFamilyBackend::V2ray",
          "removing name / mtu / gateway / sniffing alongside the key",
          "asserting equality against Value::Null instead of asserting the key is absent (tun[\"settings\"].get(\"autoOutboundsInterface\").is_none())"
        ],
        "seeding": [
          "xray-tun-on (v2ray.rs tests): let mut settings = default_settings(); settings.tun.enabled = true; settings.tun.address_v4 = \"198.18.0.1/30\".to_string(); generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray); read via the existing helper find_tun_inbound",
          "xray-tun-on (xray.rs tests): let mut settings = AppSettings::default(); settings.tun.enabled = true; XrayGenerator.generate(&[xray_vless_with_xtls()], &[], &settings).unwrap()",
          "xray-tun-off: default_settings() / AppSettings::default() with tun untouched (TunConfig default is enabled: false)",
          "v2ray-tun-on: V2rayGenerator.generate(&[ss_node()], &[], &settings) with settings.tun.enabled = true",
          "never hand-build the inbound Value; always go through the generator"
        ],
        "budgets": [
          "crate test run: timeout 5m, --test-threads=4 (Makefile TEST_TIMEOUT=5m, TEST_THREADS=4)",
          "generator is pure and allocation-only; no timing assertion"
        ]
      },
      "codeTasks": [
        "Flip crates/core/src/config/v2ray.rs tests::test_xray_tun_inbound_emitted_when_enabled: replace `assert_eq!(tun[\"settings\"][\"autoOutboundsInterface\"], \"auto\");` with `assert!(tun[\"settings\"].get(\"autoOutboundsInterface\").is_none());` and keep the name/mtu/gateway/sniffing assertions",
        "Flip crates/core/src/config/xray.rs tests::test_xray_generator_emits_tun_inbound_when_enabled the same way; keep `assert_eq!(tun[\"sniffing\"][\"enabled\"], true);`",
        "Delete the line `\"autoOutboundsInterface\": \"auto\",` from the `settings` object of build_xray_tun_inbound in crates/core/src/config/v2ray.rs; `name`, `mtu`, `gateway` and the sniffing block are untouched"
      ]
    },
    {
      "id": "fwmark-sole-guard-invariant",
      "tasks": [
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent. NO-TESTER-WAIVER: same; the invariant test is the first codeTask here. With the interface binding gone, streamSettings.sockopt.mark is the only thing keeping xray's own dials out of the tunnel, so a test asserts the mark on every dialing outbound of a multi-node xray TUN config, including via-node targets. Test file the coder may change: crates/core/src/config/xray.rs (mod tests) only.",
      "contract": {
        "states": [
          "tun-on-multi-node",
          "tun-off"
        ],
        "transitions": [
          {
            "input": "node outbound (protocol vless / vmess / shadowsocks / trojan), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "outbound[\"streamSettings\"][\"sockopt\"][\"mark\"] = Value::from(XRAY_TUN_FWMARK);"
          },
          {
            "input": "via-node outbound named by a rule's via_node — it IS nodes[i+1]'s own outbound, not a separate object",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "pub(crate) fn via_outbound_tags("
          },
          {
            "input": "tag `direct` (protocol freedom), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "set",
            "evidence": "\"tag\": \"direct\","
          },
          {
            "input": "tag `block` (protocol blackhole), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "forced",
            "evidence": "if outbound[\"protocol\"] == \"blackhole\" || outbound[\"protocol\"] == \"dns\" {"
          },
          {
            "input": "tag `dns-out` (protocol dns, present because DnsHijackMode::Hijack is the TunConfig default), tun enabled",
            "state": "tun-on-multi-node",
            "effect": "forced",
            "evidence": "\"tag\": \"dns-out\","
          },
          {
            "input": "any outbound, settings.tun.enabled = false",
            "state": "tun-off",
            "effect": "no-op",
            "evidence": "fn test_xray_no_fwmark_when_tun_disabled"
          }
        ],
        "forbidden": [
          "exempting outbounds by tag (\"block\" / \"dns-out\") instead of by protocol — apply_tun_fwmark skips on protocol, and a tag-keyed test fails on the dns-out hijack outbound",
          "asserting a mark on a blackhole or dns outbound",
          "a single-node fixture for this test: with one node there is no nodes[1], via_outbound_tags yields nothing and the via-node class is never exercised",
          "any outbound reachable in the tun-on config with protocol outside {blackhole, dns} and no streamSettings.sockopt.mark",
          "reintroducing an interface-binding key as a second guard"
        ],
        "seeding": [
          "tun-on-multi-node: let mut settings = AppSettings::default(); settings.tun.enabled = true; nodes = [xray_vless_with_xtls(), ws_vless_with_host_header(&[])]; rules = vec![ RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: \"api.example.com\".into() }, action: RuleAction::Proxy, enabled: true, group: None, via_node: Some(ConnectionNodeRef::Manual { node_id: Uuid::new_v4() }) } ]; XrayGenerator.generate(&nodes, &rules, &settings).unwrap()",
          "the xray.rs tests mod needs `use uuid::Uuid;` added — `use crate::models::*;` does not re-export Uuid (v2ray.rs tests import it explicitly)",
          "tun-off: AppSettings::default() (TunConfig default enabled: false), same nodes; covered by the existing test_xray_no_fwmark_when_tun_disabled",
          "dns-out is seeded only by leaving tun.dns_hijack at its DnsHijackMode::Hijack default; never inject the outbound by hand"
        ],
        "budgets": [
          "expected mark value: exactly 255 (XRAY_TUN_FWMARK, cross-asserted against netctl's XRAY_FWMARK in the netctl tests)",
          "crate test run: timeout 5m, --test-threads=4",
          "at least 2 proxy nodes in the fixture so the via-node class exists"
        ]
      },
      "codeTasks": [
        "Add crates/core/src/config/xray.rs tests::test_xray_tun_marks_every_dialing_outbound (sibling of the existing test_xray_tun_marks_dialing_outbounds, which keeps its single-node coverage): build the multi-node + via-node-rule config per the seeding line, then iterate config[\"outbounds\"] asserting mark == 255 for every outbound whose protocol is neither \"blackhole\" nor \"dns\", and is_null() for those two; include the outbound in the failure message as the neighbouring tests do",
        "Add `use uuid::Uuid;` to the xray.rs tests mod",
        "Assert the via-node tag actually resolved, locating the rule BY ITS MATCH and never by index: find the entry of config[\"routing\"][\"rules\"] whose `domain` array contains the emitted, prefixed form `domain:api.example.com` (the generator renders RuleMatch::Domain { pattern } as `format!(\"domain:{pattern}\")`, so a bare `api.example.com` never matches; no other rule in the tun-on fixture carries a `domain` array), and assert its `outboundTag` equals crate::config::common::outbound_tag(&nodes[1], 1). Index 0 is NOT the user rule — with backend xray and tun.enabled, build_routing prepends the optional dns-direct rule, the DNS_INTERNAL_TAG inboundTag rule, the exclusion rules and the udp/53 hijack ahead of every user rule."
      ]
    },
    {
      "id": "docs-loop-guard",
      "tasks": [
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: prose change, no executable contract. NO-TESTER-WAIVER: Rust stack has no test-writer agent and docs are not asserted. Name the fwmark plus the pref-9000 policy rule as the only xray loop guard in docs/ARCHITECTURE.md. No test file.",
      "contract": {
        "states": [],
        "transitions": [
          {
            "input": "reader of the netctl policy-rules list looking for what stops xray's dials re-entering the tunnel",
            "state": "",
            "effect": "no-op",
            "evidence": "9000 — `fwmark 0xff` → `main`, so xray's own sockets reach the real default"
          }
        ],
        "forbidden": [
          "adding a mention of autoOutboundsInterface to docs/ (it has zero occurrences today — this is a rewording of the pref-9000 bullet, not a deletion)",
          "editing the bootstrap-resolver paragraph that already leans on the pref-9000 rule beyond leaving it consistent"
        ],
        "seeding": [],
        "budgets": []
      },
      "codeTasks": [
        "In docs/ARCHITECTURE.md, extend the pref-9000 bullet under `xray-up` to state that the fwmark and this rule are the sole loop guard — xray outbounds are not bound to an interface, so they follow the whole `main` table including VPN routes",
        "Read back the pref-9000 bullet and the `Bootstrap resolvers (xray)` paragraph to confirm they agree"
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: verification seam, no new behavior. NO-TESTER-WAIVER: Rust stack has no test-writer agent. Whole-workspace floor plus the live TUN check. 2.2 is MANUAL: it needs a real xray backend, CAP_NET_ADMIN and a split-tunnel VPN up; it cannot run in this environment and must be reported as manual, not as green.",
      "contract": {
        "states": [
          "automated-floor",
          "manual-live"
        ],
        "transitions": [
          {
            "input": "workspace test run with xray present in PATH",
            "state": "automated-floor",
            "effect": "set",
            "evidence": "fn xray_available() -> bool"
          },
          {
            "input": "workspace test run with xray absent",
            "state": "automated-floor",
            "effect": "no-op",
            "evidence": "every test in crates/core/tests/xray_check.rs returns early when xray_available() is false — green does not prove the config still validates; say so in the report"
          },
          {
            "input": "live xray TUN session with a split-tunnel VPN up",
            "state": "manual-live",
            "effect": "forced",
            "evidence": "dig <internal-host> @<vpn-dns-ip> returns the internal A record with no COOKIE warning; ip route get <node-ip> mark 255 goes via the uplink; backend.log shows no tun-in connection to the node's own address"
          }
        ],
        "forbidden": [
          "running crates/core/tests/xray_check.rs concurrently with a live app instance — it binds real ports (39000+slot / 39400+slot) and creates xchk<slot> tun names",
          "reporting 2.1 green as evidence the generated config still validates when xray was not installed",
          "a bare `cargo test` with no timeout or thread cap"
        ],
        "seeding": [
          "automated-floor: `make test TEST_TIMEOUT=10m` from the repo root (expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`)",
          "manual-live: operator runs the app with xray TUN connected; not reachable from the test suite"
        ],
        "budgets": [
          "workspace run: timeout 10m, --test-threads=4",
          "per-crate run: timeout 5m, --test-threads=4"
        ]
      },
      "codeTasks": [
        "Run `make test TEST_TIMEOUT=10m`; report whether xray was in PATH (if not, state that crates/core/tests/xray_check.rs silently skipped)",
        "Run `make lint` (cargo fmt -- --check, then cargo clippy --workspace --all-targets --all-features -- -D warnings)",
        "Report 2.2 as MANUAL with the three checks spelled out for the operator; do not mark it done"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbounds, with the configured name, MTU, gateway address(es), DNS, and sniffing enabled; the inbound SHALL NOT set `autoOutboundsInterface`.",
      "tests": [
        "tests::test_xray_tun_inbound_emitted_when_enabled",
        "tests::test_xray_generator_emits_tun_inbound_when_enabled"
      ]
    },
    {
      "shall": "When `dns_hijack` is `Hijack`, the config SHALL additionally contain a `{\"protocol\": \"dns\", \"tag\": \"dns-out\"}` outbound and a routing rule `{\"network\": \"udp\", \"port\": 53, \"outboundTag\": \"dns-out\"}` placed after the `dns-internal` inboundTag rule and before exclusion and user rules; `Native` and `Disabled` SHALL omit both.",
      "tests": [
        "tests::test_xray_tun_hijack_emits_dns_out_and_udp53_rule",
        "tests::test_xray_tun_native_and_disabled_skip_hijack"
      ]
    },
    {
      "shall": "- **THEN** the generated config inbounds SHALL include a `{ \"protocol\": \"tun\", \"settings\": { \"name\": \"...\", \"mtu\": 1500, \"gateway\": [\"198.18.0.1/30\"] } }` entry with sniffing enabled and no `autoOutboundsInterface` key",
      "tests": [
        "tests::test_xray_tun_inbound_emitted_when_enabled",
        "tests::test_xray_generator_emits_tun_inbound_when_enabled"
      ]
    },
    {
      "shall": "- **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound",
      "tests": [
        "tests::test_xray_no_tun_inbound_when_disabled",
        "tests::test_v2ray_never_emits_tun_even_when_enabled"
      ]
    },
    {
      "shall": "- **THEN** the config SHALL contain the `dns-out` outbound and the `udp/53 → dns-out` routing rule so TUN-captured plaintext DNS is answered by the built-in resolver",
      "tests": [
        "tests::test_xray_tun_hijack_emits_dns_out_and_udp53_rule"
      ]
    },
    {
      "shall": "- **THEN** the config SHALL contain neither the `dns-out` outbound nor the `udp/53` routing rule",
      "tests": [
        "tests::test_xray_tun_native_and_disabled_skip_hijack"
      ]
    },
    {
      "shall": "The system SHALL configure each backend so the backend's own outbound traffic bypasses the TUN interface and does not loop. For xray, loop prevention SHALL rely on the fwmark carried by every dialing outbound and the route helper's policy rule sending marked traffic to the `main` table; the generated config SHALL NOT bind outbound sockets to a fixed or auto-detected interface, so xray's egress follows every route in `main`, including more-specific routes through other interfaces such as a VPN.",
      "tests": [
        "tests::test_xray_tun_marks_every_dialing_outbound",
        "tests::test_xray_tun_marks_dialing_outbounds",
        "tests::test_xray_tun_inbound_emitted_when_enabled"
      ]
    },
    {
      "shall": "- **THEN** the route section SHALL set `auto_detect_interface: true`",
      "tests": [
        "tests::test_singbox_tun_inbound_emitted_when_enabled"
      ]
    },
    {
      "shall": "- **THEN** every outbound other than `blackhole` and `dns` SHALL set `streamSettings.sockopt.mark` to 255, and the tun inbound settings SHALL NOT contain `autoOutboundsInterface`",
      "tests": [
        "tests::test_xray_tun_marks_every_dialing_outbound",
        "tests::test_xray_tun_marks_dialing_outbounds",
        "tests::test_xray_generator_emits_tun_inbound_when_enabled"
      ]
    },
    {
      "shall": "- **THEN** that traffic SHALL leave through the VPN interface, not the default-route interface",
      "tests": [
        "MANUAL task 2.2: live split-tunnel VPN check — `ip route get <node-ip> mark 255` via the uplink, `dig @<vpn-dns-ip>` returns the internal record, no `tun-in` connection to the node address in backend.log"
      ]
    }
  ],
  "testHarness": [
    "fixtures::default_settings / vless_node / vmess_node / ss_node / trojan_node / xhttp_node — crates/core/src/config/test_fixtures.rs — Default AppSettings and one ProxyNode per protocol, shared by all generator unit tests.",
    "tests::find_tun_inbound — crates/core/src/config/v2ray.rs — Returns the first inbound with protocol == \"tun\" from a generated config Value.",
    "tests::proxy_rule(pattern, via_node) — crates/core/src/config/v2ray.rs — An enabled Domain-match Proxy RoutingRule with an optional ConnectionNodeRef pin — the way to exercise via-node outbounds.",
    "tests::xray_vless_with_xtls / vless_without_xtls — crates/core/src/config/xray.rs — VLESS ProxyNodes with and without XTLS flow, used by the existing fwmark tests.",
    "xray_available — crates/core/tests/xray_check.rs — Probe for `xray version`; every test in the file returns early (silent skip) when absent.",
    "isolate(name, settings) — crates/core/tests/xray_check.rs — Per-case socks/http ports (39000+slot / 39400+slot) and tun iface name `xchk<slot>` so `xray run -test` does not collide.",
    "check / check_with_rules / check_with_nodes — crates/core/tests/xray_check.rs — Generates the config, writes it to a tempfile, runs `xray run -test -c`, asserts success and no non-allowlisted `deprecated` line.",
    "deprecated_node_feature + DEPRECATED_NODE_FEATURES — crates/core/tests/xray_check.rs — Allowlist filter for upstream deprecation warnings caused by the node itself (Shadowsocks, WebSocket transport)."
  ],
  "floor": "make test TEST_TIMEOUT=10m && make lint   —   expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`, then `cargo fmt -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Task 2.2 stays MANUAL (needs a real xray backend, CAP_NET_ADMIN and a split-tunnel VPN) and is reported, never claimed green.",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
