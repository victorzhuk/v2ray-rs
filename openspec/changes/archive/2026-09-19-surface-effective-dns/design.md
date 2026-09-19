## Context

- xray and v2ray share `generate_v2ray_family_config` (`crates/core/src/config/v2ray.rs:38`). A `dns` section exists only when `dns.enabled || tun_xray` (`:50`); `tun_xray` is xray with TUN on (`:46`). v2ray never has TUN, so with DNS disabled it emits no `dns` section.
- DNS disabled under xray TUN: servers are `FALLBACK_DNS` + `FALLBACK_DNS_SECONDARY` (`:98`, `:102`, `:631-638`); `dns.servers` is ignored. The in-code reason (`:634-636`): the OS resolver's poisoned answers would feed `IPIfNonMatch` geoip rules. Queries carry `dns.tag = dns-internal` (`:78`), routed to the first proxy outbound (`:492-496`).
- DNS enabled with no servers: `FALLBACK_DNS` under xray TUN, otherwise `localhost` (`:643-649`).
- xray only: a domain-scoped server gets `skipFallback`; if none is unrestricted, `FALLBACK_DNS` is appended (`apply_dns_fallback_policy`, `:118-130`, applied at `:651-653` regardless of TUN).
- xray TUN bootstrap: for non-IP node addresses (and DNS server hostnames when DNS is enabled) two `dns-direct`-tagged servers, UDP `1.1.1.1` then DoH `1.1.1.1`, scoped to those names (`:682-734`), routed `direct` (`:485-491`).
- xray `direct` detour is honored only under TUN (`direct_detour`, `:785-787`). Excluded domains without a split-horizon server go to `localhost` (`:906-911`).
- Outside TUN with rules present, xray/v2ray routing uses `IPIfNonMatch` (`:535-544`); with DNS disabled there is no `dns` section, so the OS resolver answers those lookups. Untagged built-in resolver traffic follows routing; unmatched traffic takes the first outbound.
- sing-box: DNS disabled + TUN → derived plane with optional `hosts` server and DoH `1.1.1.1:443` tagged `remote`, `detour` = first proxy (`crates/core/src/config/singbox.rs:52-58`, `:71-98`). DNS enabled: a server gets `detour` only for a non-`direct` value (`:463-465`); no detour dials directly. A `local` server `sys-dns-bootstrap` is added when no configured server is an IP literal (`:500-511`). Default `remote`/`domestic` servers carry `detour: None` (`crates/core/src/models/dns.rs:366-383`).
- `resolve_effective_config` replaces `settings.dns` wholesale and the routing rules when a subscription node has `use_imported_profile` and a profile (`crates/core/src/models/imported_profile.rs:18-45`).
- The connection task pins node hostnames into `effective_settings.dns.hosts` before generating (`crates/ui/src/connection.rs:139-158`, `:314-346`).
- `write_session_record` writes one `session` line before each launch (`crates/process/src/manager.rs:395-416`), called from `launch` (`:359`), which also runs on crash respawn.

## Goals / Non-Goals

**Goals:**
- One pure function answers "which resolvers, on which path, from where" for a backend, settings, and rules, and the preferences page and `backend.log` both use it.

**Non-Goals:**
- Changing resolver selection, fallback servers, or the bootstrap pair.
- Resolving the v2ray/xray DoH-downgrade question; the summary shows the transport the generator emits.
- Validating server semantics (`validate-dns-server-semantics`) or changing pin behavior (`harden-node-pinning`).

## Decisions

- **Keep fallbacks when DNS is disabled.** Using the user's servers while DNS is disabled was considered and rejected: disabled means the user did not opt in, the listed servers may be the untouched defaults (`223.5.5.5` domestic), and the fallback exists to keep TUN off a poisoned OS resolver (`v2ray.rs:634-636`). Visibility fixes the confusion without changing traffic.
- **Compute from settings, cross-checked against the generator.** A function `effective_dns(backend, settings, rules, node_hosts: &[&str])` in `crates/core/src/config/effective_dns.rs` returns ordered entries `{ address, transport, path, source, scope }` plus flags (`uses_fallback`, `system_resolver`). Alternative rejected: parsing the generated JSON — preferences have no generated config for the current settings, and xray's plain-string server entries lose protocol and scope. Unit tests assert that for a fixture matrix (3 backends × TUN on/off × DNS on/off × scoped/unscoped × IP/hostname nodes) every summary address appears in the generated `dns.servers` and vice versa.
- **Path vocabulary.** `direct`, `proxy`, `routing` (v2ray/xray untagged servers outside TUN: backend routing decides, unmatched traffic goes to the proxy), `system` (OS resolver: `localhost`, sing-box `local`, no `dns` section). `static` for `hosts` entries.
- **Source vocabulary.** `user`, `fallback` (the hardcoded or derived resolvers), `bootstrap`, `system`, `profile` (at connect time, when an imported profile supplied the DNS config).
- **Preferences placement.** A "Effective DNS" group on the DNS page above Advanced, always visible and sensitive even when the master toggle is off, since that is exactly when fallbacks apply. Preferences have no nodes, so bootstrap entries appear as one line "bootstrap resolvers for proxy hostnames (xray TUN)" instead of per-host scope. Subscriptions with an enabled imported profile are listed by name.
- **Log record as its own lines.** One `dns` stream line per entry after the `session` line, e.g. `dns server=https://1.1.1.1/dns-query path=proxy source=fallback scope=all`, written by the manager from a `Vec<String>` supplied via a builder, so each respawn repeats it next to its session. Separate lines avoid colliding with session-record fields added by `log-connection-decisions`.
- **Notice once per connection.** Emitted by the connection task on the first candidate whose summary has `uses_fallback`: `notice: DNS is disabled; TUN resolves through fallback DoH 1.1.1.1/8.8.8.8 via the proxy. Enable DNS in Preferences to use your servers` (text per backend/case). No toast: defaults always carry servers, so a toast would fire on every TUN connect.

## Risks / Trade-offs

- [Summary drifts from generator] → cross-check test matrix fails when either changes.
- [`routing` path is vague] → it is honest: the backend decides by rule; the tooltip names the default.
- [Summary adds lines per launch] → a handful of lines against thousands of access lines; `reduce-backend-log-noise` addresses volume.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, lenses `spec` + `quality` (cross-crate behavior surface with a generator-drift guard; no auth/SQL/secrets/boundary moves). Base SHA `2fc8ee662d464a757ebf8269718aa6b86fa220af`. Stack note: Rust — no test-writer and no tester stage exist; every chunk is one `rust-coder` dispatch that ships its inline tests and code together and closes by waiver. The seal is the pre-seal over `crates/<crate>/tests/*.rs` (integration tests) plus the chunk's literal cargo command; inline `#[cfg(test)] mod tests` are guarded by the source diff.

### Chunks (dispatch order)

**Wave 1** — two parallel shards, disjoint crates:

1. `CH-1-core-effective-dns-model` (tasks 1.1, 1.3; shard `core-config`; parallel) — NEW `crates/core/src/config/effective_dns.rs`: `effective_dns(backend, settings, rules, node_hosts) -> EffectiveDns` with `EffectiveDnsEntry { address, transport: Option<DnsProtocol>, path: DnsPath, source: DnsSource, scope: DnsScope }`, flags `uses_fallback` / `system_resolver`, `EffectiveDns::mark_profile()` (User→Profile relabel, the task-1.3 caller flag), `EffectiveDns::log_lines()` emitting `server={address} path={path} source={source} scope={scope}` per non-static entry. Registers `pub mod effective_dns;` in `crates/core/src/config/mod.rs` (anchor `pub(crate) mod v2ray;`). Tests: one inline scenario test per dns-configuration scenario + `imported_profile_dns_marked_profile` + `uses_fallback_false_with_unscoped_user_server` + `hosts_overrides_are_static_entries` + `log_lines_format_is_server_path_source_scope`. Zero generator changes.
2. `CH-3-process-session-extra` (task 2.1; shard `process-manager`; parallel) — `ProcessManager::with_session_extra(Vec<String>)` (builder beside `with_session_fields`, anchor `fn with_session_fields(`): each string written as a `dns` stream line immediately after the `session` record inside `write_session_record`, routed through the existing `truncate_reason` sanitizer (no new helper), before any backend output; repeats on every crash respawn. Tests: `session_extra_dns_lines_follow_session_on_start`, `session_extra_dns_lines_repeat_after_crash_respawn`, `session_extra_flattens_newlines`, `no_session_extra_writes_no_dns_lines`.

**Wave 2** — after CH-1 merges back into the integration worktree (and CH-3 is closed for CH-4):

3. `CH-2-core-cross-check` (task 1.2; prev CH-1, sharedPkg `v2ray-rs-core`; integration worktree) — `effective_dns_addresses_match_generated_servers` over the 48-case matrix (3 backends × 2 TUN × 2 dns.enabled × 2 node kind, dns-enabled cells split scoped/unscoped): bidirectional set equality between non-static summary addresses and normalized generated `dns.servers` addresses; `system_resolver` iff no dns section or a `localhost`/`local` server; hosts cell asserts `dns.hosts` keys == static-entry scopes. Normalization: v2ray plain-string vs `{address,port}` objects, udp non-default port, sing-box typed servers, skip fakeip/hosts servers.
4. `CH-4-ui-connection-summary-notice` (task 2.2; prev CH-1, shard `ui-connection`; **dispatches only after CH-1 AND CH-3 are both closed** — `with_session_extra` exists only after CH-3) — per candidate in `crates/ui/src/connection.rs`: `EffectiveDns` computed from `effective_settings` (after `resolve_effective_config` + pinning), `mark_profile()` for imported-profile candidates, `.with_session_extra(summary.log_lines())` in the builder chain (anchor `let mut capture_notice_sent = false;`). Pure selector `fallback_dns_notice` beside `STRICT_ROUTE_NOTICE` with three consts, texts binding (tests quote verbatim):
   - `FALLBACK_DNS_DISABLED_NOTICE_XRAY`: `DNS is disabled; TUN resolves through fallback DoH 1.1.1.1/8.8.8.8 via the proxy. Enable DNS in Preferences to use your servers`
   - `FALLBACK_DNS_DISABLED_NOTICE_SINGBOX`: `DNS is disabled; TUN resolves through the fallback DoH 1.1.1.1 via the proxy. Enable DNS in Preferences to use your servers`
   - `FALLBACK_DNS_SCOPED_NOTICE`: `Every configured DNS server is domain-scoped; the fallback 1.1.1.1 answers all other domains. Add an unscoped server in Preferences to change that`
   Notice emitted at most once per connection (flag mirroring `capture_notice_sent`), through `backend_log` `notice` line + `AppMsg::ProcessLogLine`, never a toast. Tests: `fallback_notice_once_across_two_failed_candidates`, `no_notice_with_dns_enabled_unscoped_server`, `dns_lines_follow_session_in_backend_log`, `dns_lines_carry_source_profile_for_imported_profile_candidate`, `fallback_dns_notice` unit tests.

**Wave 3**:

5. `CH-5-ui-preferences-group` (task 3.1; prev CH-4, sharedPkg `v2ray-rs-ui`; integration worktree) — pure `effective_dns_rows(summary, profile_subscriptions) -> Vec<String>` in `crates/ui/src/preferences/dns.rs` (pattern `primary_dns_subtitle`, anchor `let advanced_expander`): `{address} [{transport}] path={path} source={source} scope={scope}`, consecutive Bootstrap entries collapsed to `bootstrap resolvers for proxy hostnames (xray TUN)`, one row per DNS-carrying profile subscription: `nodes from "<name>" use the imported profile's DNS instead`. `adw::PreferencesGroup` above the Advanced expander, always sensitive (never bound to the master toggle), rebuilt through the `subscribe_settings` observers on DNS/TUN/backend change. `show_preferences` threads routing rules + subscriptions into `build_dns_page` (anchor `build_dns_page(&settings_state`). Tests: the five `effective_dns_rows_*` pure-fn tests; the GTK wiring halves verified live per task 3.1.

Tasks 4.1 and 4.2 own no chunk: 4.1 is the workspace floor below, 4.2 is the live end-to-end check recorded in the run report.

### Commands, floor, waivers

