## Context

- Scope: the derived DNS plane — the auto-split rules and the TUN-derived sing-box plane — referencing only servers that exist and carrying what the spec says it carries.
- Auto split coupling: xray assigns derived domains only to servers tagged `remote`/`domestic` (`build_user_dns_servers`, `crates/core/src/config/v2ray.rs`); sing-box emits `"server": "remote"` / `"server": "domestic"` unconditionally (`build_dns`, `crates/core/src/config/singbox.rs`). `DnsConfig::validate` checks `rules[].server_tag` only — custom rules — so a derived rule naming an absent server passes validation.
- The sing-box derived plane `derived_tun_dns` (`crates/core/src/config/singbox.rs`) emits `strategy`, `servers`, `rules` and `final` only. The enabled path sets per-server `client_subnet` and top-level `disable_cache`; the config-generator spec requires both on the derived path too.
- The derived server tag is the constant `DERIVED_DNS_TAG`; the derived rules hard-code the `remote`/`domestic` literals.

## Goals / Non-Goals

**Goals:**
- No generated config references a DNS server that is not in it.
- The sing-box derived plane carries the same cache control and client subnet as the user-configured path.

**Non-Goals:**
- Failing generation over a missing tag: connections that run today with the split simply unused must keep working.
- Requiring the tags in `DnsConfig::validate`: it sees no routing rules, so it would reject configs whose routing has no domain rules at all.
- Renaming servers to `remote`/`domestic` automatically.

## Decisions

- **Skip derived rules for missing tags, and log it.** Both generators check tag presence before emitting a derived rule; sing-box stops referencing an undefined server, xray keeps its current skip but logs the tag and the number of skipped entries. The same predicate feeds the primary-row note in the UI.
- **The derived sing-box plane copies cache control and client subnet exactly like the enabled path.** `disable_cache` at the `dns` level, `client_subnet` on the derived DoH server.

## Risks / Trade-offs

- [A skipped derived rule changes which resolver answers a domain on sing-box] → before the derived block is changed, a `singbox_check` case with servers `remote` + `lan` records what the current output actually did with `sing-box check`, so the proposal's claim is confirmed or corrected before merge.
- [The tag presence check diverges between the two generators] → both read the same predicate (`DnsConfig::has_server_tag`, the constants `AUTO_SPLIT_REMOTE_TAG`/`AUTO_SPLIT_DOMESTIC_TAG`), and each has a test naming the missing-tag scenario. The generated server list is not the same list as `settings.dns.servers` — sing-box injects `hosts`, `fakeip` and bootstrap-local servers, xray prepends bootstrap servers — but none of those injected entries carries `remote` or `domestic`, so for the two auto-split tags the settings-level predicate and the generated-list predicate agree.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, base `aa95c841`. Lenses: **spec** (three delta specs, every SHALL mapped) and **quality** (a shared predicate consumed by three call sites). Not triggered: `sec` (no auth, secrets, SQL or new I/O), `arch` (no new package, no moved boundary), `perf` (config generation, not a hot path).

Stack note: Rust. The execution flow has no test-writer stage and no tester stage for this stack, so every chunk is one `rust-coder` dispatch that ships its tests and its code together, and closes by waiver; the guard is the pre-seal over the crate's integration test files (`crates/<crate>/tests/*.rs`) plus the chunk's own literal command and the workspace floor. Inline `#[cfg(test)] mod tests` are guarded by the source diff.

Four chunks in three waves — `c1`, then `c2` and `c4` concurrently (different crates, disjoint files, one worktree each), then `c3`:

### c1-xray-tag-warning — tasks 1.1, 1.5 (first, no predecessor)

- Sites: `crates/core/src/config/v2ray.rs` `build_user_dns_servers` (anchor `fn build_user_dns_servers(`); `crates/core/src/models/dns.rs` `DnsConfig` (anchor `pub struct DnsConfig {`) — new `pub const AUTO_SPLIT_REMOTE_TAG`/`AUTO_SPLIT_DOMESTIC_TAG` and `DnsConfig::has_server_tag`; `crates/core/src/config/common.rs` (anchor `pub(crate) fn split_horizon_server(settings: &AppSettings) -> Option<&DnsServerConfig> {`) — new `skipped_derived_warning(tag, skipped) -> Option<String>`; `crates/core/src/models/mod.rs` (anchor `pub use dns::{`) — re-export the constants; `crates/core/Cargo.toml` (task 1.5).
- Contract: states `split-attached`, `split-skipped-warned`, `split-skipped-silent`, `custom-rules`, `dns-off`. The observable change is `split-skipped-warned`: derived domains exist for a tag no configured server carries, so nothing is attached and the payload names the tag and the dropped-entry count. `split-skipped-silent` is count 0 — no warning. Forbidden: any `dns.servers[].domains` entry collected for a split whose tag has no configured server; a warning at count 0; a warning for a tag whose server exists. Seeding only through the shipped generator: `default_settings()` with `dns.enabled = true`, `use_custom_rules = false`, servers replaced by one tagged `lan`, one proxy `Domain` rule, through the `#[cfg(test)] build_dns` wrapper that delegates to `build_dns_for_backend`. Budget: the count is the length of the dropped vec (1 for one rule); at most one warning per missing tag, at most two per generation.
- Tests (first items of the chunk): `test_skipped_derived_warning_payload_names_tag_and_count` pins the message and `None` at count 0; `test_dns_derived_remote_tag_missing_keeps_domains_off_and_warns`; `test_dns_derived_standard_tags_attach_without_warning`. All three call the shipped wrapper.
- Verify: `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`.

### c2-singbox-rule-tag-gate — tasks 1.2, 1.3 (after c1, own worktree)

- Sites: `crates/core/tests/singbox_check.rs` `check_with_rules` (anchor `fn check_with_rules(`); `crates/core/src/config/singbox.rs` `build_dns` (anchor `fn build_dns(rules: &[RoutingRule], settings: &AppSettings, first_proxy_tag: &str) -> Value {`).
- Order matters inside the chunk: the integration case is written and run **before** the generator edit, so it records what the current dangling `"server": "domestic"` output does under `sing-box check` (task 1.2).
- Contract: states `rules-emitted`, `rules-skipped-warned`, `rules-skipped-silent`, `custom-rules`, `dns-off`. Entering `rules-skipped-warned` is the observable delta: the domestic rule is no longer pushed, so the config no longer names an absent server. Forbidden: any `dns.rules[]` entry on the auto-derived path whose `server` names a tag absent from the generated `dns.servers[].tag` list; `hosts`, `fakeip` and tun-exclusion rules must be untouched by the gate. Seeding: `default_settings()` with servers tagged `remote` (DoH) + `lan` (UDP) and one enabled direct `GeoSite{category-ru}` rule through `SingboxGenerator::generate`, plus the same settings through `check_with_rules` for the integration case. Budget: the count is geosite categories + domain patterns for that action (1 in the scenario); at most two warnings per generation; the gate covers at most four rule pushes.
- Tests: `missing_domestic_tag_derived_rules_pass_sing_box_check` (integration, `sing_box_available()`-gated — the binary is at `/usr/bin/sing-box` on this host, so it runs), `test_singbox_derived_domestic_tag_missing_emits_no_domestic_rule`, `test_singbox_derived_standard_tags_emit_both_split_rules`.
- Verify: `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`.