- Per-chunk verify (literal): `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` (CH-1, CH-2) · `-p v2ray-rs-process` (CH-3) · `-p v2ray-rs-ui` (CH-4, CH-5). No `redRun` exists — no red stage on this stack.
- Floor: `timeout 10m cargo test --workspace -- --test-threads=4` (task 4.1).
- Waivers: every chunk carries `NO-RED-WAIVER:` (no separate red stage on Rust — the chunk ships its own inline tests) and `NO-TESTER-WAIVER:` (no tester stage; the chunk's literal command plus the floor verify it). The seal is the pre-seal over the crate's integration test files; inline test modules are guarded by the source diff.
- Requirements: all 16 SHALL blocks across the three delta specs map in the appendix to named tests (new inline tests above; the two live-check halves for preferences sensitivity/wiring are named in the requirements, not silent).

### Risks carried

- Summary drifts from the generator — the 48-case bidirectional cross-check (task 1.2) fails when either side changes; `DnsProtocol::server_address` shared between model and generators keeps formatting single-sourced.
- No headless GTK harness exists in `v2ray-rs-ui` — all preferences row text comes from the pure `effective_dns_rows` fn; group construction, sensitivity, and observer wiring are verified live per task 3.1.
- Respawn repetition of dns lines is intended (each session repeats its record next to itself); bounded by `session_extra.len()` per launch, asserted.
- Notice dedup across candidates via the `dns_notice_sent` flag, asserted across a two-candidate failover.
- Subscriptions captured at dialog open — an import while the dialog stays open shows a stale profile list until reopen (spec demands naming on view; accepted).

### Plan review

Verdict **pass**, reviewer `zarchitect`, 5 kernel-counted rounds (round 1: full review, 1 blocker + 6 warnings — chunk codeTasks mispartition, dangling notice-text pointer, phantom `flatten_newlines` helper, matrix arithmetic, drifted evidence ranges, stale harness entry — all corrected; round 3: delta-check, residue in the seam layer found and regenerated; final round: clean, zero blockers, zero warnings).

### Execution rules for a foreign agent (without the kernel)

Standing rules: terse output, no AI/process references in anything written; native file tools for read/search (`rg`/`grep`/`cat`/`sed` through a shell are forbidden); `git -C <dir>` for every git call; every test run resource-limited exactly as the literal commands above; batch independent reads; never re-read a file already read. Work in the assigned worktree only: `git -C <worktree> rev-parse --show-toplevel` must return that worktree or you stop. Every path you touch is worktree-absolute. Commit contract: Conventional Commits, subject ≤72 chars, imperative, lowercase, type from `feat fix refactor perf docs test build ci chore revert`; body lines ≤100; stage only files you edited this run (sole occupant: `git -C <worktree> add -A -- ':!openspec'`); never stage or touch `openspec/`. One verify run per code change, one conventional commit per finished task. A contract test once written is read-only — the only way to change it is an explicit AMEND order naming the test and the one-line why. A blocker (missing input, failing precondition, ambiguous contract) goes into your result packet and you yield immediately; Main is unreachable while you run.

## Plan appendix

```json
{
  "v": 2,
  "change": "surface-effective-dns",
  "baseSha": "2fc8ee662d464a757ebf8269718aa6b86fa220af",
  "generatedAt": "2026-09-18T19:20:43.250Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "CH-1-core-effective-dns-model",
      "taskIds": [
        "1.1",
        "1.3"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "effective-dns-model",
      "shard": "core-config",
      "pkgDirs": [
        "crates/core/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/config/effective_dns.rs",
          "symbol": "effective_dns / EffectiveDns / EffectiveDnsEntry / DnsPath / DnsSource / DnsScope / EffectiveDns::log_lines",
          "anchor": null,
          "change": "NEW module: pure classifier effective_dns(backend, settings, rules, node_hosts) -> EffectiveDns with ordered entries {address, transport: Option<DnsProtocol>, path, source, scope} + uses_fallback + system_resolver; log_lines() emits `server={address} path={path} source={source} scope={scope}` per non-static entry; inline #[cfg(test)] scenario tests (names in codeTasks)"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/mod.rs",
          "symbol": "module declarations",
          "anchor": "pub(crate) mod v2ray;",
          "change": "add `pub mod effective_dns;`"
        },
        {
          "task": "1.3",
          "file": "crates/core/src/config/effective_dns.rs",
          "symbol": "EffectiveDns::mark_profile",
          "anchor": null,
          "change": "NEW: User->Profile relabel driven by the caller flag (invoked after resolve_effective_config applied a profile DNS, crates/core/src/models/imported_profile.rs `pub fn resolve_effective_config(`); fallback/bootstrap/system entries unchanged; test imported_profile_dns_marked_profile with ImportedProfile { dns: Some(..) }"
        }
      ],
      "contract": {
        "states": [
          "entries: ordered Vec<EffectiveDnsEntry>",
          "entry.source in {user, fallback, bootstrap, system, profile}",
          "entry.path in {direct, proxy, routing, system, static}",
          "entry.scope in {All, Domains(Vec<String>)}",
          "uses_fallback flag (any entry source == Fallback)",
          "system_resolver flag (any entry path == System, incl. no-dns-section)",
          "profile-marked (User entries relabeled Profile by mark_profile)"
        ],
        "transitions": [
          {
            "input": "xray, tun.enabled, !dns.enabled",
            "state": "entries = [https://1.1.1.1/dns-query, https://8.8.8.8/dns-query] transport DoH, path=proxy, source=fallback, scope=all; configured servers absent; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec dns-configuration scenario 'xray TUN with DNS disabled'; v2ray.rs:631-638 fallback pair, :492-496 dns-internal routed to first proxy"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, node_hosts contains a hostname",
            "state": "bootstrap pair prepended: 1.1.1.1 UDP then https://1.1.1.1/dns-query DoH, path=direct, source=bootstrap, scope=Domains([hostname])",
            "effect": "set",
            "evidence": "spec scenario 'xray TUN bootstrap for a hostname node'; v2ray.rs:682-734 bootstrap_dns_servers, :485-491 dns-direct routing rule"
          },
          {
            "input": "xray, dns.enabled, every configured server domain-scoped (any TUN state)",
            "state": "configured entries keep Domains scope, then appended https://1.1.1.1/dns-query source=fallback scope=all; uses_fallback=true",
            "effect": "set",
            "evidence": "spec scenario 'xray with only domain-scoped servers'; v2ray.rs:118-130 apply_dns_fallback_policy applied at :651-653"
          },
          {
            "input": "xray or v2ray, !tun.enabled, !dns.enabled",
            "state": "single entry {address: \"system\", transport: None, path=system, source=system, scope=all}; system_resolver=true; uses_fallback=false",
            "effect": "forced",
            "evidence": "spec scenario 'No DNS section outside TUN'; v2ray.rs:50 dns section only when dns.enabled || tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, servers empty, !tun.enabled",
            "state": "entry {address: localhost, transport: None, path=system, source=system}; system_resolver=true",
            "effect": "forced",
            "evidence": "v2ray.rs:643-649 servers empty -> localhost unless tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, !tun.enabled, untagged server (detour != direct or backend v2ray)",
            "state": "user entry path=routing",
            "effect": "set",
            "evidence": "design.md path vocabulary 'routing'; v2ray.rs:535-544 IPIfNonMatch outside TUN, unmatched traffic takes first outbound"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, server.detour == \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "v2ray.rs:785-787 direct_detour honored only under TUN; :485-491"
          },
          {
            "input": "xray TUN, dns.enabled, DNS server addressed by hostname",
            "state": "server hostname joins the bootstrap Domains scope",
            "effect": "set",
            "evidence": "v2ray.rs:764-774 bootstrap_domains includes server addresses when dns.enabled"
          },
          {
            "input": "sing-box, tun.enabled, !dns.enabled",
            "state": "entry {address: https://1.1.1.1/dns-query, transport: DoH, path: proxy, source: fallback, scope: all}; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec scenario 'sing-box TUN with DNS disabled'; singbox.rs:52-58, :71-98 derived_tun_dns with detour = first proxy"
          },
          {
            "input": "sing-box, !tun.enabled, !dns.enabled",
            "state": "single system entry (no dns section)",
            "effect": "forced",
            "evidence": "singbox.rs:50-58 dns emitted only when dns.enabled or tun.enabled"
          },
          {
            "input": "sing-box, dns.enabled, server.detour None or \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "spec scenario 'sing-box server without detour'; singbox.rs:463-465 detour emitted only for non-direct values"
          },
          {
            "input": "sing-box, dns.enabled, server.detour set to non-direct",
            "state": "user entry path=proxy",
            "effect": "set",
            "evidence": "singbox.rs:463-465 detour = first_proxy_tag"
          },
          {
            "input": "sing-box, dns.enabled, no configured server is an IP literal and servers non-empty",
            "state": "appended entry {address: local, transport: None, path=system, source=system}",
            "effect": "set",
            "evidence": "singbox.rs:500-511 sys-dns-bootstrap local server; BOOTSTRAP_RESOLVER_TAG singbox.rs:160-170"
          },
          {
            "input": "settings.dns.hosts non-empty and a dns section exists for this backend state",
            "state": "static entries {address: domain, transport: None, path=static, source=user, scope=Domains([domain])}; excluded from log_lines and from the cross-check address set",
            "effect": "set",
            "evidence": "design.md path vocabulary 'static for hosts entries'; v2ray.rs hosts_for_strategy; singbox.rs:100-131 hosts_server/hosts_rule"
          },
          {
            "input": "caller invokes EffectiveDns::mark_profile() after resolve_effective_config applied a profile DNS",
            "state": "every User entry relabeled Profile; fallback/bootstrap/system entries unchanged",
            "effect": "forced",
            "evidence": "task 1.3; spec scenario 'Imported profile supplies DNS'; imported_profile.rs:18-45"
          },
          {
            "input": "xray/v2ray, dns.enabled, at least one unscoped user server",
            "state": "no appended fallback entry; uses_fallback=false",
            "effect": "clear",
            "evidence": "v2ray.rs:118-130 fallback appended only when no unrestricted server remains"
          },
          {
            "input": "v2ray backend, user server protocol Dot/Doq/H3",
            "state": "entry transport/address follow effective_for_backend downgrade to DoH",
            "effect": "forced",
            "evidence": "models/dns.rs DnsProtocol::fallback_protocol_for_backend / effective_for_backend; v2ray.rs dns_server_address_for_backend"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, untagged server (detour != direct)",
            "state": "user entry path=proxy (queries carry dns.tag=dns-internal routed to the first proxy outbound)",
            "effect": "set",
            "evidence": "v2ray.rs:492-496 dns-internal routing"
          }
        ],
        "forbidden": [
          "any change to generator output: no edit to the resolver-emitting logic of crates/core/src/config/{v2ray.rs, singbox.rs, xray.rs, common.rs, writer.rs} — the cross-check (task 1.2) and the existing generator tests pin it",
          "uses_fallback == true when DNS is enabled and an unscoped user server exists (xray/v2ray)",
          "configured user servers appearing in the summary when dns.enabled == false under xray TUN (spec scenario 1 forbids it)",
          "effective_dns performing IO: no generated-JSON parsing, no file reads (design.md rejected alternative)",
          "mark_profile relabeling fallback, bootstrap, or system entries",
          "summary non-static address set diverging from the generated dns.servers addresses for any matrix cell (bidirectional set equality)"
        ],
        "seeding": [
          "default_settings() from crates/core/src/config/test_fixtures.rs; node fixtures vless_node() (hostname example.com); an IP-address node by overriding address on a fixture (mutation pattern: test_v2ray_verify_off_emits_allow_insecure in v2ray.rs tests)",
          "settings.tun.enabled / settings.dns.enabled / settings.dns.servers / settings.dns.rules / settings.dns.use_custom_rules mutated directly on the fixture",
          "scoped server: DnsServerConfig { tag, protocol, address, port, detour } (models/dns.rs:99-105) + DnsRule { match_condition: DnsRuleMatch::DomainSuffix{..}, server_tag } with use_custom_rules=true; auto-split scope via server tag \"remote\"/\"domestic\" (AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG, models/dns.rs:6-8) plus routing rules",
          "generated config for the cross-check: generate_v2ray_family_config(&nodes, &rules, &settings, V2rayFamilyBackend::{Xray,V2ray}) (pub(crate), v2ray.rs:86-104; the #[cfg(test)] build_dns wrapper pattern exists at v2ray.rs:623-627) and SingboxGenerator::generate (config/mod.rs:88-95)",
          "profile fixture: Subscription::new_from_url(\"Provider\", \"https://example.com/sub\") + imported_profile = Some(ImportedProfile { dns: Some(..), .. }) + use_imported_profile = true (subscription.rs:8-20, :182-187), resolved through resolve_effective_config (imported_profile.rs:18-45)"
        ],
        "budgets": [
          "cross-check matrix: 3 backends x 2 TUN x 2 dns.enabled x 2 IP/hostname node, dns-enabled cells split scoped/unscoped (disabled cells ignore servers) = 48 cases, pure, zero network/IO, inside the 5m core test budget",
          "entry count bounded by servers.len() + 2 (bootstrap pair) + 1 (appended fallback) + 1 (system/local) + hosts.len() static entries",
          "0 diffs to generator files (forbidden list; observed by the same cargo test -p v2ray-rs-core run keeping existing generator tests green)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (inline #[cfg(test)] mod in effective_dns.rs), one per dns-configuration scenario quoting entry fields verbatim: xray_tun_dns_disabled_lists_fallback_pair_not_configured; xray_all_servers_scoped_appends_unscoped_fallback; xray_tun_hostname_bootstrap_pair_direct_scoped; singbox_tun_dns_disabled_derived_doh_fallback; singbox_server_without_detour_is_direct; no_dns_section_outside_tun_single_system_entry (v2ray + xray + sing-box); imported_profile_dns_marked_profile (task 1.3, ImportedProfile { dns: Some(..) }); uses_fallback_false_with_unscoped_user_server; hosts_overrides_are_static_entries; log_lines_format_is_server_path_source_scope",
        "impl: pub struct EffectiveDns { entries: Vec<EffectiveDnsEntry>, uses_fallback: bool, system_resolver: bool }; pub struct EffectiveDnsEntry { address: String, transport: Option<DnsProtocol>, path: DnsPath, source: DnsSource, scope: DnsScope }; pub enum DnsPath { Direct, Proxy, Routing, System, Static }; pub enum DnsSource { User, Fallback, Bootstrap, System, Profile }; pub enum DnsScope { All, Domains(Vec<String>) } (Debug, Clone, PartialEq, Eq)",
        "impl: pub fn effective_dns(backend: BackendType, settings: &AppSettings, rules: &[RoutingRule], node_hosts: &[&str]) -> EffectiveDns",
        "impl: EffectiveDns::mark_profile(&mut self) — relabels User -> Profile only (task 1.3 caller flag)",
        "impl: EffectiveDns::log_lines(&self) -> Vec<String> — one line per non-static entry, exact format `server={address} path={path} source={source} scope={scope}` with lowercase vocabulary direct|proxy|routing|system and user|fallback|bootstrap|system|profile; scope=all or comma-joined domains",
        "impl: crates/core/src/config/mod.rs — add `pub mod effective_dns;`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it."
    },
    {
      "id": "CH-2-core-cross-check",
      "taskIds": [
        "1.2"
      ],
      "prev": "CH-1-core-effective-dns-model",
      "sharedPkg": "v2ray-rs-core",
      "parallel": false,
      "seam": "effective-dns-model",
      "shard": "",
      "pkgDirs": [
        "crates/core/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/core/src/config/effective_dns.rs",
          "symbol": "tests :: effective_dns_addresses_match_generated_servers (cross-check matrix)",
          "anchor": null,
          "change": "NEW test (file created by CH-1): 48-case matrix — 3 backends x 2 TUN x 2 dns.enabled x 2 IP/hostname node, dns-enabled cells split scoped/unscoped (disabled cells ignore servers) — set equality between non-static summary addresses and normalized generated dns.servers addresses; system_resolver == (no dns section || localhost || local server); hosts cell asserts dns.hosts keys == static-entry scopes; normalization helper covers v2ray plain-string vs {address,port} objects, udp non-default port, sing-box typed servers, skips fakeip/hosts servers; generators reached via generate_v2ray_family_config (pub(crate)) and SingboxGenerator::generate"
        }
      ],
      "contract": {
        "states": [
          "entries: ordered Vec<EffectiveDnsEntry>",
          "entry.source in {user, fallback, bootstrap, system, profile}",
          "entry.path in {direct, proxy, routing, system, static}",
          "entry.scope in {All, Domains(Vec<String>)}",
          "uses_fallback flag (any entry source == Fallback)",
          "system_resolver flag (any entry path == System, incl. no-dns-section)",
          "profile-marked (User entries relabeled Profile by mark_profile)"
        ],
        "transitions": [
          {
            "input": "xray, tun.enabled, !dns.enabled",
            "state": "entries = [https://1.1.1.1/dns-query, https://8.8.8.8/dns-query] transport DoH, path=proxy, source=fallback, scope=all; configured servers absent; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec dns-configuration scenario 'xray TUN with DNS disabled'; v2ray.rs:631-638 fallback pair, :492-496 dns-internal routed to first proxy"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, node_hosts contains a hostname",
            "state": "bootstrap pair prepended: 1.1.1.1 UDP then https://1.1.1.1/dns-query DoH, path=direct, source=bootstrap, scope=Domains([hostname])",
            "effect": "set",
            "evidence": "spec scenario 'xray TUN bootstrap for a hostname node'; v2ray.rs:682-734 bootstrap_dns_servers, :485-491 dns-direct routing rule"
          },
          {
            "input": "xray, dns.enabled, every configured server domain-scoped (any TUN state)",
            "state": "configured entries keep Domains scope, then appended https://1.1.1.1/dns-query source=fallback scope=all; uses_fallback=true",
            "effect": "set",
            "evidence": "spec scenario 'xray with only domain-scoped servers'; v2ray.rs:118-130 apply_dns_fallback_policy applied at :651-653"
          },
          {
            "input": "xray or v2ray, !tun.enabled, !dns.enabled",
            "state": "single entry {address: \"system\", transport: None, path=system, source=system, scope=all}; system_resolver=true; uses_fallback=false",
            "effect": "forced",
            "evidence": "spec scenario 'No DNS section outside TUN'; v2ray.rs:50 dns section only when dns.enabled || tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, servers empty, !tun.enabled",
            "state": "entry {address: localhost, transport: None, path=system, source=system}; system_resolver=true",
            "effect": "forced",
            "evidence": "v2ray.rs:643-649 servers empty -> localhost unless tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, !tun.enabled, untagged server (detour != direct or backend v2ray)",
            "state": "user entry path=routing",
            "effect": "set",
            "evidence": "design.md path vocabulary 'routing'; v2ray.rs:535-544 IPIfNonMatch outside TUN, unmatched traffic takes first outbound"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, server.detour == \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "v2ray.rs:785-787 direct_detour honored only under TUN; :485-491"
          },
          {
            "input": "xray TUN, dns.enabled, DNS server addressed by hostname",
            "state": "server hostname joins the bootstrap Domains scope",
            "effect": "set",
            "evidence": "v2ray.rs:764-774 bootstrap_domains includes server addresses when dns.enabled"
          },
          {
            "input": "sing-box, tun.enabled, !dns.enabled",
            "state": "entry {address: https://1.1.1.1/dns-query, transport: DoH, path: proxy, source: fallback, scope: all}; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec scenario 'sing-box TUN with DNS disabled'; singbox.rs:52-58, :71-98 derived_tun_dns with detour = first proxy"
          },
          {
            "input": "sing-box, !tun.enabled, !dns.enabled",
            "state": "single system entry (no dns section)",
            "effect": "forced",
            "evidence": "singbox.rs:50-58 dns emitted only when dns.enabled or tun.enabled"
          },
          {
            "input": "sing-box, dns.enabled, server.detour None or \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "spec scenario 'sing-box server without detour'; singbox.rs:463-465 detour emitted only for non-direct values"
          },
          {
            "input": "sing-box, dns.enabled, server.detour set to non-direct",
            "state": "user entry path=proxy",
            "effect": "set",
            "evidence": "singbox.rs:463-465 detour = first_proxy_tag"
          },
          {
            "input": "sing-box, dns.enabled, no configured server is an IP literal and servers non-empty",
            "state": "appended entry {address: local, transport: None, path=system, source=system}",
            "effect": "set",
            "evidence": "singbox.rs:500-511 sys-dns-bootstrap local server; BOOTSTRAP_RESOLVER_TAG singbox.rs:160-170"
          },
          {
            "input": "settings.dns.hosts non-empty and a dns section exists for this backend state",
            "state": "static entries {address: domain, transport: None, path=static, source=user, scope=Domains([domain])}; excluded from log_lines and from the cross-check address set",
            "effect": "set",
            "evidence": "design.md path vocabulary 'static for hosts entries'; v2ray.rs hosts_for_strategy; singbox.rs:100-131 hosts_server/hosts_rule"
          },
          {
            "input": "caller invokes EffectiveDns::mark_profile() after resolve_effective_config applied a profile DNS",
            "state": "every User entry relabeled Profile; fallback/bootstrap/system entries unchanged",
            "effect": "forced",
            "evidence": "task 1.3; spec scenario 'Imported profile supplies DNS'; imported_profile.rs:18-45"
          },
          {
            "input": "xray/v2ray, dns.enabled, at least one unscoped user server",
            "state": "no appended fallback entry; uses_fallback=false",
            "effect": "clear",
            "evidence": "v2ray.rs:118-130 fallback appended only when no unrestricted server remains"
          },
          {
            "input": "v2ray backend, user server protocol Dot/Doq/H3",
            "state": "entry transport/address follow effective_for_backend downgrade to DoH",
            "effect": "forced",
            "evidence": "models/dns.rs DnsProtocol::fallback_protocol_for_backend / effective_for_backend; v2ray.rs dns_server_address_for_backend"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, untagged server (detour != direct)",
            "state": "user entry path=proxy (queries carry dns.tag=dns-internal routed to the first proxy outbound)",
            "effect": "set",
            "evidence": "v2ray.rs:492-496 dns-internal routing"
          }
        ],
        "forbidden": [
          "any change to generator output: no edit to the resolver-emitting logic of crates/core/src/config/{v2ray.rs, singbox.rs, xray.rs, common.rs, writer.rs} — the cross-check (task 1.2) and the existing generator tests pin it",
          "uses_fallback == true when DNS is enabled and an unscoped user server exists (xray/v2ray)",
          "configured user servers appearing in the summary when dns.enabled == false under xray TUN (spec scenario 1 forbids it)",
          "effective_dns performing IO: no generated-JSON parsing, no file reads (design.md rejected alternative)",
          "mark_profile relabeling fallback, bootstrap, or system entries",
          "summary non-static address set diverging from the generated dns.servers addresses for any matrix cell (bidirectional set equality)"
        ],
        "seeding": [
          "default_settings() from crates/core/src/config/test_fixtures.rs; node fixtures vless_node() (hostname example.com); an IP-address node by overriding address on a fixture (mutation pattern: test_v2ray_verify_off_emits_allow_insecure in v2ray.rs tests)",
          "settings.tun.enabled / settings.dns.enabled / settings.dns.servers / settings.dns.rules / settings.dns.use_custom_rules mutated directly on the fixture",
          "scoped server: DnsServerConfig { tag, protocol, address, port, detour } (models/dns.rs:99-105) + DnsRule { match_condition: DnsRuleMatch::DomainSuffix{..}, server_tag } with use_custom_rules=true; auto-split scope via server tag \"remote\"/\"domestic\" (AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG, models/dns.rs:6-8) plus routing rules",
          "generated config for the cross-check: generate_v2ray_family_config(&nodes, &rules, &settings, V2rayFamilyBackend::{Xray,V2ray}) (pub(crate), v2ray.rs:86-104; the #[cfg(test)] build_dns wrapper pattern exists at v2ray.rs:623-627) and SingboxGenerator::generate (config/mod.rs:88-95)",
          "profile fixture: Subscription::new_from_url(\"Provider\", \"https://example.com/sub\") + imported_profile = Some(ImportedProfile { dns: Some(..), .. }) + use_imported_profile = true (subscription.rs:8-20, :182-187), resolved through resolve_effective_config (imported_profile.rs:18-45)"
        ],
        "budgets": [
          "cross-check matrix: 3 backends x 2 TUN x 2 dns.enabled x 2 IP/hostname node, dns-enabled cells split scoped/unscoped (disabled cells ignore servers) = 48 cases, pure, zero network/IO, inside the 5m core test budget",
          "entry count bounded by servers.len() + 2 (bootstrap pair) + 1 (appended fallback) + 1 (system/local) + hosts.len() static entries",
          "0 diffs to generator files (forbidden list; observed by the same cargo test -p v2ray-rs-core run keeping existing generator tests green)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "cross-check tests (task 1.2): effective_dns_addresses_match_generated_servers over the 48-case matrix (3 backends x 2 TUN x 2 dns.enabled x 2 node kind, dns-enabled cells split scoped/unscoped) — set equality between non-static summary addresses and normalized generated dns.servers addresses; system_resolver true iff no dns section or a localhost/local server; hosts cell: dns.hosts keys equal static-entry scopes"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it."
    },
    {
      "id": "CH-3-process-session-extra",
      "taskIds": [
        "2.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "session-extra-dns-records",
      "shard": "process-manager",
      "pkgDirs": [
        "crates/process/tests"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::with_session_extra + write_session_record",
          "anchor": "fn with_session_fields(",
          "change": "session_extra: Vec<String> field + builder beside with_session_fields; in write_session_record, after the session line, one dns stream line per extra (newline-flattened), before any backend output; repeats on every crash respawn"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "mod tests (session-record + respawn tests)",
          "anchor": "fn manager_for(",
          "change": "tests: session_extra_dns_lines_follow_session_on_start; session_extra_dns_lines_repeat_after_crash_respawn; session_extra_flattens_newlines; no_session_extra_writes_no_dns_lines — via manager_for/backend_log/read_lines/wait_for_lines helpers"
        }
      ],
      "contract": {
        "states": [
          "manager.session_extra: Vec<String> (default empty)",
          "launch log tail: session line followed by 0..N dns lines, before any backend output",
          "crash respawn: block repeats after the new session line"
        ],
        "transitions": [
          {
            "input": "with_session_extra(lines) on the builder",
            "state": "session_extra stored on the manager",
            "effect": "set",
            "evidence": "builder pattern beside with_session_fields, manager.rs:275"
          },
          {
            "input": "no with_session_extra call",
            "state": "session_extra empty, session record identical to today",
            "effect": "no-op",
            "evidence": "default init in ProcessManager::new, manager.rs:183-214"
          },
          {
            "input": "launch() with N extras and a log file attached",
            "state": "session line then N dns lines (`<rfc3339> dns <line>`), written before try_spawn so no backend output can interleave",
            "effect": "set",
            "evidence": "write_session_record is the first call in launch, manager.rs:682-713; write_stream_line manager.rs:1134; RotatingFileWriter::append_line format rotating_log.rs:91-98"
          },
          {
            "input": "crash respawn (auto_restart path)",
            "state": "new session line followed by the same N dns lines",
            "effect": "forced",
            "evidence": "respawn -> launch, manager.rs:594-603; existing respawn ordering test manager.rs:1865-1877"
          },
          {
            "input": "preflight failure (binary/config missing, version/capability/helper gate)",
            "state": "no session line and no dns lines",
            "effect": "no-op",
            "evidence": "launch never reached; existing tests manager.rs:2043-2082 assert no session record"
          },
          {
            "input": "no log file attached (with_log_file(None))",
            "state": "nothing written",
            "effect": "no-op",
            "evidence": "write_stream_line no-ops on None writer, manager.rs:1134"
          },
          {
            "input": "an extra string containing \\n or \\r",
            "state": "flattened onto one physical line, cannot forge a second record",
            "effect": "forced",
            "evidence": "line-forging guard pattern: session_fields tests manager.rs:1531-1549 and :1551-1590"
          }
        ],
        "forbidden": [
          "dns lines anywhere but immediately after the session line (never before it, never after backend output)",
          "an extra line forging additional physical lines (newline injection)",
          "any change to the session record format (session backend=... fields, manager.rs:703-712, pinned by tests manager.rs:1509-1549)",
          "dns lines on a preflight-failed start",
          "any change to ProcessManager lifecycle behavior other than the appended lines (state machine, respawn budget untouched)"
        ],
        "seeding": [
          "manager_for(&dir, &format!(\"{VERSION_STUB}exec sleep 30\\n\")) + .with_log_file(Some(backend_log(dir.path()))) + .with_session_extra(vec![..]) — exact pattern of session_record_appends_caller_fields, manager.rs:1509-1529",
          "assertions via read_lines / wait_for_lines on dir.path().join(\"backend.log\") (manager.rs tests helpers)",
          "crash respawn seeding: stub that fails the first start then sleeps, driven through wait_and_handle_exit + auto respawn — pattern of the session/exit/session ordering test manager.rs:1865-1877"
        ],
        "budgets": [
          "dns lines per launch == session_extra.len() exactly (test pins 2 after session on start, and again 2 after the respawn session: 4 dns lines across 2 launches)",
          "exactly 1 session line per launch, unchanged"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (manager.rs inline tests): session_extra_dns_lines_follow_session_on_start (two dns lines directly after the session line); session_extra_dns_lines_repeat_after_crash_respawn (session, dns, dns, exit, session, dns, dns); session_extra_flattens_newlines; no_session_extra_writes_no_dns_lines",
        "impl: ProcessManager field `session_extra: Vec<String>` (Vec::new() in new()); `pub fn with_session_extra(mut self, lines: Vec<String>) -> Self` beside with_session_fields (manager.rs:275)",
        "impl: in write_session_record, after write_stream_line(&self.log_writer, \"session\", &record): `for line in &self.session_extra { write_stream_line(&self.log_writer, \"dns\", line) routed through the same truncate_reason sanitizer session_fields already flows through (manager.rs write_session_record) — no new helper; }`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it."
    },
    {
      "id": "CH-4-ui-connection-summary-notice",
      "taskIds": [
        "2.2"
      ],
      "prev": "CH-1-core-effective-dns-model",
      "sharedPkg": null,
      "parallel": true,
      "seam": "connection-summary-notice",
      "shard": "ui-connection",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with candidate loop",
          "anchor": "let mut capture_notice_sent = false;",
          "change": "compute EffectiveDns from effective_settings per candidate (after resolve_effective_config + apply_pins); mark_profile() when the candidate uses an imported profile; .with_session_extra(summary.log_lines()) in the builder chain; fallback notice emitted once per connection (dns_notice_sent flag, backend_log notice line + AppMsg::ProcessLogLine, no toast)"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "fallback_dns_notice + FALLBACK_DNS_DISABLED_NOTICE_XRAY / _SINGBOX / FALLBACK_DNS_SCOPED_NOTICE",
          "anchor": "STRICT_ROUTE_NOTICE",
          "change": "pure case selector beside STRICT_ROUTE_NOTICE with verbatim design texts; unit tests quote the consts"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "mod tests",
          "anchor": "fn connect_with(",
          "change": "tests via connect_with/candidate/stub harness: fallback_notice_once_across_two_failed_candidates; no_notice_with_dns_enabled_unscoped_server; dns_lines_follow_session_in_backend_log; dns_lines_carry_source_profile_for_imported_profile_candidate; fallback_dns_notice unit tests"
        }
      ],
      "contract": {
        "states": [
          "per-candidate dns summary lines handed to the manager (source=profile when the profile supplied DNS)",
          "dns_notice_sent flag per connection (unsent -> sent)",
          "notice text selected by case (xray disabled / sing-box disabled / xray scoped-only)"
        ],
        "transitions": [
          {
            "input": "candidate iteration with effective_settings (post resolve_effective_config + apply_pins)",
            "state": "summary = effective_dns(backend, &effective_settings, &effective_rules, node_hosts of this candidate's nodes); lines = summary.log_lines() passed via .with_session_extra(lines)",
            "effect": "set",
            "evidence": "candidate loop crates/ui/src/connection.rs:236-271 (resolve_effective_config, pins) and builder chain :347-358; spec diagnostic-logs 'computed from the settings actually used for that launch'"
          },
          {
            "input": "candidate whose subscription profile replaced DNS (uses_imported_profile)",
            "state": "summary.mark_profile() before log_lines; dns line carries source=profile",
            "effect": "forced",
            "evidence": "imported_profile.rs:18-45; session_fields already stamps profile=imported, connection.rs:213"
          },
          {
            "input": "candidate summary uses_fallback && !dns_notice_sent",
            "state": "notice emitted once: backend_log append_line(\"notice\", text) + sender.emit(AppMsg::ProcessLogLine(generation, text)); flag set",
            "effect": "set",
            "evidence": "design 'Notice once per connection'; emission pattern STRICT_ROUTE_NOTICE connection.rs:150-173; flag pattern capture_notice_sent connection.rs:213"
          },
          {
            "input": "later candidate with uses_fallback while flag already set (failover)",
            "state": "no second notice",
            "effect": "no-op",
            "evidence": "spec diagnostic-logs scenario 'Fallback notice once per connection'"
          },
          {
            "input": "candidate summary with uses_fallback == false (DNS enabled, unscoped user server)",
            "state": "no notice at any point, flag stays unsent",
            "effect": "clear",
            "evidence": "spec diagnostic-logs scenario 'No notice with user resolvers'"
          },
          {
            "input": "notice case selection: (!dns.enabled && tun && xray) | (!dns.enabled && tun && sing-box) | (dns.enabled && fallback-from-scoped)",
            "state": "FALLBACK_DNS_DISABLED_NOTICE_XRAY | FALLBACK_DNS_DISABLED_NOTICE_SINGBOX | FALLBACK_DNS_SCOPED_NOTICE",
            "effect": "set",
            "evidence": "design notice text verbatim; pure selector fallback_dns_notice"
          },
          {
            "input": "writer.write_config failed for a candidate",
            "state": "no manager built, no dns lines, loop continues to next candidate",
            "effect": "no-op",
            "evidence": "connection.rs:296-313 error path"
          }
        ],
        "forbidden": [
          "notice emitted more than once per connection (any candidate count)",
          "notice when uses_fallback == false (DNS enabled with an unscoped server)",
          "notice delivered as a toast (AppMsg::ShowToast) — design decision: no toast",
          "dns lines computed from the global settings instead of effective_settings (imported profile or connect-time pins would be lost)",
          "dns lines written for a candidate that never launched (config-generation failure path)",
          "any change to pinning behavior, candidate ordering, or failure handling (harden-node-pinning is a different change)"
        ],
        "seeding": [
          "connect(&stub, settings, candidates) / connect_with(&stub, settings, candidates, configure) harness — connection.rs:1238-1308; stub(script) :1205; candidate(address) :1226; ready_singbox_settings() :1310",
          "xray TUN DNS-off settings: settings.backend.backend_type = BackendType::Xray; settings.tun.enabled = true; settings.dns.enabled = false (backend-mutation pattern of xray_dns_burst_reports_dns_health, connection.rs:2887-2895)",
          "two failing candidates: stub script exiting non-zero for the first candidate's config (pattern two_candidates_first_exits_before_ready, connection.rs:2551-2575); messages drained via the AppMsg collectors at connection.rs:1719-1723 and :2303-2307",
          "profile candidate: ConnectionCandidate with node_ref ConnectionNodeRef::Subscription { .. } and the subscription fixture (imported_profile dns Some) inside the request's subscriptions"
        ],
        "budgets": [
          "notice count per connection <= 1 (asserted == 1 across two failed candidates)",
          "dns lines per backend.log session block == summary.entries.len() minus static entries (fallback-pair fixture: 2)",
          "0 toasts for the fallback notice"
        ],
        "dispatch_order": "CH-4 dispatches only after BOTH CH-1-core-effective-dns-model (prev) and CH-3-process-session-extra are closed: .with_session_extra exists only after CH-3 lands; compiling crates/ui against v2ray-rs-process requires it",
        "notice_texts": {
          "FALLBACK_DNS_DISABLED_NOTICE_XRAY": "DNS is disabled; TUN resolves through fallback DoH 1.1.1.1/8.8.8.8 via the proxy. Enable DNS in Preferences to use your servers",
          "FALLBACK_DNS_DISABLED_NOTICE_SINGBOX": "DNS is disabled; TUN resolves through the fallback DoH 1.1.1.1 via the proxy. Enable DNS in Preferences to use your servers",
          "FALLBACK_DNS_SCOPED_NOTICE": "Every configured DNS server is domain-scoped; the fallback 1.1.1.1 answers all other domains. Add an unscoped server in Preferences to change that"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (connection.rs inline tests): fallback_notice_once_across_two_failed_candidates (exactly one ProcessLogLine with the notice prefix across two candidates); no_notice_with_dns_enabled_unscoped_server (zero notice lines); dns_lines_follow_session_in_backend_log (two ` dns server=` lines directly after the ` session ` line); dns_lines_carry_source_profile_for_imported_profile_candidate; fallback_dns_notice unit tests quoting the three consts verbatim",
        "impl: pure fn fallback_dns_notice(summary: &EffectiveDns, dns_enabled: bool, tun: bool, backend: BackendType) -> Option<&'static str> beside STRICT_ROUTE_NOTICE, with the exact texts pinned in contract.notice_texts (tests quote them verbatim)",
        "impl: candidate loop computes EffectiveDns from effective_settings (+ mark_profile when uses_imported_profile), passes .with_session_extra(summary.log_lines()) in the builder chain, emits the notice gated by a dns_notice_sent flag mirroring capture_notice_sent (connection.rs:213), through backend_log append_line(\"notice\", ..) + AppMsg::ProcessLogLine only"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it."
    },
    {
      "id": "CH-5-ui-preferences-group",
      "taskIds": [
        "3.1"
      ],
      "prev": "CH-4-ui-connection-summary-notice",
      "sharedPkg": "v2ray-rs-ui",
      "parallel": false,
      "seam": "preferences-effective-dns",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "effective_dns_rows + 'Effective DNS' group in build_dns_page",
          "anchor": "let advanced_expander",
          "change": "pure formatter effective_dns_rows(summary, profile_subscriptions) (headless-testable like primary_dns_subtitle); adw::PreferencesGroup above the Advanced expander, always sensitive, rebuilt through the settings observers on DNS/TUN/backend change, consecutive Bootstrap entries collapsed to one line, one row per DNS-carrying profile subscription"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/mod.rs",
          "symbol": "show_preferences / build_dns_page call",
          "anchor": "build_dns_page(&settings_state",
          "change": "thread routing rules and subscriptions into build_dns_page (single caller; subscriptions via store.load_subscriptions() captured at dialog open)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "mod tests (pure-fn)",
          "anchor": "fn primary_row_missing_note(",
          "change": "tests: effective_dns_rows_lists_fallback_pair_when_dns_off; effective_dns_rows_follow_enabled_dns; effective_dns_rows_names_imported_profile_subscription; effective_dns_rows_collapses_bootstrap_pair; effective_dns_rows_no_profile_row_without_profiles"
        }
      ],
      "contract": {
        "states": [
          "summary rows rendered from (settings, rules, subscriptions) for the current backend/TUN state",
          "group visible and sensitive regardless of dns master toggle",
          "profile subscription rows present/absent"
        ],
        "transitions": [
          {
            "input": "DNS page built (dialog opened)",
            "state": "'Effective DNS' group above Advanced, populated via effective_dns(backend, settings, rules, &[])",
            "effect": "set",
            "evidence": "design 'Preferences placement'; build_dns_page crates/ui/src/preferences/dns.rs, loaded by show_preferences preferences/mod.rs:81"
          },
          {
            "input": "settings observer fires (DNS toggle, TUN toggle, backend change)",
            "state": "rows rebuilt from the new settings snapshot — no dialog reopen",
            "effect": "set",
            "evidence": "subscribe_settings observers preferences/mod.rs:108-113 fanned out from settings_cb :49-58; spec dns-preferences-ui scenario 'Summary follows edits'"
          },
          {
            "input": "dns.enabled == false under xray TUN",
            "state": "both fallback DoH rows shown (path=proxy, source=fallback); group stays sensitive",
            "effect": "forced",
            "evidence": "spec dns-preferences-ui scenario 'Fallback shown while DNS is off'; design 'always visible and sensitive even when the master toggle is off'"
          },
          {
            "input": "dns.enabled toggled true",
            "state": "fallback rows replaced by configured-server rows in place",
            "effect": "set",
            "evidence": "spec dns-preferences-ui scenario 'Summary follows edits'"
          },
          {
            "input": "subscription with use_imported_profile && imported_profile.dns == Some",
            "state": "one appended row per subscription: nodes from \"<name>\" use the imported profile's DNS instead",
            "effect": "set",
            "evidence": "spec dns-preferences-ui scenario 'Imported profile named'; Subscription fields subscription.rs:8-20"
          },
          {
            "input": "no subscription with an enabled DNS-carrying profile",
            "state": "no profile row",
            "effect": "clear",
            "evidence": "same requirement, complementary case"
          },
          {
            "input": "xray TUN summary contains Bootstrap entries (no nodes exist in preferences)",
            "state": "consecutive Bootstrap entries collapse to one row: 'bootstrap resolvers for proxy hostnames (xray TUN)'",
            "effect": "forced",
            "evidence": "design 'Preferences placement'"
          }
        ],
        "forbidden": [
          "group hidden or insensitive while the DNS master toggle is off",
          "rows refreshed only on dialog reopen (must rebuild through the settings observers)",
          "per-host bootstrap rows on the preferences page",
          "formatting logic depending on GTK types (row text must come from the pure effective_dns_rows fn)",
          "changes to any other DNS page widget or handler"
        ],
        "seeding": [
          "pure-fn tests: AppSettings built from v2ray_rs_core defaults with the same mutations as the effective-dns-model seam, then effective_dns_rows(&effective_dns(..), &profile_names)",
          "subscription fixture Subscription::new_from_url(\"Provider\", \"https://example.com/sub\") + imported_profile = Some(ImportedProfile { dns: Some(..), .. }) + use_imported_profile = true (default-true pattern subscription.rs:182-187)",
          "live verification per task 3.1: xray + TUN + DNS off shows both fallback DoH entries; toggling DNS on replaces them without reopening Preferences"
        ],
        "budgets": [
          "row count bounded: entries minus collapsed bootstrap runs plus one row per DNS-carrying profile subscription; no per-host expansion",
          "exactly 1 group; rows rebuilt (replaced, not appended) on every observer fire — count stable across repeated fires"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (dns.rs inline tests, pure, no GTK): effective_dns_rows_lists_fallback_pair_when_dns_off; effective_dns_rows_follow_enabled_dns (unscoped user-server rows replace the fallback rows); effective_dns_rows_names_imported_profile_subscription (row contains \"Provider\"); effective_dns_rows_collapses_bootstrap_pair; effective_dns_rows_no_profile_row_without_profiles",
        "impl: pure fn effective_dns_rows(summary: &EffectiveDns, profile_subscriptions: &[String]) -> Vec<String> — per entry `{address} [{transport}] path={path} source={source} scope={scope}` (transport word via protocol_display_name, segment omitted when None; scope=all or comma-joined), consecutive Bootstrap entries collapsed to 'bootstrap resolvers for proxy hostnames (xray TUN)', plus one row per profile subscription: `nodes from \"<name>\" use the imported profile's DNS instead`",
        "impl: 'Effective DNS' adw::PreferencesGroup added in build_dns_page above the Advanced group, always sensitive (never bound to the master toggle), rebuilt through subscribe_settings(observers, ..) on DNS/TUN/backend change",
        "impl: thread routing rules and subscriptions into build_dns_page (signature change; single caller preferences/mod.rs:81; subscriptions via store.load_subscriptions() — workspace.rs:46 — captured at dialog open; routing via routing_state preferences/mod.rs:43-45)"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it."
    }
  ],
  "seams": [
    {
      "id": "effective-dns-model",
      "tasks": [
        "1.1",
        "1.2",
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it. Ships a pure classifier crates/core/src/config/effective_dns.rs: effective_dns(backend, settings, rules, node_hosts) -> EffectiveDns with ordered entries {address, transport, path, source, scope}, flags uses_fallback and system_resolver, EffectiveDns::mark_profile() for the imported-profile source, and EffectiveDns::log_lines() defining the dns record format. Zero generator changes; the cross-check matrix (task 1.2) pins the summary to the generators. Entry path (loader a test/live check reaches it through): ships as a new pub module v2ray_rs_core::config::effective_dns (declared in crates/core/src/config/mod.rs beside the existing mod declarations, mod.rs:1-10); consumed by the connection task (seam connection-summary-notice) and the DNS preferences page (seam preferences-effective-dns); tests reach it directly as crate::config::effective_dns::effective_dns inside the inline #[cfg(test)] mod of effective_dns.rs",
      "contract": {
        "states": [
          "entries: ordered Vec<EffectiveDnsEntry>",
          "entry.source in {user, fallback, bootstrap, system, profile}",
          "entry.path in {direct, proxy, routing, system, static}",
          "entry.scope in {All, Domains(Vec<String>)}",
          "uses_fallback flag (any entry source == Fallback)",
          "system_resolver flag (any entry path == System, incl. no-dns-section)",
          "profile-marked (User entries relabeled Profile by mark_profile)"
        ],
        "transitions": [
          {
            "input": "xray, tun.enabled, !dns.enabled",
            "state": "entries = [https://1.1.1.1/dns-query, https://8.8.8.8/dns-query] transport DoH, path=proxy, source=fallback, scope=all; configured servers absent; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec dns-configuration scenario 'xray TUN with DNS disabled'; v2ray.rs:631-638 fallback pair, :492-496 dns-internal routed to first proxy"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, node_hosts contains a hostname",
            "state": "bootstrap pair prepended: 1.1.1.1 UDP then https://1.1.1.1/dns-query DoH, path=direct, source=bootstrap, scope=Domains([hostname])",
            "effect": "set",
            "evidence": "spec scenario 'xray TUN bootstrap for a hostname node'; v2ray.rs:682-734 bootstrap_dns_servers, :485-491 dns-direct routing rule"
          },
          {
            "input": "xray, dns.enabled, every configured server domain-scoped (any TUN state)",
            "state": "configured entries keep Domains scope, then appended https://1.1.1.1/dns-query source=fallback scope=all; uses_fallback=true",
            "effect": "set",
            "evidence": "spec scenario 'xray with only domain-scoped servers'; v2ray.rs:118-130 apply_dns_fallback_policy applied at :651-653"
          },
          {
            "input": "xray or v2ray, !tun.enabled, !dns.enabled",
            "state": "single entry {address: \"system\", transport: None, path=system, source=system, scope=all}; system_resolver=true; uses_fallback=false",
            "effect": "forced",
            "evidence": "spec scenario 'No DNS section outside TUN'; v2ray.rs:50 dns section only when dns.enabled || tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, servers empty, !tun.enabled",
            "state": "entry {address: localhost, transport: None, path=system, source=system}; system_resolver=true",
            "effect": "forced",
            "evidence": "v2ray.rs:643-649 servers empty -> localhost unless tun_xray"
          },
          {
            "input": "v2ray/xray, dns.enabled, !tun.enabled, untagged server (detour != direct or backend v2ray)",
            "state": "user entry path=routing",
            "effect": "set",
            "evidence": "design.md path vocabulary 'routing'; v2ray.rs:535-544 IPIfNonMatch outside TUN, unmatched traffic takes first outbound"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, server.detour == \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "v2ray.rs:785-787 direct_detour honored only under TUN; :485-491"
          },
          {
            "input": "xray TUN, dns.enabled, DNS server addressed by hostname",
            "state": "server hostname joins the bootstrap Domains scope",
            "effect": "set",
            "evidence": "v2ray.rs:764-774 bootstrap_domains includes server addresses when dns.enabled"
          },
          {
            "input": "sing-box, tun.enabled, !dns.enabled",
            "state": "entry {address: https://1.1.1.1/dns-query, transport: DoH, path: proxy, source: fallback, scope: all}; uses_fallback=true",
            "effect": "forced",
            "evidence": "spec scenario 'sing-box TUN with DNS disabled'; singbox.rs:52-58, :71-98 derived_tun_dns with detour = first proxy"
          },
          {
            "input": "sing-box, !tun.enabled, !dns.enabled",
            "state": "single system entry (no dns section)",
            "effect": "forced",
            "evidence": "singbox.rs:50-58 dns emitted only when dns.enabled or tun.enabled"
          },
          {
            "input": "sing-box, dns.enabled, server.detour None or \"direct\"",
            "state": "user entry path=direct",
            "effect": "set",
            "evidence": "spec scenario 'sing-box server without detour'; singbox.rs:463-465 detour emitted only for non-direct values"
          },
          {
            "input": "sing-box, dns.enabled, server.detour set to non-direct",
            "state": "user entry path=proxy",
            "effect": "set",
            "evidence": "singbox.rs:463-465 detour = first_proxy_tag"
          },
          {
            "input": "sing-box, dns.enabled, no configured server is an IP literal and servers non-empty",
            "state": "appended entry {address: local, transport: None, path=system, source=system}",
            "effect": "set",
            "evidence": "singbox.rs:500-511 sys-dns-bootstrap local server; BOOTSTRAP_RESOLVER_TAG singbox.rs:160-170"
          },
          {
            "input": "settings.dns.hosts non-empty and a dns section exists for this backend state",
            "state": "static entries {address: domain, transport: None, path=static, source=user, scope=Domains([domain])}; excluded from log_lines and from the cross-check address set",
            "effect": "set",
            "evidence": "design.md path vocabulary 'static for hosts entries'; v2ray.rs hosts_for_strategy; singbox.rs:100-131 hosts_server/hosts_rule"
          },
          {
            "input": "caller invokes EffectiveDns::mark_profile() after resolve_effective_config applied a profile DNS",
            "state": "every User entry relabeled Profile; fallback/bootstrap/system entries unchanged",
            "effect": "forced",
            "evidence": "task 1.3; spec scenario 'Imported profile supplies DNS'; imported_profile.rs:18-45"
          },
          {
            "input": "xray/v2ray, dns.enabled, at least one unscoped user server",
            "state": "no appended fallback entry; uses_fallback=false",
            "effect": "clear",
            "evidence": "v2ray.rs:118-130 fallback appended only when no unrestricted server remains"
          },
          {
            "input": "v2ray backend, user server protocol Dot/Doq/H3",
            "state": "entry transport/address follow effective_for_backend downgrade to DoH",
            "effect": "forced",
            "evidence": "models/dns.rs DnsProtocol::fallback_protocol_for_backend / effective_for_backend; v2ray.rs dns_server_address_for_backend"
          },
          {
            "input": "xray, tun.enabled, dns.enabled, untagged server (detour != direct)",
            "state": "user entry path=proxy (queries carry dns.tag=dns-internal routed to the first proxy outbound)",
            "effect": "set",
            "evidence": "v2ray.rs:492-496 dns-internal routing"
          }
        ],
        "forbidden": [
          "any change to generator output: no edit to the resolver-emitting logic of crates/core/src/config/{v2ray.rs, singbox.rs, xray.rs, common.rs, writer.rs} — the cross-check (task 1.2) and the existing generator tests pin it",
          "uses_fallback == true when DNS is enabled and an unscoped user server exists (xray/v2ray)",
          "configured user servers appearing in the summary when dns.enabled == false under xray TUN (spec scenario 1 forbids it)",
          "effective_dns performing IO: no generated-JSON parsing, no file reads (design.md rejected alternative)",
          "mark_profile relabeling fallback, bootstrap, or system entries",
          "summary non-static address set diverging from the generated dns.servers addresses for any matrix cell (bidirectional set equality)"
        ],
        "seeding": [
          "default_settings() from crates/core/src/config/test_fixtures.rs; node fixtures vless_node() (hostname example.com); an IP-address node by overriding address on a fixture (mutation pattern: test_v2ray_verify_off_emits_allow_insecure in v2ray.rs tests)",
          "settings.tun.enabled / settings.dns.enabled / settings.dns.servers / settings.dns.rules / settings.dns.use_custom_rules mutated directly on the fixture",
          "scoped server: DnsServerConfig { tag, protocol, address, port, detour } (models/dns.rs:99-105) + DnsRule { match_condition: DnsRuleMatch::DomainSuffix{..}, server_tag } with use_custom_rules=true; auto-split scope via server tag \"remote\"/\"domestic\" (AUTO_SPLIT_REMOTE_TAG/AUTO_SPLIT_DOMESTIC_TAG, models/dns.rs:6-8) plus routing rules",
          "generated config for the cross-check: generate_v2ray_family_config(&nodes, &rules, &settings, V2rayFamilyBackend::{Xray,V2ray}) (pub(crate), v2ray.rs:86-104; the #[cfg(test)] build_dns wrapper pattern exists at v2ray.rs:623-627) and SingboxGenerator::generate (config/mod.rs:88-95)",
          "profile fixture: Subscription::new_from_url(\"Provider\", \"https://example.com/sub\") + imported_profile = Some(ImportedProfile { dns: Some(..), .. }) + use_imported_profile = true (subscription.rs:8-20, :182-187), resolved through resolve_effective_config (imported_profile.rs:18-45)"
        ],
        "budgets": [
          "cross-check matrix: 3 backends x 2 TUN x 2 dns.enabled x 2 IP/hostname node, dns-enabled cells split scoped/unscoped (disabled cells ignore servers) = 48 cases, pure, zero network/IO, inside the 5m core test budget",
          "entry count bounded by servers.len() + 2 (bootstrap pair) + 1 (appended fallback) + 1 (system/local) + hosts.len() static entries",
          "0 diffs to generator files (forbidden list; observed by the same cargo test -p v2ray-rs-core run keeping existing generator tests green)"
        ]
      },
      "codeTasks": [
        "tests first (inline #[cfg(test)] mod in effective_dns.rs), one per dns-configuration scenario quoting entry fields verbatim: xray_tun_dns_disabled_lists_fallback_pair_not_configured; xray_all_servers_scoped_appends_unscoped_fallback; xray_tun_hostname_bootstrap_pair_direct_scoped; singbox_tun_dns_disabled_derived_doh_fallback; singbox_server_without_detour_is_direct; no_dns_section_outside_tun_single_system_entry (v2ray + xray + sing-box); imported_profile_dns_marked_profile (task 1.3, ImportedProfile { dns: Some(..) }); uses_fallback_false_with_unscoped_user_server; hosts_overrides_are_static_entries; log_lines_format_is_server_path_source_scope",
        "impl: pub struct EffectiveDns { entries: Vec<EffectiveDnsEntry>, uses_fallback: bool, system_resolver: bool }; pub struct EffectiveDnsEntry { address: String, transport: Option<DnsProtocol>, path: DnsPath, source: DnsSource, scope: DnsScope }; pub enum DnsPath { Direct, Proxy, Routing, System, Static }; pub enum DnsSource { User, Fallback, Bootstrap, System, Profile }; pub enum DnsScope { All, Domains(Vec<String>) } (Debug, Clone, PartialEq, Eq)",
        "impl: pub fn effective_dns(backend: BackendType, settings: &AppSettings, rules: &[RoutingRule], node_hosts: &[&str]) -> EffectiveDns",
        "impl: EffectiveDns::mark_profile(&mut self) — relabels User -> Profile only (task 1.3 caller flag)",
        "impl: EffectiveDns::log_lines(&self) -> Vec<String> — one line per non-static entry, exact format `server={address} path={path} source={source} scope={scope}` with lowercase vocabulary direct|proxy|routing|system and user|fallback|bootstrap|system|profile; scope=all or comma-joined domains",
        "impl: crates/core/src/config/mod.rs — add `pub mod effective_dns;`",
        "cross-check tests (task 1.2): effective_dns_addresses_match_generated_servers over the 48-case matrix (3 backends x 2 TUN x 2 dns.enabled x 2 node kind, dns-enabled cells split scoped/unscoped) — set equality between non-static summary addresses and normalized generated dns.servers addresses; system_resolver true iff no dns section or a localhost/local server; hosts cell: dns.hosts keys equal static-entry scopes"
      ]
    },
    {
      "id": "session-extra-dns-records",
      "tasks": [
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it. Ships ProcessManager::with_session_extra(Vec<String>): each string written as a `dns` stream line immediately after the `session` record inside launch, so every start and every crash respawn repeats the block next to its own session line. Session record format itself untouched. Entry path (loader a test/live check reaches it through): ships inside ProcessManager::launch -> write_session_record (crates/process/src/manager.rs:682-713 and :696-708), which both start_with_connection and the crash respawn reach; a test loads it through manager_for(...).with_log_file(...).with_session_extra(...).start(), and the respawn path through wait_and_handle_exit (manager.rs:578-603)",
      "contract": {
        "states": [
          "manager.session_extra: Vec<String> (default empty)",
          "launch log tail: session line followed by 0..N dns lines, before any backend output",
          "crash respawn: block repeats after the new session line"
        ],
        "transitions": [
          {
            "input": "with_session_extra(lines) on the builder",
            "state": "session_extra stored on the manager",
            "effect": "set",
            "evidence": "builder pattern beside with_session_fields, manager.rs:275"
          },
          {
            "input": "no with_session_extra call",
            "state": "session_extra empty, session record identical to today",
            "effect": "no-op",
            "evidence": "default init in ProcessManager::new, manager.rs:183-214"
          },
          {
            "input": "launch() with N extras and a log file attached",
            "state": "session line then N dns lines (`<rfc3339> dns <line>`), written before try_spawn so no backend output can interleave",
            "effect": "set",
            "evidence": "write_session_record is the first call in launch, manager.rs:682-713; write_stream_line manager.rs:1134; RotatingFileWriter::append_line format rotating_log.rs:91-98"
          },
          {
            "input": "crash respawn (auto_restart path)",
            "state": "new session line followed by the same N dns lines",
            "effect": "forced",
            "evidence": "respawn -> launch, manager.rs:594-603; existing respawn ordering test manager.rs:1865-1877"
          },
          {
            "input": "preflight failure (binary/config missing, version/capability/helper gate)",
            "state": "no session line and no dns lines",
            "effect": "no-op",
            "evidence": "launch never reached; existing tests manager.rs:2043-2082 assert no session record"
          },
          {
            "input": "no log file attached (with_log_file(None))",
            "state": "nothing written",
            "effect": "no-op",
            "evidence": "write_stream_line no-ops on None writer, manager.rs:1134"
          },
          {
            "input": "an extra string containing \\n or \\r",
            "state": "flattened onto one physical line, cannot forge a second record",
            "effect": "forced",
            "evidence": "line-forging guard pattern: session_fields tests manager.rs:1531-1549 and :1551-1590"
          }
        ],
        "forbidden": [
          "dns lines anywhere but immediately after the session line (never before it, never after backend output)",
          "an extra line forging additional physical lines (newline injection)",
          "any change to the session record format (session backend=... fields, manager.rs:703-712, pinned by tests manager.rs:1509-1549)",
          "dns lines on a preflight-failed start",
          "any change to ProcessManager lifecycle behavior other than the appended lines (state machine, respawn budget untouched)"
        ],
        "seeding": [
          "manager_for(&dir, &format!(\"{VERSION_STUB}exec sleep 30\\n\")) + .with_log_file(Some(backend_log(dir.path()))) + .with_session_extra(vec![..]) — exact pattern of session_record_appends_caller_fields, manager.rs:1509-1529",
          "assertions via read_lines / wait_for_lines on dir.path().join(\"backend.log\") (manager.rs tests helpers)",
          "crash respawn seeding: stub that fails the first start then sleeps, driven through wait_and_handle_exit + auto respawn — pattern of the session/exit/session ordering test manager.rs:1865-1877"
        ],
        "budgets": [
          "dns lines per launch == session_extra.len() exactly (test pins 2 after session on start, and again 2 after the respawn session: 4 dns lines across 2 launches)",
          "exactly 1 session line per launch, unchanged"
        ]
      },
      "codeTasks": [
        "tests first (manager.rs inline tests): session_extra_dns_lines_follow_session_on_start (two dns lines directly after the session line); session_extra_dns_lines_repeat_after_crash_respawn (session, dns, dns, exit, session, dns, dns); session_extra_flattens_newlines; no_session_extra_writes_no_dns_lines",
        "impl: ProcessManager field `session_extra: Vec<String>` (Vec::new() in new()); `pub fn with_session_extra(mut self, lines: Vec<String>) -> Self` beside with_session_fields (manager.rs:275)",
        "impl: in write_session_record, after write_stream_line(&self.log_writer, \"session\", &record): `for line in &self.session_extra { write_stream_line(&self.log_writer, \"dns\", line) routed through the same truncate_reason sanitizer session_fields already flows through (manager.rs write_session_record) — no new helper; }`"
      ]
    },
    {
      "id": "connection-summary-notice",
      "tasks": [
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it. Ships the connection-task half: per candidate, EffectiveDns computed from effective_settings (after resolve_effective_config and apply_pins), mark_profile applied for imported-profile candidates, summary.log_lines() passed via .with_session_extra(..), and the fallback notice emitted exactly once per connection through the process log stream (backend.log 'notice' stream + AppMsg::ProcessLogLine, no toast). Entry path (loader a test/live check reaches it through): ships inside the connection task spawned by spawn_with (crates/ui/src/connection.rs:99-460), per candidate after apply_pins and before ProcessManager construction (:347-358); a test loads it through the real task via connect_with(&stub, settings, candidates, configure) (connection.rs:1291) and observes AppMsg::ProcessLogLine plus stub.paths.logs_dir().join(\"backend.log\")",
      "contract": {
        "states": [
          "per-candidate dns summary lines handed to the manager (source=profile when the profile supplied DNS)",
          "dns_notice_sent flag per connection (unsent -> sent)",
          "notice text selected by case (xray disabled / sing-box disabled / xray scoped-only)"
        ],
        "transitions": [
          {
            "input": "candidate iteration with effective_settings (post resolve_effective_config + apply_pins)",
            "state": "summary = effective_dns(backend, &effective_settings, &effective_rules, node_hosts of this candidate's nodes); lines = summary.log_lines() passed via .with_session_extra(lines)",
            "effect": "set",
            "evidence": "candidate loop crates/ui/src/connection.rs:236-271 (resolve_effective_config, pins) and builder chain :347-358; spec diagnostic-logs 'computed from the settings actually used for that launch'"
          },
          {
            "input": "candidate whose subscription profile replaced DNS (uses_imported_profile)",
            "state": "summary.mark_profile() before log_lines; dns line carries source=profile",
            "effect": "forced",
            "evidence": "imported_profile.rs:18-45; session_fields already stamps profile=imported, connection.rs:213"
          },
          {
            "input": "candidate summary uses_fallback && !dns_notice_sent",
            "state": "notice emitted once: backend_log append_line(\"notice\", text) + sender.emit(AppMsg::ProcessLogLine(generation, text)); flag set",
            "effect": "set",
            "evidence": "design 'Notice once per connection'; emission pattern STRICT_ROUTE_NOTICE connection.rs:150-173; flag pattern capture_notice_sent connection.rs:213"
          },
          {
            "input": "later candidate with uses_fallback while flag already set (failover)",
            "state": "no second notice",
            "effect": "no-op",
            "evidence": "spec diagnostic-logs scenario 'Fallback notice once per connection'"
          },
          {
            "input": "candidate summary with uses_fallback == false (DNS enabled, unscoped user server)",
            "state": "no notice at any point, flag stays unsent",
            "effect": "clear",
            "evidence": "spec diagnostic-logs scenario 'No notice with user resolvers'"
          },
          {
            "input": "notice case selection: (!dns.enabled && tun && xray) | (!dns.enabled && tun && sing-box) | (dns.enabled && fallback-from-scoped)",
            "state": "FALLBACK_DNS_DISABLED_NOTICE_XRAY | FALLBACK_DNS_DISABLED_NOTICE_SINGBOX | FALLBACK_DNS_SCOPED_NOTICE",
            "effect": "set",
            "evidence": "design notice text verbatim; pure selector fallback_dns_notice"
          },
          {
            "input": "writer.write_config failed for a candidate",
            "state": "no manager built, no dns lines, loop continues to next candidate",
            "effect": "no-op",
            "evidence": "connection.rs:296-313 error path"
          }
        ],
        "forbidden": [
          "notice emitted more than once per connection (any candidate count)",
          "notice when uses_fallback == false (DNS enabled with an unscoped server)",
          "notice delivered as a toast (AppMsg::ShowToast) — design decision: no toast",
          "dns lines computed from the global settings instead of effective_settings (imported profile or connect-time pins would be lost)",
          "dns lines written for a candidate that never launched (config-generation failure path)",
          "any change to pinning behavior, candidate ordering, or failure handling (harden-node-pinning is a different change)"
        ],
        "seeding": [
          "connect(&stub, settings, candidates) / connect_with(&stub, settings, candidates, configure) harness — connection.rs:1238-1308; stub(script) :1205; candidate(address) :1226; ready_singbox_settings() :1310",
          "xray TUN DNS-off settings: settings.backend.backend_type = BackendType::Xray; settings.tun.enabled = true; settings.dns.enabled = false (backend-mutation pattern of xray_dns_burst_reports_dns_health, connection.rs:2887-2895)",
          "two failing candidates: stub script exiting non-zero for the first candidate's config (pattern two_candidates_first_exits_before_ready, connection.rs:2551-2575); messages drained via the AppMsg collectors at connection.rs:1719-1723 and :2303-2307",
          "profile candidate: ConnectionCandidate with node_ref ConnectionNodeRef::Subscription { .. } and the subscription fixture (imported_profile dns Some) inside the request's subscriptions"
        ],
        "budgets": [
          "notice count per connection <= 1 (asserted == 1 across two failed candidates)",
          "dns lines per backend.log session block == summary.entries.len() minus static entries (fallback-pair fixture: 2)",
          "0 toasts for the fallback notice"
        ],
        "dispatch_order": "CH-4 dispatches only after BOTH CH-1-core-effective-dns-model (prev) and CH-3-process-session-extra are closed: .with_session_extra exists only after CH-3 lands; compiling crates/ui against v2ray-rs-process requires it",
        "notice_texts": {
          "FALLBACK_DNS_DISABLED_NOTICE_XRAY": "DNS is disabled; TUN resolves through fallback DoH 1.1.1.1/8.8.8.8 via the proxy. Enable DNS in Preferences to use your servers",
          "FALLBACK_DNS_DISABLED_NOTICE_SINGBOX": "DNS is disabled; TUN resolves through the fallback DoH 1.1.1.1 via the proxy. Enable DNS in Preferences to use your servers",
          "FALLBACK_DNS_SCOPED_NOTICE": "Every configured DNS server is domain-scoped; the fallback 1.1.1.1 answers all other domains. Add an unscoped server in Preferences to change that"
        }
      },
      "codeTasks": [
        "tests first (connection.rs inline tests): fallback_notice_once_across_two_failed_candidates (exactly one ProcessLogLine with the notice prefix across two candidates); no_notice_with_dns_enabled_unscoped_server (zero notice lines); dns_lines_follow_session_in_backend_log (two ` dns server=` lines directly after the ` session ` line); dns_lines_carry_source_profile_for_imported_profile_candidate; fallback_dns_notice unit tests quoting the three consts verbatim",
        "impl: pure fn fallback_dns_notice(summary: &EffectiveDns, dns_enabled: bool, tun: bool, backend: BackendType) -> Option<&'static str> beside STRICT_ROUTE_NOTICE, with the exact texts pinned in contract.notice_texts (tests quote them verbatim)",
        "impl: candidate loop computes EffectiveDns from effective_settings (+ mark_profile when uses_imported_profile), passes .with_session_extra(summary.log_lines()) in the builder chain, emits the notice gated by a dns_notice_sent flag mirroring capture_notice_sent (connection.rs:213), through backend_log append_line(\"notice\", ..) + AppMsg::ProcessLogLine only"
      ]
    },
    {
      "id": "preferences-effective-dns",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no test-writer stage; the chunk ships tests and code in one rust-coder dispatch. NO-TESTER-WAIVER: no tester stage; the chunk literal cargo command plus the workspace floor verify it. Ships the read-only 'Effective DNS' group on the DNS page: rows from a pure formatter effective_dns_rows(summary, profile_subscriptions) (headless-unit-testable like primary_dns_subtitle), group always sensitive even with the master toggle off, rebuilt through the settings observers on DNS/TUN/backend change, bootstrap entries collapsed to one line, imported-profile subscriptions named. Entry path (loader a test/live check reaches it through): ships through build_dns_page (crates/ui/src/preferences/dns.rs) loaded by show_preferences (crates/ui/src/preferences/mod.rs:81); headless tests reach the behavior through the pure formatter effective_dns_rows in dns.rs (pattern: primary_dns_subtitle unit tests in the same file); the GTK group, its sensitivity, and the subscribe_settings observer wiring (mod.rs:108-113) are exercised live per task 3.1",
      "contract": {
        "states": [
          "summary rows rendered from (settings, rules, subscriptions) for the current backend/TUN state",
          "group visible and sensitive regardless of dns master toggle",
          "profile subscription rows present/absent"
        ],
        "transitions": [
          {
            "input": "DNS page built (dialog opened)",
            "state": "'Effective DNS' group above Advanced, populated via effective_dns(backend, settings, rules, &[])",
            "effect": "set",
            "evidence": "design 'Preferences placement'; build_dns_page crates/ui/src/preferences/dns.rs, loaded by show_preferences preferences/mod.rs:81"
          },
          {
            "input": "settings observer fires (DNS toggle, TUN toggle, backend change)",
            "state": "rows rebuilt from the new settings snapshot — no dialog reopen",
            "effect": "set",
            "evidence": "subscribe_settings observers preferences/mod.rs:108-113 fanned out from settings_cb :49-58; spec dns-preferences-ui scenario 'Summary follows edits'"
          },
          {
            "input": "dns.enabled == false under xray TUN",
            "state": "both fallback DoH rows shown (path=proxy, source=fallback); group stays sensitive",
            "effect": "forced",
            "evidence": "spec dns-preferences-ui scenario 'Fallback shown while DNS is off'; design 'always visible and sensitive even when the master toggle is off'"
          },
          {
            "input": "dns.enabled toggled true",
            "state": "fallback rows replaced by configured-server rows in place",
            "effect": "set",
            "evidence": "spec dns-preferences-ui scenario 'Summary follows edits'"
          },
          {
            "input": "subscription with use_imported_profile && imported_profile.dns == Some",
            "state": "one appended row per subscription: nodes from \"<name>\" use the imported profile's DNS instead",
            "effect": "set",
            "evidence": "spec dns-preferences-ui scenario 'Imported profile named'; Subscription fields subscription.rs:8-20"
          },
          {
            "input": "no subscription with an enabled DNS-carrying profile",
            "state": "no profile row",
            "effect": "clear",
            "evidence": "same requirement, complementary case"
          },
          {
            "input": "xray TUN summary contains Bootstrap entries (no nodes exist in preferences)",
            "state": "consecutive Bootstrap entries collapse to one row: 'bootstrap resolvers for proxy hostnames (xray TUN)'",
            "effect": "forced",
            "evidence": "design 'Preferences placement'"
          }
        ],
        "forbidden": [
          "group hidden or insensitive while the DNS master toggle is off",
          "rows refreshed only on dialog reopen (must rebuild through the settings observers)",
          "per-host bootstrap rows on the preferences page",
          "formatting logic depending on GTK types (row text must come from the pure effective_dns_rows fn)",
          "changes to any other DNS page widget or handler"
        ],
        "seeding": [
          "pure-fn tests: AppSettings built from v2ray_rs_core defaults with the same mutations as the effective-dns-model seam, then effective_dns_rows(&effective_dns(..), &profile_names)",
          "subscription fixture Subscription::new_from_url(\"Provider\", \"https://example.com/sub\") + imported_profile = Some(ImportedProfile { dns: Some(..), .. }) + use_imported_profile = true (default-true pattern subscription.rs:182-187)",
          "live verification per task 3.1: xray + TUN + DNS off shows both fallback DoH entries; toggling DNS on replaces them without reopening Preferences"
        ],
        "budgets": [
          "row count bounded: entries minus collapsed bootstrap runs plus one row per DNS-carrying profile subscription; no per-host expansion",
          "exactly 1 group; rows rebuilt (replaced, not appended) on every observer fire — count stable across repeated fires"
        ]
      },
      "codeTasks": [
        "tests first (dns.rs inline tests, pure, no GTK): effective_dns_rows_lists_fallback_pair_when_dns_off; effective_dns_rows_follow_enabled_dns (unscoped user-server rows replace the fallback rows); effective_dns_rows_names_imported_profile_subscription (row contains \"Provider\"); effective_dns_rows_collapses_bootstrap_pair; effective_dns_rows_no_profile_row_without_profiles",
        "impl: pure fn effective_dns_rows(summary: &EffectiveDns, profile_subscriptions: &[String]) -> Vec<String> — per entry `{address} [{transport}] path={path} source={source} scope={scope}` (transport word via protocol_display_name, segment omitted when None; scope=all or comma-joined), consecutive Bootstrap entries collapsed to 'bootstrap resolvers for proxy hostnames (xray TUN)', plus one row per profile subscription: `nodes from \"<name>\" use the imported profile's DNS instead`",
        "impl: 'Effective DNS' adw::PreferencesGroup added in build_dns_page above the Advanced group, always sensitive (never bound to the master toggle), rebuilt through subscribe_settings(observers, ..) on DNS/TUN/backend change",
        "impl: thread routing rules and subscriptions into build_dns_page (signature change; single caller preferences/mod.rs:81; subscriptions via store.load_subscriptions() — workspace.rs:46 — captured at dialog open; routing via routing_state preferences/mod.rs:43-45)"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Effective DNS resolver set is derivable The system SHALL derive, for a backend, the DNS settings, the TUN settings, and the routing rules used to generate a config, the ordered list of resolvers the generated config uses. Each entry SHALL state the resolver address, its transport, its path (`direct`, `proxy`, `routing` when the backend's routing rules decide, `system` for the operating-system resolver, or `static` for host overrides), its source (`user`, `fallback`, `bootstrap`, `system`, or `profile` when a subscription's imported profile supplied the DNS settings), and its scope (all domains or the listed domains). The list SHALL match the resolvers present in the generated config for the same inputs. Deriving it SHALL NOT change which resolvers are generated.",
      "label": "dns-configuration/derive-ordered-list+dns-configuration/entry-fields+dns-configuration/matches-generator+dns-configuration/no-generator-change",
      "tests": [
        "crates/core/src/config/effective_dns.rs (new): effective_dns scenario tests assert ordering per matrix cell, e.g. xray_tun_dns_disabled_lists_fallback_pair_not_configured, singbox_tun_dns_disabled_derived_doh_fallback (task 1.1)",
        "every task-1.1 scenario test asserts EffectiveDnsEntry { address, transport, path, source, scope } verbatim; log_lines_format_is_server_path_source_scope pins the serialized form",
        "effective_dns_addresses_match_generated_servers cross-check matrix over 3 backends x TUN x dns.enabled x scoped/unscoped x IP/hostname (task 1.2, CH-2)",
        "forbidden list of the effective-dns-model seam (0 diffs to config/{v2ray,singbox,xray,common,writer}.rs) enforced by review; observed by the existing generator tests staying green in `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` plus the bidirectional cross-check (any generator or summary drift fails the matrix)"
      ]
    },
    {
      "shall": "#### Scenario: xray TUN with DNS disabled - **WHEN** the backend is xray, TUN is enabled, and DNS is disabled - **THEN** the list SHALL contain `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` with path `proxy` and source `fallback`, and SHALL NOT contain the configured DNS servers",
      "label": "dns-configuration/scenario-xray-tun-disabled",
      "tests": [
        "xray_tun_dns_disabled_lists_fallback_pair_not_configured (1.1)"
      ]
    },
    {
      "shall": "#### Scenario: xray with only domain-scoped servers - **WHEN** the backend is xray, DNS is enabled, and every configured server is scoped to domains - **THEN** the list SHALL contain the configured servers with their domain scope followed by `https://1.1.1.1/dns-query` with source `fallback` and scope all domains",
      "label": "dns-configuration/scenario-xray-scoped-only",
      "tests": [
        "xray_all_servers_scoped_appends_unscoped_fallback (1.1); FALLBACK_DNS_SCOPED_NOTICE unit test (2.2)"
      ]
    },
    {
      "shall": "#### Scenario: xray TUN bootstrap for a hostname node - **WHEN** the backend is xray, TUN is enabled, and the node address is a hostname - **THEN** the list SHALL start with UDP `1.1.1.1` and DoH `1.1.1.1` with path `direct`, source `bootstrap`, and scope that hostname",
      "label": "dns-configuration/scenario-xray-bootstrap",
      "tests": [
        "xray_tun_hostname_bootstrap_pair_direct_scoped (1.1); dns_lines_follow_session_in_backend_log exercises the hostname path end to end (2.2)"
      ]
    },
    {
      "shall": "#### Scenario: sing-box TUN with DNS disabled - **WHEN** the backend is sing-box, TUN is enabled, and DNS is disabled - **THEN** the list SHALL contain DoH `1.1.1.1` with path `proxy` and source `fallback`",
      "label": "dns-configuration/scenario-singbox-tun-disabled",
      "tests": [
        "singbox_tun_dns_disabled_derived_doh_fallback (1.1)"
      ]
    },
    {
      "shall": "#### Scenario: sing-box server without detour - **WHEN** the backend is sing-box, DNS is enabled, and a configured server has no detour - **THEN** that entry SHALL have path `direct`",
      "label": "dns-configuration/scenario-singbox-no-detour",
      "tests": [
        "singbox_server_without_detour_is_direct (1.1)"
      ]
    },
    {
      "shall": "#### Scenario: No DNS section outside TUN - **WHEN** the backend is xray or v2ray, TUN is off, DNS is disabled, and routing rules are enabled - **THEN** the list SHALL contain one entry with path `system` and source `system`",
      "label": "dns-configuration/scenario-system-outside-tun",
      "tests": [
        "no_dns_section_outside_tun_single_system_entry for v2ray, xray, sing-box (1.1)"
      ]
    },
    {
      "shall": "#### Scenario: Imported profile supplies DNS - **WHEN** the inputs come from a subscription node whose enabled imported profile carries DNS settings - **THEN** entries built from those settings SHALL have source `profile`",
      "label": "dns-configuration/scenario-imported-profile",
      "tests": [
        "imported_profile_dns_marked_profile (1.3); dns_lines_carry_source_profile_for_imported_profile_candidate (2.2)"
      ]
    },
    {
      "shall": "### Requirement: Effective DNS summary The DNS page SHALL show a read-only \"Effective DNS\" summary of the resolvers a connection with the current settings, selected backend, and TUN state would use, listing each resolver's address, transport, path, source, and scope. The summary SHALL stay visible and readable while the DNS master toggle is off. It SHALL update when DNS, TUN, or backend settings change. When one or more subscriptions have an enabled imported profile with DNS settings, the summary SHALL name those subscriptions and state that their nodes use the profile's DNS instead.",
      "label": "dns-preferences-ui/summary-shown+dns-preferences-ui/visible-when-off+dns-preferences-ui/follows-edits+dns-preferences-ui/profile-named",
      "tests": [
        "effective_dns_rows formatter tests (3.1): effective_dns_rows_lists_fallback_pair_when_dns_off etc. assert each field appears in the row text; live check per task 3.1 covers the rendered group",
        "content half: effective_dns_rows_lists_fallback_pair_when_dns_off (dns-disabled fixture yields the fallback rows); sensitivity half: no headless GTK test exists in this crate — verified live per task 3.1 (group never bound to the toggle) and recorded in the run report",
        "pure half: effective_dns_rows_follow_enabled_dns recomputes rows from the new settings; wiring half (subscribe_settings observer fire) verified live per task 3.1 — no headless GTK harness exists; named, not silent",
        "effective_dns_rows_names_imported_profile_subscription asserts the row contains \"Provider\" and the profile wording; effective_dns_rows_no_profile_row_without_profiles asserts absence (3.1)"
      ]
    },
    {
      "shall": "#### Scenario: Fallback shown while DNS is off - **WHEN** the backend is xray, TUN is enabled, DNS is disabled, and the user opens the DNS page - **THEN** the summary SHALL list `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` as fallback resolvers reached through the proxy",
      "label": "dns-preferences-ui/visible-when-off",
      "tests": [
        "content half: effective_dns_rows_lists_fallback_pair_when_dns_off (dns-disabled fixture yields the fallback rows); sensitivity half: no headless GTK test exists in this crate — verified live per task 3.1 (group never bound to the toggle) and recorded in the run report"
      ]
    },
    {
      "shall": "#### Scenario: Summary follows edits - **WHEN** the user enables DNS on that page - **THEN** the summary SHALL replace the fallback entries with the configured servers without reopening Preferences",
      "label": "dns-preferences-ui/follows-edits",
      "tests": [
        "pure half: effective_dns_rows_follow_enabled_dns recomputes rows from the new settings; wiring half (subscribe_settings observer fire) verified live per task 3.1 — no headless GTK harness exists; named, not silent"
      ]
    },
    {
      "shall": "#### Scenario: Imported profile named - **WHEN** a subscription named \"Provider\" has an enabled imported profile with DNS settings - **THEN** the summary SHALL state that nodes from \"Provider\" use that profile's DNS",
      "label": "dns-preferences-ui/profile-named",
      "tests": [
        "effective_dns_rows_names_imported_profile_subscription asserts the row contains \"Provider\" and the profile wording; effective_dns_rows_no_profile_row_without_profiles asserts absence (3.1)"
      ]
    },
    {
      "shall": "### Requirement: Effective DNS is recorded per launch Each backend launch, including a crash respawn, SHALL be followed in the backend log file, directly after its session record, by one record per effective resolver stating its address, path, source, and scope, computed from the settings actually used for that launch, including an imported profile and connect-time host overrides. When the connection's resolvers include fallback resolvers, the system SHALL write one notice line per connection to the process log stream stating that fallback resolvers are in use and which setting changes that.",
      "label": "diagnostic-logs/records-after-session+diagnostic-logs/computed-from-effective-settings+diagnostic-logs/fallback-notice",
      "tests": [
        "session_extra_dns_lines_follow_session_on_start; session_extra_dns_lines_repeat_after_crash_respawn (2.1); dns_lines_follow_session_in_backend_log end to end (2.2); live check 4.2",
        "dns_lines_carry_source_profile_for_imported_profile_candidate (2.2); bootstrap scope with pinned/host names covered by xray_tun_hostname_bootstrap_pair_direct_scoped over effective_settings post apply_pins (1.1 + 2.2 seeding); notice/lines computed after resolve_effective_config + apply_pins in the candidate loop (connection.rs:236-271)",
        "fallback_notice_once_across_two_failed_candidates; no_notice_with_dns_enabled_unscoped_server; fallback_dns_notice unit tests quoting the texts (2.2)"
      ]
    },
    {
      "shall": "#### Scenario: Records follow the session record - **WHEN** an xray TUN connection starts with DNS disabled - **THEN** `backend.log` SHALL contain, after the `session` record, records naming `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` with path `proxy` and source `fallback`",
      "label": "diagnostic-logs/scenario-records-follow-session",
      "tests": [
        "session_extra_dns_lines_follow_session_on_start (2.1) + dns_lines_follow_session_in_backend_log (2.2) + live 4.2"
      ]
    },
    {
      "shall": "#### Scenario: Fallback notice once per connection - **WHEN** such a connection fails over across two candidates - **THEN** the fallback notice SHALL appear exactly once in the process log stream",
      "label": "diagnostic-logs/scenario-notice-once",
      "tests": [
        "fallback_notice_once_across_two_failed_candidates (2.2)"
      ]
    },
    {
      "shall": "#### Scenario: No notice with user resolvers - **WHEN** DNS is enabled with at least one unscoped server - **THEN** no fallback notice SHALL be written",
      "label": "diagnostic-logs/scenario-no-notice",
      "tests": [
        "no_notice_with_dns_enabled_unscoped_server (2.2) + uses_fallback_false_with_unscoped_user_server (1.1)"
      ]
    }
  ],
  "testHarness": [
    "fixtures::default_settings — crates/core/src/config/test_fixtures.rs:5 — builds base AppSettings with default values for generator tests",
    "fixtures::vless_node — crates/core/src/config/test_fixtures.rs:9 — builds test VLESS proxy node with WS and TLS",
    "fixtures::vmess_node — crates/core/src/config/test_fixtures.rs:32 — builds test VMess proxy node with TCP",
    "fixtures::ss_node — crates/core/src/config/test_fixtures.rs:45 — builds test Shadowsocks proxy node with AES-256-GCM",
    "fixtures::trojan_node — crates/core/src/config/test_fixtures.rs:55 — builds test Trojan proxy node with TCP and TLS",
    "fixtures::xhttp_node — crates/core/src/config/test_fixtures.rs:68 — builds test VLESS proxy node with XHTTP and Reality",
    "build_dns — crates/core/src/config/v2ray.rs:667 — builds V2Ray family DNS config JSON from rules and settings",
    "proxy_rule — crates/core/src/config/v2ray.rs:1998 — builds a proxy routing rule with domain and IP match conditions",
    "direct_rule — crates/core/src/config/v2ray.rs:2008 — builds a direct routing rule with domain match condition",
    "block_rule — crates/core/src/config/v2ray.rs:2018 — builds a block routing rule with domain match condition",
    "outbound_tag — crates/core/src/config/common.rs:6 — formats outbound tag string for a proxy node and index",
    "SingboxGenerator::generate — crates/core/src/config/singbox.rs:27 — generates sing-box config JSON structure",
    "rule_with_domain — crates/core/src/config/singbox.rs:1072 — builds a sing-box test routing rule targeting an outbound",
    "manager_for — crates/process/src/manager.rs:1177 — constructs a ProcessManager backed by a stub shell script in a temp directory",
    "write_script — crates/process/src/manager.rs:1169 — writes an executable shell script acting as stub CLI binary",
    "backend_log — crates/process/src/manager.rs:1187 — creates Arc<RotatingFileWriter> pointing to a test backend.log",
    "read_lines — crates/process/src/manager.rs:1800 — reads all lines from a test backend log file",
    "wait_for_lines — crates/process/src/manager.rs:1152 — asynchronously polls a test log file until expected line count is reached",
    "request — crates/ui/src/connection.rs:1246 — constructs a ConnectionRequest populated with stub binary paths and settings",
    "candidate — crates/ui/src/connection.rs:1226 — creates a ConnectionCandidate with a given host address",
    "connect_with — crates/ui/src/connection.rs:1291 — runs a connection lifecycle task against a stub backend with custom manager configuration",
    "primary_row_missing_note — crates/ui/src/preferences/dns.rs:1128 — computes helper text for missing server tags in DNS preferences"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4",
  "estimateHours": 2.4,
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 5
  }
}
```