### c4-ui-missing-tag-note — tasks 2.1, 2.2 (after c1, own worktree, concurrent with c2)

- Sites: `crates/ui/src/preferences/dns.rs` `render_primary_dns_servers` (anchor `fn render_primary_dns_servers(ctx: &DnsRenderCtx) {`); `crates/ui/Cargo.toml` (task 2.2).
- Contract: states `note-shown`, `plain-not-configured`, `configured`. `use_custom_rules = false` with no server for the tag selects the note (proxied-domains wording for `remote`, direct-traffic wording for `domestic`); `use_custom_rules = true` keeps today's plain "Not configured"; a configured tag keeps the existing subtitle and edit button. The decision is a pure helper `primary_row_missing_note(tag, use_custom_rules) -> &'static str`, so the states are seeded without GTK.
- Tests: `test_primary_row_missing_note_states_skipped_derived_rules`, `test_primary_row_missing_note_custom_rules_shows_plain_not_configured`.
- Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4`.

### c3-derived-tun-plane — task 1.4 (after c2, own worktree)

- Sites: `crates/core/src/config/singbox.rs` `derived_tun_dns` (anchor `fn derived_tun_dns(settings: &AppSettings, first_proxy_tag: &str) -> Value {`).
- Contract: states `plane-derived-cache-set`, `plane-derived-plain`, `user-dns-path`, `no-dns-section`. With TUN on, DNS off: `disable_cache = true` lands at the `dns` level and `client_subnet` on the derived DoH server only. Forbidden: either key present when its setting is unset; `client_subnet` on the derived `hosts` server. Seeding: `default_settings()` with `tun.enabled = true`, `dns.enabled = false`, then the two settings, through `SingboxGenerator::generate`.
- Tests: `test_derived_tun_dns_carries_disable_cache`, `test_derived_tun_dns_carries_client_subnet`, `test_derived_tun_dns_without_cache_settings_emits_neither_key`, mirroring the existing `test_dns_disable_cache` / `test_dns_client_subnet`.
- Verify: `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`.

### Floor, waivers, requirements

- Floor: `timeout 10m cargo test --workspace -- --test-threads=4` (the same run `make test` performs), plus `make check` and `make lint` (`cargo fmt --all -- --check` + `cargo clippy`) before the merge.
- Waivers: every chunk carries `NO-RED-WAIVER:` (no separate red stage on this stack — the chunk's tests ship with its code) and `NO-TESTER-WAIVER:` (no tester stage; the chunk's literal command plus the floor verify it). No chunk has a red stage, so no `redRun` exists and the seal is the pre-seal taken from each worktree before the coder enters.
- Requirements: 26 SHALL blocks across the three delta specs, each mapped in the appendix to the test that observes it — an existing test for the behaviors that already shipped (C1-C18, mostly in `crates/core/src/config/{v2ray,singbox}.rs` inline modules), and the new tests above for A1-A3, B1-B3 and C19, which no test covers today.
- Live checklist 3.2 (TUN on, DNS off, both keys on the derived plane, one `domestic` warning with servers `remote` + `lan`) is not automatable here: the generated-config half is exactly what the new unit and `sing-box check` tests assert, and the running-app half is left to a live session.
- Plan review: verdict **pass**, reviewer `zarchitect`, one round, zero blockers. Warnings recorded: the warning itself is assertable only through `skipped_derived_warning`'s payload (the repo has no log-capture fixture); the UI note is asserted at helper granularity, so a dropped `set_subtitle` call site would not fail the suite; A1/B1/C1 requirement keys trace to the spec by content rather than byte-verbatim substring; and the `settings.dns.servers`  generated-list equivalence holds only for the two auto-split tags (see Risks).

## Plan appendix

```json
{
  "v": 2,
  "change": "derive-dns-from-real-tags",
  "baseSha": "aa95c841d0d556215597a353a36e4d8d240edf6c",
  "generatedAt": "2026-09-17T18:13:00.895Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "c1-xray-tag-warning",
      "taskIds": [
        "1.1",
        "1.5"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "xray-derived-split-tag-presence",
      "shard": "",
      "pkgDirs": [
        "crates/core/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_user_dns_servers",
          "anchor": "fn build_user_dns_servers(",
          "change": "Log warning naming missing tag and skipped entry count when derived domains exist for missing remote/domestic tag"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "DnsConfig",
          "anchor": "pub struct DnsConfig {",
          "change": "new pub const AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG and DnsConfig::has_server_tag"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/common.rs",
          "symbol": "skipped_derived_warning",
          "anchor": null,
          "change": "new pub(crate) fn skipped_derived_warning(tag, skipped) -> Option<String>"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/mod.rs",
          "symbol": "pub use dns::",
          "anchor": "pub use dns::{",
          "change": "re-export the two auto-split tag constants"
        },
        {
          "task": "1.5",
          "file": "crates/core/Cargo.toml",
          "symbol": "[package]",
          "anchor": "name = \"v2ray-rs-core\"",
          "change": "Verify core crate test suite succeeds with cargo test -p v2ray-rs-core -- --test-threads=4"
        }
      ],
      "contract": {
        "states": [
          "split-attached",
          "split-skipped-warned",
          "split-skipped-silent",
          "custom-rules",
          "dns-off"
        ],
        "transitions": [
          {
            "input": "dns.enabled, use_custom_rules=false, server tagged remote (domestic) present, >=1 enabled proxy-action (direct-action) Domain/GeoSite/DomainKeyword/DomainFull rule",
            "state": "split-attached",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:936-947 attaches remote_domains/domestic_domains under the exact tags; spec dns-configuration 'Standard tags unaffected'"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, no server tagged remote (domestic), >=1 entry collected for that tag",
            "state": "split-skipped-warned",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:936-943 falls through '_ => None' today; spec dns-configuration 'Missing remote tag on xray' requires the warning naming the tag and the skipped-entry count"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, tag missing, 0 entries collected for that tag",
            "state": "split-skipped-silent",
            "effect": "no-op",
            "evidence": "skipped_derived_warning returns None at count 0 (crates/core/src/config/common.rs, new); spec says the warning is per missing tag derived rules 'would have used'"
          },
          {
            "input": "dns.enabled, use_custom_rules=true",
            "state": "custom-rules",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:886-915 custom per-server domains path; out of scope"
          },
          {
            "input": "dns.enabled=false",
            "state": "dns-off",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:677-684 fallback plane; build_user_dns_servers not called"
          },
          {
            "input": "backend xray, tun.enabled, exclude_domains non-empty, no domestic server",
            "state": "split-skipped-warned",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:925-931 folds exclude_domains into domestic_domains before the tag match; the domestic warning count includes them"
          }
        ],
        "forbidden": "Any dns.servers[].domains array carrying an entry collected for a split whose tag has no configured server (e.g. domain:example.com under any server when none is tagged remote); any warning emitted at count 0; a warning naming a tag whose server exists.",
        "seeding": [
          "Only legal path is the shipped generator: default_settings() from crates/core/src/config/test_fixtures.rs (servers default to remote+domestic",
          "dns off",
          "use_custom_rules false)",
          "set settings.dns.enabled=true",
          "settings.dns.use_custom_rules=false",
          "replace settings.dns.servers (e.g. single DnsServerConfig{tag:\"lan\"",
          "protocol:Udp",
          "address:\"223.5.5.5\"",
          "port:None",
          "detour:None})",
          "pass rules to the #[cfg(test)] build_dns wrapper (crates/core/src/config/v2ray.rs:659) which delegates to build_dns_for_backend — the same function generate_v2ray_family_config calls. Never seed by writing JSON."
        ],
        "budgets": [
          "Warning count equals the length of the dropped vec for that tag: 1 for one proxy Domain rule; exactly 1 warning line per missing tag with count>0",
          "at most 2 warnings per generation (remote + domestic)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1 tests first, inline mod tests of crates/core/src/config/v2ray.rs: test_skipped_derived_warning_payload_names_tag_and_count, test_dns_derived_remote_tag_missing_keeps_domains_off_and_warns, test_dns_derived_standard_tags_attach_without_warning",
        "1.1 add pub const AUTO_SPLIT_REMOTE_TAG=\"remote\" and AUTO_SPLIT_DOMESTIC_TAG=\"domestic\" plus DnsConfig::has_server_tag(&self, tag:&str)->bool in crates/core/src/models/dns.rs; re-export both constants in crates/core/src/models/mod.rs pub use dns::{...}",
        "1.1 add pub(crate) fn skipped_derived_warning(tag:&str, skipped:usize)->Option<String> in crates/core/src/config/common.rs returning Some(\"DNS: no server tagged '{tag}' - skipping {skipped} auto-derived domain entries\") only when skipped>0",
        "1.1 in build_user_dns_servers auto-split branch: after collecting remote_domains/domestic_domains, for each tag where !settings.dns.has_server_tag(tag) log the skipped_derived_warning message via log::warn!; generated server list otherwise byte-identical"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "waiver": "NO-RED-WAIVER: Rust stack — no separate red stage; the chunk ships its own inline tests and the crate command above proves them"
    },
    {
      "id": "c2-singbox-rule-tag-gate",
      "taskIds": [
        "1.2",
        "1.3"
      ],
      "prev": "c1-xray-tag-warning",
      "sharedPkg": null,
      "parallel": true,
      "seam": "singbox-derived-rule-tag-presence",
      "shard": "core",
      "pkgDirs": [
        "crates/core/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/core/tests/singbox_check.rs",
          "symbol": "check_with_rules",
          "anchor": "fn check_with_rules(",
          "change": "Add singbox_check case with servers tagged remote + lan and direct GeoSite rule to check sing-box acceptance before generator update"
        },
        {
          "task": "1.3",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "build_dns",
          "anchor": "fn build_dns(rules: &[RoutingRule], settings: &AppSettings, first_proxy_tag: &str) -> Value {",
          "change": "Emit derived rules only for tags present in generated servers and log warning for missing tag and skipped entry count"
        }
      ],
      "contract": {
        "states": [
          "rules-emitted",
          "rules-skipped-warned",
          "rules-skipped-silent",
          "custom-rules",
          "dns-off"
        ],
        "transitions": [
          {
            "input": "dns.enabled, use_custom_rules=false, user server tagged remote (domestic) exists, >=1 enabled matching entry",
            "state": "rules-emitted",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:599-617 pushes rule_set/domain_suffix rules with server \"remote\"/\"domestic\"; spec dns-configuration 'Standard tags unaffected'"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, no server tagged domestic, >=1 direct-action GeoSite/Domain entry",
            "state": "rules-skipped-warned",
            "effect": "set",
            "evidence": "spec dns-configuration 'Missing domestic tag on sing-box': dns.rules SHALL contain no rule with \"server\":\"domestic\", warning names domestic; today singbox.rs:608-617 emits it unconditionally"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, tag missing, 0 entries for that action",
            "state": "rules-skipped-silent",
            "effect": "no-op",
            "evidence": "the existing !remote_geosite.is_empty() guards at crates/core/src/config/singbox.rs:599-617 mean nothing would have been emitted; no warning at count 0"
          },
          {
            "input": "dns.enabled, use_custom_rules=true",
            "state": "custom-rules",
            "effect": "no-op",
            "evidence": "crates/core/src/config/singbox.rs:539-566 custom DnsRule path; server_tag validity is enforced by DnsConfig::validate at crates/core/src/models/dns.rs:455-460"
          },
          {
            "input": "dns.enabled=false",
            "state": "dns-off",
            "effect": "no-op",
            "evidence": "crates/core/src/config/singbox.rs:52-63 assemble calls build_dns only when dns.enabled; TUN derives its own plane"
          }
        ],
        "forbidden": "Any dns.rules[] object on the auto-derived path whose \"server\" value names a tag absent from the generated dns.servers[].tag list; hosts/fakeip/tun-exclusion rules must be unaffected by the gate.",
        "seeding": [
          "Only legal path is the shipped generator: default_settings()",
          "settings.dns.enabled=true",
          "settings.dns.use_custom_rules=false",
          "settings.dns.servers = [DnsServerConfig{tag:\"remote\"",
          "protocol:Doh",
          "address:\"1.1.1.1\"",
          "port:None",
          "detour:None}",
          "DnsServerConfig{tag:\"lan\"",
          "protocol:Udp",
          "address:\"223.5.5.5\"",
          "port:None",
          "detour:None}]",
          "rules = one enabled RoutingRule{id:uuid::Uuid::new_v4()",
          "match_condition:RuleMatch::GeoSite{category:\"category-ru\"}",
          "action:RuleAction::Direct",
          "enabled:true",
          "group:None",
          "via_node:None}; call SingboxGenerator.generate(&[ss_node()]",
          "&rules",
          "&settings). Integration seeding: same settings/rules through check_with_rules in crates/core/tests/singbox_check.rs:41-69 gated on sing_box_available()."
        ],
        "budgets": [
          "Warning count = geosite categories + domain patterns collected for that action (scenario value: 1); at most 2 warnings per generation; the gate touches at most 4 derived rule pushes (remote/domestic x rule_set/domain_suffix)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.2 tests first: add #[test] fn missing_domestic_tag_derived_rules_pass_sing_box_check in crates/core/tests/singbox_check.rs following the new_rule_kinds_pass_sing_box_check pattern (sing_box_available() guard, check_with_rules), settings with servers tagged remote+lan and one direct GeoSite rule category-ru; run it BEFORE the build_dns change to record whether the current dangling-reference output passes sing-box check, and note the result in the change log",
        "1.3 tests first, inline mod tests of crates/core/src/config/singbox.rs: test_singbox_derived_domestic_tag_missing_emits_no_domestic_rule, test_singbox_derived_standard_tags_emit_both_split_rules",
        "1.3 build_dns auto-derived branch: gate the remote/domestic rule pushes on settings.dns.has_server_tag(AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG) and log skipped_derived_warning(tag, geosite_len + domains_len); reuse the helpers landed in tasks 1.1 (models consts + common.rs warning) so the predicate cannot drift between generators",
        "1.2/1.3 re-run the integration case after the change: green with no rule referencing domestic"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "waiver": "NO-RED-WAIVER: Rust stack — no separate red stage; the integration case in crates/core/tests/singbox_check.rs is written before the generator edit and the crate command proves it"
    },
    {
      "id": "c4-ui-missing-tag-note",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "c1-xray-tag-warning",
      "sharedPkg": null,
      "parallel": true,
      "seam": "dns-preferences-missing-tag-note",
      "shard": "ui",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "render_primary_dns_servers",
          "anchor": "fn render_primary_dns_servers(ctx: &DnsRenderCtx) {",
          "change": "Display note on primary remote/domestic rows that routing-derived DNS rules are skipped when use_custom_rules is false and tag is unconfigured"
        },
        {
          "task": "2.2",
          "file": "crates/ui/Cargo.toml",
          "symbol": "[package]",
          "anchor": "name = \"v2ray-rs-ui\"",
          "change": "Verify ui crate test suite succeeds with cargo test -p v2ray-rs-ui -- --test-threads=4"
        }
      ],
      "contract": {
        "states": [
          "note-shown",
          "plain-not-configured",
          "configured"
        ],
        "transitions": [
          {
            "input": "dns.use_custom_rules=false, no server tagged domestic",
            "state": "note-shown",
            "effect": "set",
            "evidence": "spec dns-preferences-ui 'Domestic tag missing in auto mode': the Domestic row states routing-derived DNS rules for direct traffic are skipped; today render_primary_dns_servers hard-codes \"Not configured - set up in Advanced\" (crates/ui/src/preferences/dns.rs:1121-1125, 1132-1135)"
          },
          {
            "input": "dns.use_custom_rules=false, no server tagged remote",
            "state": "note-shown",
            "effect": "set",
            "evidence": "same requirement applied to the remote row; remote variant names proxied domains"
          },
          {
            "input": "dns.use_custom_rules=true, tag missing",
            "state": "plain-not-configured",
            "effect": "clear",
            "evidence": "spec dns-preferences-ui 'Custom rules active': row shows \"Not configured\" without the skipped-rules note; derived rules are not active"
          },
          {
            "input": "server tagged remote (domestic) exists",
            "state": "configured",
            "effect": "no-op",
            "evidence": "crates/ui/src/preferences/dns.rs:1117-1123 already renders primary_dns_subtitle and enables the edit button; note logic must not fire"
          }
        ],
        "forbidden": "The skipped-rules note on any row while use_custom_rules=true or while the tag has a configured server; any wording change to the configured-row subtitle path.",
        "seeding": [
          "The note decision is a pure helper: primary_row_missing_note(AUTO_SPLIT_DOMESTIC_TAG|AUTO_SPLIT_REMOTE_TAG",
          "use_custom_rules:bool) in crates/ui/src/preferences/dns.rs; render_primary_dns_servers passes ctx.state.borrow().dns.use_custom_rules and the row tag. No GTK initialization is needed to seed the states."
        ],
        "budgets": [
          "1 &'static str per input combination; 0 extra widget allocations beyond the existing set_subtitle call; the predicate is the same DnsConfig::has_server_tag the generators use."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1 tests first, inline mod tests of crates/ui/src/preferences/dns.rs next to test_primary_row_subtitle_appends_private_warning (dns.rs:2052): test_primary_row_missing_note_states_skipped_derived_rules, test_primary_row_missing_note_custom_rules_shows_plain_not_configured",
        "2.1 add fn primary_row_missing_note(tag:&str, use_custom_rules:bool)->&'static str returning \"Not configured - set up in Advanced\" when use_custom_rules, else \"Not configured - routing-derived DNS rules for proxied domains are skipped\" for AUTO_SPLIT_REMOTE_TAG and \"Not configured - routing-derived DNS rules for direct traffic are skipped\" otherwise; use it for both missing-row subtitles in render_primary_dns_servers"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "waiver": "NO-RED-WAIVER: Rust stack — no separate red stage; the inline helper tests ship with the note and the crate command proves them"
    },
    {
      "id": "c3-derived-tun-plane",
      "taskIds": [
        "1.4"
      ],
      "prev": "c2-singbox-rule-tag-gate",
      "sharedPkg": null,
      "parallel": true,
      "seam": "singbox-derived-tun-cache-edns",
      "shard": "tunplane",
      "pkgDirs": [
        "crates/core/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.4",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "derived_tun_dns",
          "anchor": "fn derived_tun_dns(settings: &AppSettings, first_proxy_tag: &str) -> Value {",
          "change": "Include disable_cache at dns level and client_subnet on derived DoH server to mirror user-configured DNS plane"
        }
      ],
      "contract": {
        "states": [
          "plane-derived-cache-set",
          "plane-derived-plain",
          "user-dns-path",
          "no-dns-section"
        ],
        "transitions": [
          {
            "input": "tun.enabled, dns.enabled=false, settings.dns.disable_cache=true",
            "state": "plane-derived-cache-set",
            "effect": "set",
            "evidence": "spec config-generator 'Cache control and client subnet survive the sing-box derived path'; user path emits dns.disable_cache at crates/core/src/config/singbox.rs:629-632"
          },
          {
            "input": "tun.enabled, dns.enabled=false, settings.dns.client_subnet=Some(ip)",
            "state": "plane-derived-cache-set",
            "effect": "set",
            "evidence": "spec same scenario; user path sets client_subnet per server at crates/core/src/config/singbox.rs:507-510"
          },
          {
            "input": "tun.enabled, dns.enabled=false, disable_cache=false and client_subnet=None",
            "state": "plane-derived-plain",
            "effect": "no-op",
            "evidence": "derived_tun_dns today (crates/core/src/config/singbox.rs:71-105) emits neither key; spec adds keys only when carried"
          },
          {
            "input": "dns.enabled=true (tun on or off)",
            "state": "user-dns-path",
            "effect": "no-op",
            "evidence": "assemble at crates/core/src/config/singbox.rs:52-58 calls build_dns, which already sets both; derived_tun_dns not invoked"
          },
          {
            "input": "tun.enabled=false and dns.enabled=false",
            "state": "no-dns-section",
            "effect": "no-op",
            "evidence": "assemble at crates/core/src/config/singbox.rs:52-63 emits no dns key at all"
          }
        ],
        "forbidden": "A derived plane missing \"disable_cache\": true while settings.dns.disable_cache is true; a derived DoH server (tag DERIVED_DNS_TAG=\"remote\", type https) missing client_subnet while settings.dns.client_subnet is Some; either key present when the setting is unset; client_subnet on the derived hosts server.",
        "seeding": [
          "Only legal path: default_settings()",
          "settings.tun.enabled=true",
          "settings.dns.enabled=false",
          "then set settings.dns.disable_cache=true and/or settings.dns.client_subnet=Some(\"203.0.113.1\".to_string()); call SingboxGenerator.generate(&[ss_node()]",
          "&[]",
          "&settings) and read config[\"dns\"] — never hand-build the dns JSON."
        ],
        "budgets": [
          "Exactly 1 derived DoH server in the derived plane (+1 hosts server only when settings.dns.hosts is non-empty); client_subnet copied verbatim; 0 behavior change when both settings are unset."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.4 tests first, inline mod tests of crates/core/src/config/singbox.rs mirroring test_dns_disable_cache (singbox.rs:1959) and test_dns_client_subnet (singbox.rs:1971): test_derived_tun_dns_carries_disable_cache, test_derived_tun_dns_carries_client_subnet, test_derived_tun_dns_without_cache_settings_emits_neither_key",
        "1.4 derived_tun_dns: when settings.dns.disable_cache set dns_config[\"disable_cache\"]=true at the dns level; when settings.dns.client_subnet is Some set \"client_subnet\" on the DERIVED_DNS_TAG DoH server object only (not on the hosts server)"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "waiver": "NO-RED-WAIVER: Rust stack — no separate red stage; the inline derived-plane tests ship with the change and the crate command proves them"
    }
  ],
  "seams": [
    {
      "id": "xray-derived-split-tag-presence",
      "tasks": [
        "1.1",
        "1.5"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no separate test-writer stage; every chunk is one rust-coder dispatch writing tests and code together, verified by the chunk's own bounded cargo command plus the workspace floor. NO-TESTER-WAIVER: no go-tester stage exists; the inline tests in crates/core/src/config/v2ray.rs are the first items of codeTasks and the source diff guards them (the seal covers crates/<crate>/tests/*.rs only). Ships: build_user_dns_servers warns per missing auto-split tag with the dropped-entry count instead of silently discarding derived domains; generated config unchanged otherwise.",
      "contract": {
        "states": [
          "split-attached",
          "split-skipped-warned",
          "split-skipped-silent",
          "custom-rules",
          "dns-off"
        ],
        "transitions": [
          {
            "input": "dns.enabled, use_custom_rules=false, server tagged remote (domestic) present, >=1 enabled proxy-action (direct-action) Domain/GeoSite/DomainKeyword/DomainFull rule",
            "state": "split-attached",
            "effect": "set",
            "evidence": "crates/core/src/config/v2ray.rs:936-947 attaches remote_domains/domestic_domains under the exact tags; spec dns-configuration 'Standard tags unaffected'"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, no server tagged remote (domestic), >=1 entry collected for that tag",
            "state": "split-skipped-warned",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:936-943 falls through '_ => None' today; spec dns-configuration 'Missing remote tag on xray' requires the warning naming the tag and the skipped-entry count"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, tag missing, 0 entries collected for that tag",
            "state": "split-skipped-silent",
            "effect": "no-op",
            "evidence": "skipped_derived_warning returns None at count 0 (crates/core/src/config/common.rs, new); spec says the warning is per missing tag derived rules 'would have used'"
          },
          {
            "input": "dns.enabled, use_custom_rules=true",
            "state": "custom-rules",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:886-915 custom per-server domains path; out of scope"
          },
          {
            "input": "dns.enabled=false",
            "state": "dns-off",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:677-684 fallback plane; build_user_dns_servers not called"
          },
          {
            "input": "backend xray, tun.enabled, exclude_domains non-empty, no domestic server",
            "state": "split-skipped-warned",
            "effect": "no-op",
            "evidence": "crates/core/src/config/v2ray.rs:925-931 folds exclude_domains into domestic_domains before the tag match; the domestic warning count includes them"
          }
        ],
        "forbidden": "Any dns.servers[].domains array carrying an entry collected for a split whose tag has no configured server (e.g. domain:example.com under any server when none is tagged remote); any warning emitted at count 0; a warning naming a tag whose server exists.",
        "seeding": [
          "Only legal path is the shipped generator: default_settings() from crates/core/src/config/test_fixtures.rs (servers default to remote+domestic",
          "dns off",
          "use_custom_rules false)",
          "set settings.dns.enabled=true",
          "settings.dns.use_custom_rules=false",
          "replace settings.dns.servers (e.g. single DnsServerConfig{tag:\"lan\"",
          "protocol:Udp",
          "address:\"223.5.5.5\"",
          "port:None",
          "detour:None})",
          "pass rules to the #[cfg(test)] build_dns wrapper (crates/core/src/config/v2ray.rs:659) which delegates to build_dns_for_backend — the same function generate_v2ray_family_config calls. Never seed by writing JSON."
        ],
        "budgets": [
          "Warning count equals the length of the dropped vec for that tag: 1 for one proxy Domain rule; exactly 1 warning line per missing tag with count>0",
          "at most 2 warnings per generation (remote + domestic)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1 tests first, inline mod tests of crates/core/src/config/v2ray.rs: test_skipped_derived_warning_payload_names_tag_and_count, test_dns_derived_remote_tag_missing_keeps_domains_off_and_warns, test_dns_derived_standard_tags_attach_without_warning",
        "1.1 add pub const AUTO_SPLIT_REMOTE_TAG=\"remote\" and AUTO_SPLIT_DOMESTIC_TAG=\"domestic\" plus DnsConfig::has_server_tag(&self, tag:&str)->bool in crates/core/src/models/dns.rs; re-export both constants in crates/core/src/models/mod.rs pub use dns::{...}",
        "1.1 add pub(crate) fn skipped_derived_warning(tag:&str, skipped:usize)->Option<String> in crates/core/src/config/common.rs returning Some(\"DNS: no server tagged '{tag}' - skipping {skipped} auto-derived domain entries\") only when skipped>0",
        "1.1 in build_user_dns_servers auto-split branch: after collecting remote_domains/domestic_domains, for each tag where !settings.dns.has_server_tag(tag) log the skipped_derived_warning message via log::warn!; generated server list otherwise byte-identical"
      ]
    },
    {
      "id": "singbox-derived-rule-tag-presence",
      "tasks": [
        "1.2",
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no separate test-writer stage; one rust-coder dispatch writes tests and code together, verified by the chunk's bounded cargo command plus the workspace floor. NO-TESTER-WAIVER: no go-tester stage; the inline tests in crates/core/src/config/singbox.rs are the first codeTasks items, and the integration case in crates/core/tests/singbox_check.rs is a site this chunk names explicitly (seal covers it). Ships: build_dns emits the derived remote/domestic dns.rules only for tags present among the configured servers and warns per missing tag, so no rule references an undefined server.",
      "contract": {
        "states": [
          "rules-emitted",
          "rules-skipped-warned",
          "rules-skipped-silent",
          "custom-rules",
          "dns-off"
        ],
        "transitions": [
          {
            "input": "dns.enabled, use_custom_rules=false, user server tagged remote (domestic) exists, >=1 enabled matching entry",
            "state": "rules-emitted",
            "effect": "set",
            "evidence": "crates/core/src/config/singbox.rs:599-617 pushes rule_set/domain_suffix rules with server \"remote\"/\"domestic\"; spec dns-configuration 'Standard tags unaffected'"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, no server tagged domestic, >=1 direct-action GeoSite/Domain entry",
            "state": "rules-skipped-warned",
            "effect": "no-op",
            "evidence": "spec dns-configuration 'Missing domestic tag on sing-box': dns.rules SHALL contain no rule with \"server\":\"domestic\", warning names domestic; today singbox.rs:608-617 emits it unconditionally"
          },
          {
            "input": "dns.enabled, use_custom_rules=false, tag missing, 0 entries for that action",
            "state": "rules-skipped-silent",
            "effect": "no-op",
            "evidence": "the existing !remote_geosite.is_empty() guards at crates/core/src/config/singbox.rs:599-617 mean nothing would have been emitted; no warning at count 0"
          },
          {
            "input": "dns.enabled, use_custom_rules=true",
            "state": "custom-rules",
            "effect": "no-op",
            "evidence": "crates/core/src/config/singbox.rs:539-566 custom DnsRule path; server_tag validity is enforced by DnsConfig::validate at crates/core/src/models/dns.rs:455-460"
          },
          {
            "input": "dns.enabled=false",
            "state": "dns-off",
            "effect": "no-op",
            "evidence": "crates/core/src/config/singbox.rs:52-63 assemble calls build_dns only when dns.enabled; TUN derives its own plane"
          }
        ],
        "forbidden": "Any dns.rules[] object on the auto-derived path whose \"server\" value names a tag absent from the generated dns.servers[].tag list; hosts/fakeip/tun-exclusion rules must be unaffected by the gate.",
        "seeding": [
          "Only legal path is the shipped generator: default_settings()",
          "settings.dns.enabled=true",
          "settings.dns.use_custom_rules=false",
          "settings.dns.servers = [DnsServerConfig{tag:\"remote\"",
          "protocol:Doh",
          "address:\"1.1.1.1\"",
          "port:None",
          "detour:None}",
          "DnsServerConfig{tag:\"lan\"",
          "protocol:Udp",
          "address:\"223.5.5.5\"",
          "port:None",
          "detour:None}]",
          "rules = one enabled RoutingRule{id:uuid::Uuid::new_v4()",
          "match_condition:RuleMatch::GeoSite{category:\"category-ru\"}",
          "action:RuleAction::Direct",
          "enabled:true",
          "group:None",
          "via_node:None}; call SingboxGenerator.generate(&[ss_node()]",
          "&rules",
          "&settings). Integration seeding: same settings/rules through check_with_rules in crates/core/tests/singbox_check.rs:41-69 gated on sing_box_available()."
        ],
        "budgets": [
          "Warning count = geosite categories + domain patterns collected for that action (scenario value: 1); at most 2 warnings per generation; the gate touches at most 4 derived rule pushes (remote/domestic x rule_set/domain_suffix)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.2 tests first: add #[test] fn missing_domestic_tag_derived_rules_pass_sing_box_check in crates/core/tests/singbox_check.rs following the new_rule_kinds_pass_sing_box_check pattern (sing_box_available() guard, check_with_rules), settings with servers tagged remote+lan and one direct GeoSite rule category-ru; run it BEFORE the build_dns change to record whether the current dangling-reference output passes sing-box check, and note the result in the change log",
        "1.3 tests first, inline mod tests of crates/core/src/config/singbox.rs: test_singbox_derived_domestic_tag_missing_emits_no_domestic_rule, test_singbox_derived_standard_tags_emit_both_split_rules",
        "1.3 build_dns auto-derived branch: gate the remote/domestic rule pushes on settings.dns.has_server_tag(AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG) and log skipped_derived_warning(tag, geosite_len + domains_len); reuse the helpers landed in tasks 1.1 (models consts + common.rs warning) so the predicate cannot drift between generators",
        "1.2/1.3 re-run the integration case after the change: green with no rule referencing domestic"
      ]
    },
    {
      "id": "singbox-derived-tun-cache-edns",
      "tasks": [
        "1.4"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no separate test-writer stage; one rust-coder dispatch writes tests and code together, verified by the chunk's bounded cargo command plus the workspace floor. NO-TESTER-WAIVER: no go-tester stage exists; the inline tests in crates/core/src/config/singbox.rs are the first codeTasks items and are guarded by the source diff. Ships: derived_tun_dns carries dns-level disable_cache and client_subnet on the derived DoH server, mirroring the user-configured path.",
      "contract": {
        "states": [
          "plane-derived-cache-set",
          "plane-derived-plain",
          "user-dns-path",
          "no-dns-section"
        ],
        "transitions": [
          {
            "input": "tun.enabled, dns.enabled=false, settings.dns.disable_cache=true",
            "state": "plane-derived-cache-set",
            "effect": "set",
            "evidence": "spec config-generator 'Cache control and client subnet survive the sing-box derived path'; user path emits dns.disable_cache at crates/core/src/config/singbox.rs:629-632"
          },
          {
            "input": "tun.enabled, dns.enabled=false, settings.dns.client_subnet=Some(ip)",
            "state": "plane-derived-cache-set",
            "effect": "set",
            "evidence": "spec same scenario; user path sets client_subnet per server at crates/core/src/config/singbox.rs:507-510"
          },
          {
            "input": "tun.enabled, dns.enabled=false, disable_cache=false and client_subnet=None",
            "state": "plane-derived-plain",
            "effect": "no-op",
            "evidence": "derived_tun_dns today (crates/core/src/config/singbox.rs:71-105) emits neither key; spec adds keys only when carried"
          },
          {
            "input": "dns.enabled=true (tun on or off)",
            "state": "user-dns-path",
            "effect": "no-op",
            "evidence": "assemble at crates/core/src/config/singbox.rs:52-58 calls build_dns, which already sets both; derived_tun_dns not invoked"
          },
          {
            "input": "tun.enabled=false and dns.enabled=false",
            "state": "no-dns-section",
            "effect": "no-op",
            "evidence": "assemble at crates/core/src/config/singbox.rs:52-63 emits no dns key at all"
          }
        ],
        "forbidden": "A derived plane missing \"disable_cache\": true while settings.dns.disable_cache is true; a derived DoH server (tag DERIVED_DNS_TAG=\"remote\", type https) missing client_subnet while settings.dns.client_subnet is Some; either key present when the setting is unset; client_subnet on the derived hosts server.",
        "seeding": [
          "Only legal path: default_settings()",
          "settings.tun.enabled=true",
          "settings.dns.enabled=false",
          "then set settings.dns.disable_cache=true and/or settings.dns.client_subnet=Some(\"203.0.113.1\".to_string()); call SingboxGenerator.generate(&[ss_node()]",
          "&[]",
          "&settings) and read config[\"dns\"] — never hand-build the dns JSON."
        ],
        "budgets": [
          "Exactly 1 derived DoH server in the derived plane (+1 hosts server only when settings.dns.hosts is non-empty); client_subnet copied verbatim; 0 behavior change when both settings are unset."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.4 tests first, inline mod tests of crates/core/src/config/singbox.rs mirroring test_dns_disable_cache (singbox.rs:1959) and test_dns_client_subnet (singbox.rs:1971): test_derived_tun_dns_carries_disable_cache, test_derived_tun_dns_carries_client_subnet, test_derived_tun_dns_without_cache_settings_emits_neither_key",
        "1.4 derived_tun_dns: when settings.dns.disable_cache set dns_config[\"disable_cache\"]=true at the dns level; when settings.dns.client_subnet is Some set \"client_subnet\" on the DERIVED_DNS_TAG DoH server object only (not on the hosts server)"
      ]
    },
    {
      "id": "dns-preferences-missing-tag-note",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no separate test-writer stage; one rust-coder dispatch writes tests and code together, verified by the chunk's bounded cargo command plus the workspace floor. NO-TESTER-WAIVER: no go-tester stage exists; the inline tests in crates/ui/src/preferences/dns.rs are the first codeTasks items, guarded by the source diff. Ships: while custom DNS rules are off, a primary remote/domestic row whose tag has no configured server states that routing-derived DNS rules for that server are skipped, instead of only 'Not configured'.",
      "contract": {
        "states": [
          "note-shown",
          "plain-not-configured",
          "configured"
        ],
        "transitions": [
          {
            "input": "dns.use_custom_rules=false, no server tagged domestic",
            "state": "note-shown",
            "effect": "set",
            "evidence": "spec dns-preferences-ui 'Domestic tag missing in auto mode': the Domestic row states routing-derived DNS rules for direct traffic are skipped; today render_primary_dns_servers hard-codes \"Not configured - set up in Advanced\" (crates/ui/src/preferences/dns.rs:1121-1125, 1132-1135)"
          },
          {
            "input": "dns.use_custom_rules=false, no server tagged remote",
            "state": "note-shown",
            "effect": "set",
            "evidence": "same requirement applied to the remote row; remote variant names proxied domains"
          },
          {
            "input": "dns.use_custom_rules=true, tag missing",
            "state": "plain-not-configured",
            "effect": "clear",
            "evidence": "spec dns-preferences-ui 'Custom rules active': row shows \"Not configured\" without the skipped-rules note; derived rules are not active"
          },
          {
            "input": "server tagged remote (domestic) exists",
            "state": "configured",
            "effect": "no-op",
            "evidence": "crates/ui/src/preferences/dns.rs:1117-1123 already renders primary_dns_subtitle and enables the edit button; note logic must not fire"
          }
        ],
        "forbidden": "The skipped-rules note on any row while use_custom_rules=true or while the tag has a configured server; any wording change to the configured-row subtitle path.",
        "seeding": [
          "The note decision is a pure helper: primary_row_missing_note(AUTO_SPLIT_DOMESTIC_TAG|AUTO_SPLIT_REMOTE_TAG",
          "use_custom_rules:bool) in crates/ui/src/preferences/dns.rs; render_primary_dns_servers passes ctx.state.borrow().dns.use_custom_rules and the row tag. No GTK initialization is needed to seed the states."
        ],
        "budgets": [
          "1 &'static str per input combination; 0 extra widget allocations beyond the existing set_subtitle call; the predicate is the same DnsConfig::has_server_tag the generators use."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1 tests first, inline mod tests of crates/ui/src/preferences/dns.rs next to test_primary_row_subtitle_appends_private_warning (dns.rs:2052): test_primary_row_missing_note_states_skipped_derived_rules, test_primary_row_missing_note_custom_rules_shows_plain_not_configured",
        "2.1 add fn primary_row_missing_note(tag:&str, use_custom_rules:bool)->&'static str returning \"Not configured - set up in Advanced\" when use_custom_rules, else \"Not configured - routing-derived DNS rules for proxied domains are skipped\" for AUTO_SPLIT_REMOTE_TAG and \"Not configured - routing-derived DNS rules for direct traffic are skipped\" otherwise; use it for both missing-row subtitles in render_primary_dns_servers"
      ]
    },
    {
      "id": "run-verification",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no test-writer stage — 3.1 is the workspace floor the run itself executes at the join and 3.2 is a live end-to-end check no automated test can own. NO-TESTER-WAIVER: no go-tester stage exists for Rust; the floor command and the sing-box integration cases are the verification. Owns no chunk: 3.1 is satisfied by the plan floor, 3.2 by the live check recorded in the run report.",
      "contract": {
        "states": [
          "floor-green",
          "floor-red"
        ],
        "transitions": [
          {
            "input": "the change is implemented and every chunk closed",
            "state": "floor-green",
            "effect": "set",
            "evidence": "tasks.md 3.1 — timeout 10m cargo test --workspace -- --test-threads=4"
          }
        ],
        "forbidden": "merging with a red workspace floor",
        "seeding": [
          "run the plan floor in the worktree"
        ],
        "budgets": [
          "one workspace run per merge attempt"
        ]
      },
      "redTasks": [],
      "codeTasks": []
    }
  ],
  "requirements": [
    {
      "shall": "the generators SHALL emit derived DNS rules for proxy-action domains only when a server tagged `remote` exists",
      "label": "A1",
      "tests": [
        "new: test_skipped_derived_warning_payload_names_tag_and_count (crates/core/src/config/v2ray.rs inline), test_dns_derived_remote_tag_missing_keeps_domains_off_and_warns (v2ray.rs inline), test_singbox_derived_domestic_tag_missing_emits_no_domestic_rule (crates/core/src/config/singbox.rs inline)"
      ]
    },
    {
      "shall": "THEN** `dns.rules` SHALL contain no rule with `\"server\": \"domestic\"`, ",
      "label": "A2",
      "tests": [
        "new: test_singbox_derived_domestic_tag_missing_emits_no_domestic_rule (singbox.rs inline) + missing_domestic_tag_derived_rules_pass_sing_box_check (new, crates/core/tests/singbox_check.rs — real sing-box check, binary present at /usr/bin/sing-box)"
      ]
    },
    {
      "shall": "THEN** no DNS server SHALL carry `domain:example.com` in `domains` and",
      "label": "A3",
      "tests": [
        "new: test_dns_derived_remote_tag_missing_keeps_domains_off_and_warns (crates/core/src/config/v2ray.rs inline)"
      ]
    },
    {
      "shall": "THEN** derived DNS rules SHALL be emitted as before and no warning SHA",
      "label": "A4",
      "tests": [
        "existing test_dns_auto_derive_rules_from_routing (crates/core/src/config/singbox.rs:2115), test_dns_derived_domain_rule_gains_prefix (crates/core/src/config/v2ray.rs:2285); new: test_singbox_derived_standard_tags_emit_both_split_rules (singbox.rs inline)"
      ]
    },
    {
      "shall": "the primary `remote` or `domestic` row whose tag has no configured server SHALL state that DNS rules derived from routing for that server are skipped",
      "label": "B1",
      "tests": [
        "new: test_primary_row_missing_note_states_skipped_derived_rules (crates/ui/src/preferences/dns.rs inline)"
      ]
    },
    {
      "shall": "THEN** the Domestic row SHALL state that routing-derived DNS rules for",
      "label": "B2",
      "tests": [
        "new: test_primary_row_missing_note_states_skipped_derived_rules (crates/ui/src/preferences/dns.rs inline)"
      ]
    },
    {
      "shall": "THEN** the Domestic row SHALL show \"Not configured\" without the skippe",
      "label": "B3",
      "tests": [
        "new: test_primary_row_missing_note_custom_rules_shows_plain_not_configured (crates/ui/src/preferences/dns.rs inline)"
      ]
    },
    {
      "shall": "the generated config SHALL NOT depend on the operating-system resolver for any resolution that feeds routing decisions or direct dials",
      "label": "C1",
      "tests": [
        "test_xray_tun_derives_dns_plane_when_dns_disabled (crates/core/src/config/v2ray.rs:2626), test_xray_tun_freedom_uses_builtin_resolver (v2ray.rs:2973), test_singbox_tun_with_dns_disabled_derives_dns_plane (crates/core/src/config/singbox.rs:1072)"
      ]
    },
    {
      "shall": "Static host overrides, cache control and the EDNS client subnet SHALL ",
      "label": "C2",
      "tests": [
        "test_derived_tun_dns_carries_the_host_pin (singbox.rs:2034), test_xray_tun_emits_hosts_even_when_dns_disabled (v2ray.rs:2752); cache/subnet half new: test_derived_tun_dns_carries_disable_cache, test_derived_tun_dns_carries_client_subnet (singbox.rs inline)"
      ]
    },
    {
      "shall": "For sing-box, dial-time name resolution does not consult `dns.rules`, ",
      "label": "C3",
      "tests": [
        "test_pinned_node_resolves_through_the_hosts_server (crates/core/src/config/singbox.rs:2063)"
      ]
    },
    {
      "shall": "For xray under TUN the generator SHALL emit bootstrap DNS servers for ",
      "label": "C4",
      "tests": [
        "test_xray_tun_bootstrap_servers_are_scoped_and_direct_tagged (v2ray.rs:2799), test_xray_tun_bootstrap_covers_hostname_addressed_dns_servers (v2ray.rs:2870)"
      ]
    },
    {
      "shall": "THEN** the generated config SHALL contain a `dns` section with `\"tag\":",
      "label": "C5",
      "tests": [
        "test_xray_tun_derives_dns_plane_when_dns_disabled (crates/core/src/config/v2ray.rs:2626)"
      ]
    },
    {
      "shall": "THEN** the generated `dns` section SHALL contain a `hosts` object mapp",
      "label": "C6",
      "tests": [
        "test_xray_tun_emits_hosts_even_when_dns_disabled (crates/core/src/config/v2ray.rs:2752)"
      ]
    },
    {
      "shall": "THEN** the emitted `hosts` entry SHALL contain only the IPv4 address, ",
      "label": "C7",
      "tests": [
        "test_hosts_keep_only_the_family_the_strategy_uses (crates/core/src/config/v2ray.rs:2769)"
      ]
    },
    {
      "shall": "THEN** `dns.servers` SHALL begin with a plain-UDP entry and then a DoH",
      "label": "C8",
      "tests": [
        "test_xray_tun_bootstrap_servers_are_scoped_and_direct_tagged (crates/core/src/config/v2ray.rs:2799)"
      ]
    },
    {
      "shall": "THEN** the second SHALL be queried over the same direct route, and the",
      "label": "C9",
      "tests": [
        "test_xray_tun_bootstrap_servers_are_scoped_and_direct_tagged (crates/core/src/config/v2ray.rs:2799) — structural properties only; live fallback is not exercised"
      ]
    },
    {
      "shall": "THEN** that hostname SHALL appear in the bootstrap server's `domains` ",
      "label": "C10",
      "tests": [
        "test_xray_tun_bootstrap_covers_hostname_addressed_dns_servers (crates/core/src/config/v2ray.rs:2870)"
      ]
    },
    {
      "shall": "THEN** the config SHALL contain no `dns-direct` server and no `dns-dir",
      "label": "C11",
      "tests": [
        "test_xray_tun_omits_bootstrap_for_ip_addressed_nodes (crates/core/src/config/v2ray.rs:2850)"
      ]
    },
    {
      "shall": "THEN** the `dns-direct` routing rule SHALL appear before the `{\"networ",
      "label": "C12",
      "tests": [
        "test_dns_direct_rule_precedes_the_internal_and_hijack_rules (crates/core/src/config/v2ray.rs:2830)"
      ]
    },
    {
      "shall": "THEN** the `freedom` outbound SHALL carry `streamSettings.sockopt.doma",
      "label": "C13",
      "tests": [
        "test_xray_tun_freedom_uses_builtin_resolver (crates/core/src/config/v2ray.rs:2973), test_xray_freedom_has_no_domain_strategy_without_tun (v2ray.rs:3004)"
      ]
    },
    {
      "shall": "THEN** the generated config SHALL contain `dns.servers` with the deriv",
      "label": "C14",
      "tests": [
        "test_singbox_tun_with_dns_disabled_derives_dns_plane (crates/core/src/config/singbox.rs:1072)"
      ]
    },
    {
      "shall": "THEN** `dns.servers` SHALL contain a `hosts` server whose `predefined`",
      "label": "C15",
      "tests": [
        "test_derived_tun_dns_carries_the_host_pin (crates/core/src/config/singbox.rs:2034)"
      ]
    },
    {
      "shall": "THEN** that outbound SHALL carry `domain_resolver` naming the `hosts` ",
      "label": "C16",
      "tests": [
        "test_pinned_node_resolves_through_the_hosts_server (crates/core/src/config/singbox.rs:2063)"
      ]
    },
    {
      "shall": "THEN** that outbound SHALL NOT carry `domain_resolver`",
      "label": "C17",
      "tests": [
        "test_unpinned_node_gets_no_hosts_resolver (crates/core/src/config/singbox.rs:2084), test_hosts_resolver_is_not_named_when_no_dns_section_exists (singbox.rs:2097)"
      ]
    },
    {
      "shall": "THEN** the user's servers SHALL be emitted as today, and (xray) the `d",
      "label": "C18",
      "tests": [
        "test_xray_tun_user_dns_gains_internal_tag_and_rule (crates/core/src/config/v2ray.rs:2646)"
      ]
    },
    {
      "shall": "THEN** the derived `dns` section SHALL contain `\"disable_cache\": true`",
      "label": "C19",
      "tests": [
        "new: test_derived_tun_dns_carries_disable_cache, test_derived_tun_dns_carries_client_subnet (crates/core/src/config/singbox.rs inline), mirroring existing test_dns_disable_cache (singbox.rs:1959) and test_dns_client_subnet (singbox.rs:1971)"
      ]
    }
  ],
  "testHarness": [
    "ss_node — crates/core/tests/singbox_check.rs:20 — builds a sample Shadowsocks ProxyNode fixture",
    "dns_servers — crates/core/tests/singbox_check.rs:30 — builds remote DoH (1.1.1.1) and domestic UDP (223.5.5.5) DnsServerConfig pair",
    "check — crates/core/tests/singbox_check.rs:49 — generates Singbox config with ss_node and empty rules and runs sing-box check -c",
    "check_with_rules — crates/core/tests/singbox_check.rs:53 — generates Singbox config with rules and patch closure, writes tempfile, executes sing-box check -c",
    "check_starts — crates/core/tests/singbox_check.rs:89 — strips tun inbound, assigns distinct listen_port, spawns sing-box run -c, checks running and no FATAL",
    "default_settings — crates/core/src/config/test_fixtures.rs:5 — builds default AppSettings fixture",
    "vless_node — crates/core/src/config/test_fixtures.rs:9 — builds default Vless ProxyNode fixture",
    "vmess_node — crates/core/src/config/test_fixtures.rs:30 — builds default Vmess ProxyNode fixture",
    "trojan_node — crates/core/src/config/test_fixtures.rs:53 — builds default Trojan ProxyNode fixture",
    "test_dns_disable_cache — crates/core/src/config/singbox.rs:1959 — asserts config[\"dns\"][\"disable_cache\"] == true",
    "test_dns_client_subnet — crates/core/src/config/singbox.rs:1971 — asserts non-hosts/non-fakeip servers receive client_subnet",
    "test_dns_auto_derive_rules_from_routing — crates/core/src/config/singbox.rs:2115 — tests routing rules mapped to derived singbox dns rules",
    "test_derived_tun_dns_carries_the_host_pin — crates/core/src/config/singbox.rs:2034 — tests derived TUN DNS contains hosts server and rule when DNS disabled",
    "dns_servers_with_tag — crates/core/src/config/v2ray.rs:2496 — helper extracting servers matching given tag from config[\"dns\"][\"servers\"]",
    "test_dns_derived_domain_rule_gains_prefix — crates/core/src/config/v2ray.rs:2285 — tests v2ray derived domain prefix generation for routing rules",
    "primary_dns_subtitle — crates/ui/src/preferences/dns.rs:771 — formats subtitle string for primary DNS rows",
    "test_primary_row_subtitle_appends_private_warning — crates/ui/src/preferences/dns.rs:2053 — unit test for primary row subtitle formatting without GTK"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4",
  "estimateHours": 2.7,
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1
  }
}
```
