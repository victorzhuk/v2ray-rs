## Context

See proposal.md for motivation. Domain patterns and TUN excluded domains represent a name plus all subdomains, but generators currently preserve a leading `*.` and xray exclusions omit their matcher prefix.

## Goals / Non-Goals

**Goals:**
- Generate equivalent suffix matching semantics on every supported backend.
- Keep stored values unchanged.

**Non-Goals:**
- Add an apex-excluding matcher.
- Change keyword or full-domain matcher semantics.
- Validate or migrate persisted values.

## Decisions

- Strip exactly one leading `*.` at emission through a shared core configuration helper.
- Use `domain:<name>` for xray and v2ray suffix conditions and `domain_suffix: <name>` for sing-box.
- Apply the mapping to routing, DNS rules and derived DNS lists, and TUN exclusions.

## Risks / Trade-offs

- Existing non-matching wildcard patterns begin matching the documented suffix intent.
- The emitted xray syntax is verified by existing configuration validation tests and the installed backend when available.

## Implementation plan

Tier: standard. Mode: existing-service-strict. Lenses: spec, quality. Rust stack — no red stage and no test-writer/tester agent exists for Rust chunks; every seam carries `NO-RED-WAIVER:` / `NO-TESTER-WAIVER:` and closes by waiver, contract tests are the first of each chunk's code tasks, written into the production files' inline `mod tests`. `planReview`: pass (zarchitect, 2 rounds — round 1 inherited from the pre-kernel-patch session; round 2 on this appendix, zero blockers).

Dispatch order: C1-helper → (C2-v2ray-family ∥ C3-singbox, disjoint shard worktrees) → C4-floor (joins the fork: run only after both C2 and C3 close).

### C1-helper — task 1.1 — rust-coder — integration worktree

- Site: `crates/core/src/config/common.rs`, new `strip_suffix_wildcard` beside anchor `pub(crate) fn split_horizon_server(settings: &AppSettings) -> Option<&DnsServerConfig> {`.
- Contract: `pub(crate) fn strip_suffix_wildcard(pattern: &str) -> &str`, strips exactly one leading `*.` (`pattern.strip_prefix("*.").unwrap_or(pattern)`); zero allocations, borrowed slice out. `*.google.com` → `google.com`; plain names unchanged; `*.*.example.com` → `*.example.com` (one strip only); `""` and bare `*` unchanged. Forbidden: second strip, stripping `*` without dot.
- Tests (new, inline `mod tests` in common.rs): wildcard, plain, one-strip-only, empty/bare-`*` cases.
- Verify: `timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::common`.

### C2-v2ray-family — tasks 2.1, 2.2 — rust-coder — shard `v2ray-family` (prev C1)

- Sites: `crates/core/src/config/v2ray.rs` — `build_routing_rule` Domain arm (anchor `"domain": [format!("domain:{pattern}")],`), TUN exclusion rule (anchor `"domain": &settings.tun.exclude_domains,`), custom DNS DomainSuffix arm (:887), derived Domain arm (:907), `exclude_domains` fold (:925-927), `attach_exclude_domains` (:979/:998/:1001/:1004); `crates/core/src/config/xray.rs` — `mod tests` (anchor `fn xray_vless_with_xtls() -> ProxyNode {`; xray delegates via `generate_v2ray_family_config`, xray.rs:33).
- Contract: every suffix-meaning emission routes through `strip_suffix_wildcard`; xray/v2ray emit `domain:<name>`; keyword → bare `sina`, full → `full:<name>`, geosite/bootstrap/hosts unchanged; TUN-disabled emits no exclusion-derived rules (pins :2603-2635 stay green). Seeding: `default_settings()` + field writes, `RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern }, action, enabled: true, group: None, via_node: None }`, `V2rayGenerator/XrayGenerator.generate(&[node], &rules, &settings)`.
- Breaking by design (expected updates): `test_domain_routing_rule` (`domain:*.google.com` → `domain:google.com`, v2ray.rs:1605), `test_xray_tun_exclusion_ip_and_domain` (:2424), `test_xray_tun_exclusion_dns` (:2518), `test_excluded_domains_bind_to_the_direct_detoured_server` (:2537). New: XrayGenerator wildcard test in xray.rs, wildcard derived-list and custom-DNS cases, a walk asserting no emitted suffix value contains `*`.
- Verify: `timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::v2ray config::xray`.

### C3-singbox — task 2.3 — rust-coder — shard `singbox` (prev C1)

- Site: `crates/core/src/config/singbox.rs` — `build_route_rule` Domain arm (anchor `"domain_suffix": [pattern],`, :836-839), custom DNS DomainSuffix arm (:565-568), derived Domain pushes (:598-601), TUN exclusion DNS rule (:547-552) and route rule (:705-710).
- Contract: same normalization, sing-box keys stay `domain_suffix` / `domain_keyword` / `domain`; `.cn`-style leading-dot suffixes pass through unchanged (pin :1969-1973); hosts/rule_set arms untouched; TUN-disabled/empty pins stay green.
- Tests: extend `test_singbox_domain_route_stays_suffix` (:1418) and `test_singbox_tun_exclusion_domain` (:2380) with wildcard cases, `test_singbox_dns_custom_rules_domain_keyword_and_full` (:1570) with a wildcard suffix case, new derived wildcard Proxy/Direct cases, no-`*` walk.
- Verify: `timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::singbox`.

### C4-floor — tasks 3.1, 3.2 — zpatcher — integration worktree (joins C2+C3)

- No code. Run `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`, `timeout 10m cargo test --workspace -- --test-threads=4`, and the xray syntax check below; report output verbatim.
- Verify: `timeout 10m cargo test --workspace -- --test-threads=4`.

### Floor (run once, Phase 5)

`timeout 10m cargo test --workspace -- --test-threads=4 && printf '%s' '{"inbounds":[{"port":1080,"protocol":"socks"}],"outbounds":[{"protocol":"freedom"}],"routing":{"rules":[{"type":"field","domain":["domain:google.com"],"outboundTag":"direct"}]}}' > /var/tmp/zapply/normalize-domain-suffix-emission/xray-suffix-check.json && xray run -test -c /var/tmp/zapply/normalize-domain-suffix-emission/xray-suffix-check.json`

(xray 26.9.9 verified installed at /usr/bin/xray.)

### Risks carried into the run

- Four tests break by design (listed in C2) — updates are part of the tasks, not regressions.
- Semantics shift: `domain:<name>` / `domain_suffix` match the apex too; apex-excluding matcher is a stated non-goal.
- `*.*.` inputs are unreachable via the editor (validation rejects, pinned `("*.*.example.com", false)` at models/validation.rs:313); the helper still strips exactly one prefix on pathological persisted values.
- Keyword/full/geosite/bootstrap/hosts emissions pinned unchanged — any of those tests failing means the coder over-reached.

### Rules for foreign agents (inlined; the kernel enforces the same)

- Worktree assertion: `git -C <worktree> rev-parse --show-toplevel` must equal the worktree path, else stop.
- Every path worktree-absolute; `git -C` for every git call; native file tools for reading/searching.
- Commit contract: subject ≤72 chars, body lines ≤100, imperative, lowercase, type from `feat fix refactor perf docs test build ci chore revert`; describe only the staged diff.
- Staging (sole occupant): `git -C <worktree> add -A -- ':!openspec'`; shared worktree: explicit pathspec — never stage a file you did not edit this run.
- One verify run per finished task; a conventional commit per finished task.
- A contract test once written is read-only — the only way to change one is a dispatched AMEND.

## Plan appendix

```json
{
  "v": 2,
  "change": "normalize-domain-suffix-emission",
  "baseSha": "1e8de4b2e8eb5ef7440330e43af18d1a836ce78c",
  "generatedAt": "2026-09-18T17:50:19.134Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "C1-helper",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "suffix-helper",
      "shard": "",
      "pkgDirs": [
        "crates/core"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/config/common.rs",
          "symbol": "strip_suffix_wildcard",
          "anchor": "pub(crate) fn split_horizon_server(settings: &AppSettings) -> Option<&DnsServerConfig> {",
          "change": "Add pub(crate) fn strip_suffix_wildcard(pattern: &str) -> &str beside split_horizon_server: strips exactly one leading \"*.\": pattern.strip_prefix(\"*.\").unwrap_or(pattern); zero allocations, borrowed slice out. Add inline #[cfg(test)] mod tests in common.rs covering: \"*.google.com\" -> \"google.com\", plain \"google.com\" unchanged, one-strip-only \"*.*.example.com\" -> \"*.example.com\", \"\" and bare \"*\" unchanged."
        }
      ],
      "contract": {
        "budgets": [
          "zero allocations: borrowed &str in, borrowed &str out"
        ],
        "forbidden": [
          "stripping a second `*.\"",
          "stripping a bare `*` not followed by a dot",
          "allocating: signature is &str -> &str returning a borrowed slice"
        ],
        "names": [
          "strip_suffix_wildcard — pub(crate) fn in crates/core/src/config/common.rs"
        ],
        "seeding": [
          "direct calls from a new #[cfg(test)] mod tests inside crates/core/src/config/common.rs; no AppSettings/RoutingRuleSet needed"
        ],
        "states": [
          "returned-unchanged",
          "stripped-once"
        ],
        "transitions": [
          {
            "input": "plain name 'google.com'",
            "state": "returned-unchanged",
            "effect": "no-op",
            "expect": "returned unchanged",
            "evidence": "validate_domain_pattern accepts plain names, models/validation.rs:96-116"
          },
          {
            "input": "'*.google.com'",
            "state": "stripped-once",
            "effect": "set",
            "expect": "returns 'google.com'",
            "evidence": "design.md decision: strip exactly one leading `*.\""
          },
          {
            "input": "'*.*.example.com'",
            "state": "stripped-once",
            "effect": "forced",
            "expect": "*.example.com — one strip only, residual `*` survives",
            "evidence": "design.md 'exactly one'; unreachable via editor because validation.rs:118-120 rejects >1 star, pinned by models/validation.rs:313"
          },
          {
            "input": "'' or bare '*'",
            "state": "returned-unchanged",
            "effect": "no-op",
            "expect": "returned unchanged (no `*.` prefix present)",
            "evidence": "invalid inputs per validation.rs:97 and :105; generator behavior for them unchanged"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "add strip_suffix_wildcard to config/common.rs plus inline mod tests covering wildcard ('*.google.com' -> 'google.com') and plain names"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::common",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: non-Go Rust stack — no test-writer agent or red stage for Rust chunks. NO-TESTER-WAIVER: no tester agent for Rust; contract tests are the first of codeTasks, written by the coder into the production files inline mod tests."
    },
    {
      "id": "C2-v2ray-family",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "C1-helper",
      "sharedPkg": null,
      "parallel": true,
      "seam": "v2ray-family-suffix-emission",
      "shard": "v2ray-family",
      "pkgDirs": [
        "crates/core"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_routing_rule",
          "anchor": "\"domain\": [format!(\"domain:{pattern}\")],",
          "change": "Route the Domain arm through strip_suffix_wildcard so \"*.google.com\" emits \"domain:google.com\"; update test_domain_routing_rule (v2ray.rs:1586-1606, pins \"domain:*.google.com\" at :1605 today) to expect \"domain:google.com\" and to assert no emitted suffix value contains \"*\". Keyword (test_domain_keyword_routing_rule :1608-1628) and full (test_domain_full_routing_rule :1630-1650) arms unchanged."
        },
        {
          "task": "2.2",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_user_dns_servers, attach_exclude_domains, build_routing",
          "anchor": "\"domain\": &settings.tun.exclude_domains,",
          "change": "Normalize all suffix emission sites: TUN exclusion routing rule (:551-557), custom DNS DomainSuffix arm (:887 format!(\"domain:{suffix}\")), derived Domain arm (:907), exclude_domains fold (:925-927), attach_exclude_domains (:979/:998/:1001/:1004 — compute the mapped Vec once at fn top). Update the four breaking tests: test_xray_tun_exclusion_ip_and_domain (:2424), test_xray_tun_exclusion_dns (:2518), test_excluded_domains_bind_to_the_direct_detoured_server (:2537), test_domain_routing_rule (:1587); add wildcard-input cases for derived lists and custom DNS rules; keep no-exclusion-when-disabled pins (:2603-2635)."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/config/xray.rs",
          "symbol": "mod tests (XrayGenerator wildcard assertion)",
          "anchor": "fn xray_vless_with_xtls() -> ProxyNode {",
          "change": "Add one XrayGenerator.generate-level test that a \"*.google.com\" Domain rule yields \"domain:google.com\" (xray delegates via generate_v2ray_family_config, v2ray.rs:78)."
        }
      ],
      "contract": {
        "budgets": [
          "no numeric budgets: value-for-value emission mapping"
        ],
        "forbidden": [
          "any emitted routing domain[] or dns domains[] value contains '*' for any input passing validate_domain_pattern (validation.rs:96-122 caps wildcards at one leading '*.')",
          "any unprefixed bare TUN-exclusion value in xray/v2ray routing rules or DNS server domains lists",
          "keyword, full, or geosite emission changes",
          "bootstrap 'full:' entries (v2ray.rs:769-777) or hosts pinning changes",
          "exclusion-derived routing or DNS entries when tun.enabled is false"
        ],
        "names": [
          "strip_suffix_wildcard (from seam 1.1)",
          "JSON keys unchanged: \"domain\", \"domains\", \"outboundTag\", \"server\", \"type\":\"field\"",
          "constants unchanged: DNS_DIRECT_TAG, AUTO_SPLIT_REMOTE_TAG, AUTO_SPLIT_DOMESTIC_TAG"
        ],
        "seeding": [
          "settings: default_settings() (in v2ray.rs mod tests) then field writes: settings.tun.enabled = true; settings.tun.exclude_domains = vec![...]; settings.dns.enabled = true; settings.dns.use_custom_rules = bool; settings.dns.servers = vec![DnsServerConfig{tag, protocol, address, port: None, detour}]; settings.dns.rules = vec![DnsRule{match_condition: DnsRuleMatch::DomainSuffix{suffix}, server_tag}]",
          "routing rules: RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: ... }, action: RuleAction::Proxy|Direct, enabled: true, group: None, via_node: None }",
          "generators: V2rayGenerator.generate(&[vless_node()], &rules, &settings); XrayGenerator.generate(...); generate_v2ray_family_config(&[ss_node()], &rules, &settings, V2rayFamilyBackend::Xray) — generate() takes &[RoutingRule], no RoutingRuleSet needed"
        ],
        "states": [
          "routing-domain-emitted",
          "dns-domains-emitted",
          "tun-routing-exclusion",
          "tun-dns-exclusion",
          "derived-dns-list",
          "no-exclusion"
        ],
        "transitions": [
          {
            "input": "routing Domain pattern '*.google.com'",
            "state": "routing-domain-emitted",
            "effect": "set",
            "expect": "routing.rules[0].domain == [\"domain:google.com\"]",
            "evidence": "spec delta xray routing; breaks test_domain_routing_rule v2ray.rs:1586-1606 (expected update)"
          },
          {
            "input": "routing Domain pattern plain 'example.com'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"domain:example.com\"]",
            "evidence": "pinned today at v2ray.rs:2316-2321 and :3191-3198"
          },
          {
            "input": "routing DomainKeyword 'sina'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"sina\"]",
            "evidence": "pinned by test_domain_keyword_routing_rule v2ray.rs:1608-1628 — unchanged"
          },
          {
            "input": "routing DomainFull 'example.com'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"full:example.com\"]",
            "evidence": "pinned by test_domain_full_routing_rule v2ray.rs:1630-1650 — unchanged"
          },
          {
            "input": "tun.enabled + exclude_domains ['example.com']",
            "state": "tun-routing-exclusion",
            "effect": "set",
            "expect": "routing rule {\"type\":\"field\",\"domain\":[\"domain:example.com\"],\"outboundTag\":\"direct\"}",
            "evidence": "spec delta: never unprefixed; breaks test_xray_tun_exclusion_ip_and_domain v2ray.rs:2422-2451 (expected update)"
          },
          {
            "input": "tun.enabled + exclude_domains ['*.corp.example']",
            "state": "tun-routing-exclusion",
            "effect": "set",
            "expect": "routing exclusion domain == [\"domain:corp.example\"]",
            "evidence": "helper applied to exclusion values, design.md"
          },
          {
            "input": "tun disabled + non-empty exclude_domains",
            "state": "no-exclusion",
            "effect": "no-op",
            "expect": "no exclusion-derived routing rule at all",
            "evidence": "pinned at v2ray.rs:2603-2618 (xray) and :2620-2635 (v2ray); must stay"
          },
          {
            "input": "custom DNS DomainSuffix '.cn'",
            "state": "derived-dns-list",
            "effect": "no-op",
            "expect": "server domains == [\"domain:.cn\"]",
            "evidence": "pinned at v2ray.rs:2249-2257; leading-dot suffix has no `*.` so helper passes it through"
          },
          {
            "input": "custom DNS DomainSuffix '*.google.com'",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "server domains == [\"domain:google.com\"]",
            "evidence": "spec delta DNS rules"
          },
          {
            "input": "custom DNS DomainKeyword / DomainFull",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "bare keyword / 'full:example.com' entries",
            "evidence": "pinned at v2ray.rs:2290-2293 — unchanged"
          },
          {
            "input": "derived: enabled Domain Proxy '*.google.com' (use_custom_rules=false)",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "remote server domains contains \"domain:google.com\"",
            "evidence": "v2ray.rs:907 emission site; spec delta derived DNS"
          },
          {
            "input": "derived: enabled Domain Direct '*.ru.example'",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "domestic server domains contains \"domain:ru.example\"",
            "evidence": "same site, v2ray.rs:907/930-941"
          },
          {
            "input": "derived: plain patterns",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "\"domain:example.com\" / \"domain:ru.example\" as today",
            "evidence": "pinned at v2ray.rs:3191-3198 — stays green"
          },
          {
            "input": "!use_custom_rules && tun.enabled && exclude_domains ['example.com']",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "domestic_domains gains \"domain:example.com\" (v2ray.rs:922-928 fold)",
            "evidence": "breaks test_xray_tun_exclusion_dns v2ray.rs:2506-2535 (expected update)"
          },
          {
            "input": "use_custom_rules && tun.enabled && exclude_domains ['corp.example']",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "split-horizon server domains contains \"domain:corp.example\" (all four push sites in attach_exclude_domains)",
            "evidence": "breaks test_excluded_domains_bind_to_the_direct_detoured_server v2ray.rs:2537-2575 (expected update)"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "route the 6 emission sites through strip_suffix_wildcard: build_routing_rule Domain arm (v2ray.rs:612-616), TUN exclusion routing rule (:551-557), custom DNS DomainSuffix arm (:885), derived Domain arm (:907), exclude_domains fold (:925-927), attach_exclude_domains (:979,:998,:1001,:1004 — compute the mapped Vec once at fn top and reuse)",
        "update the four breaking tests (test_domain_routing_rule, test_xray_tun_exclusion_ip_and_domain, test_xray_tun_exclusion_dns, test_excluded_domains_bind_to_the_direct_detoured_server) and add wildcard-input cases for derived lists and custom DNS rules",
        "add one XrayGenerator.generate-level assertion in xray.rs mod tests that a '*.google.com' rule yields \"domain:google.com\"",
        "add an assertion helper walking every emitted domain/domains value to assert none contains '*' (task 2.1 wording)"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::v2ray config::xray",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: non-Go Rust stack — no test-writer agent or red stage for Rust chunks. NO-TESTER-WAIVER: no tester agent for Rust; contract tests are the first of codeTasks, written by the coder into the production files inline mod tests."
    },
    {
      "id": "C3-singbox",
      "taskIds": [
        "2.3"
      ],
      "prev": "C1-helper",
      "sharedPkg": null,
      "parallel": true,
      "seam": "singbox-suffix-emission",
      "shard": "singbox",
      "pkgDirs": [
        "crates/core"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "build_dns, build_route, rule_to_singbox_value",
          "anchor": "\"domain_suffix\": [pattern],",
          "change": "Route the 5 emission sites through strip_suffix_wildcard: build_route_rule Domain arm (:836-839), custom DNS DomainSuffix arm (:565-568), derived Domain pushes (:598-601), TUN exclusion DNS rule (:547-552), TUN exclusion route rule (:705-710). Extend tests: test_singbox_domain_route_stays_suffix (:1418, wildcard case), test_singbox_dns_custom_rules_domain_keyword_and_full (:1570, add wildcard suffix case), test_singbox_tun_exclusion_domain (:2380, add wildcard exclude_domains case; plain-value assertions stay green), derived wildcard Proxy/Direct cases, plus a no-emitted-domain_suffix-contains-\"*\" assertion. Keyword/full/geosite/hosts/rule_set arms unchanged."
        }
      ],
      "contract": {
        "budgets": [
          "no numeric budgets: value-for-value emission mapping"
        ],
        "forbidden": [
          "any emitted domain_suffix value contains '*' for any input passing validate_domain_pattern",
          "keyword/full/geosite emission changes (domain_keyword, domain, rule_set arms)",
          "hosts_rule changes (:132-140 — hosts domains are exact-match keys, never suffixed)",
          "route.rule_set / .srs plumbing changes (apply_local_rule_sets untouched)"
        ],
        "names": [
          "strip_suffix_wildcard (from seam 1.1)",
          "JSON keys unchanged: \"domain_suffix\", \"domain_keyword\", \"domain\", \"server\", \"outbound\""
        ],
        "seeding": [
          "settings: default_settings() (in singbox.rs mod tests) then field writes: settings.tun.enabled = true; settings.tun.exclude_domains = vec![...]; settings.dns.enabled = true; settings.dns.use_custom_rules = bool; settings.dns.rules = vec![DnsRule{match_condition: DnsRuleMatch::DomainSuffix{suffix}, server_tag}]",
          "routing rules: RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: ... }, action, enabled: true, group: None, via_node: None }",
          "generator: SingboxGenerator.generate(&[ss_node()], &rules, &settings)"
        ],
        "states": [
          "route-domain-suffix-emitted",
          "dns-rule-domain-suffix-emitted",
          "derived-domain-suffix-emitted",
          "tun-route-exclusion",
          "tun-dns-exclusion",
          "no-exclusion"
        ],
        "transitions": [
          {
            "input": "routing Domain pattern '*.google.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "set",
            "expect": "route.rules[0].domain_suffix == [\"google.com\"]",
            "evidence": "spec delta sing-box routing; extend test_singbox_domain_route_stays_suffix singbox.rs:1418-1437"
          },
          {
            "input": "routing Domain pattern plain 'example.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix == [\"example.com\"]",
            "evidence": "pinned at singbox.rs:1436 — stays green"
          },
          {
            "input": "routing DomainKeyword 'sina'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_keyword == [\"sina\"]",
            "evidence": "pinned by test_singbox_domain_keyword_route :1440-1459 — unchanged"
          },
          {
            "input": "routing DomainFull 'example.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain == [\"example.com\"]",
            "evidence": "pinned by test_singbox_domain_full_route :1462-1481 — unchanged"
          },
          {
            "input": "custom DNS DomainSuffix '*.google.com'",
            "state": "tun-dns-exclusion",
            "effect": "set",
            "expect": "dns rule domain_suffix == [\"google.com\"]",
            "evidence": "spec delta DNS rules; :565-568 emission site"
          },
          {
            "input": "custom DNS DomainSuffix '.google.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix == [\".google.com\"]",
            "evidence": "pinned at :1986 — leading dot has no `*.`; unchanged"
          },
          {
            "input": "derived: enabled Domain Proxy '*.google.com'",
            "state": "derived-domain-suffix-emitted",
            "effect": "set",
            "expect": "remote derived rule domain_suffix contains \"google.com\"",
            "evidence": ":598-601 push site, :631 emission"
          },
          {
            "input": "derived: enabled Domain Direct '*.example.com'",
            "state": "derived-domain-suffix-emitted",
            "effect": "set",
            "expect": "domestic derived rule domain_suffix == [\"example.com\"]",
            "evidence": "spec delta scenario (domestic DNS rule for `*.example.com` direct rule)"
          },
          {
            "input": "tun.enabled + exclude_domains ['*.example.com']",
            "state": "tun-route-exclusion",
            "effect": "set",
            "expect": "route rule domain_suffix == [\"example.com\"] AND dns rule domain_suffix == [\"example.com\"]",
            "evidence": "spec delta TUN exclusions; sites :705-710 and :547-552"
          },
          {
            "input": "tun.enabled + exclude_domains plain 'example.com' / 'corp.example'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix values unchanged; direct-detoured server binding unchanged",
            "evidence": "pinned by test_singbox_tun_exclusion_domain :2380-2403 and test_singbox_excluded_domains_use_the_direct_detoured_server :2406-2438 — stay green"
          },
          {
            "input": "tun disabled (or lists empty)",
            "state": "no-exclusion",
            "effect": "no-op",
            "expect": "no exclusion-derived route or dns rules at all",
            "evidence": "pinned by test_singbox_no_exclusion_when_tun_disabled :2459-2494 and test_singbox_no_exclusion_when_lists_empty :2481+; must stay"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "route the 5 emission sites through strip_suffix_wildcard: build_route_rule Domain arm (:836-839), custom DNS DomainSuffix arm (:565-568), derived Domain pushes (:598-601), TUN exclusion DNS rule (:547-552), TUN exclusion route rule (:705-710)",
        "extend focused tests: wildcard routing, wildcard custom DNS suffix, wildcard derived Proxy/Direct, wildcard TUN exclusion; plus an assertion that no emitted domain_suffix value contains '*'"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core --lib -- --test-threads=4 config::singbox",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: non-Go Rust stack — no test-writer agent or red stage for Rust chunks. NO-TESTER-WAIVER: no tester agent for Rust; contract tests are the first of codeTasks, written by the coder into the production files inline mod tests."
    },
    {
      "id": "C4-floor",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "C3-singbox",
      "sharedPkg": null,
      "parallel": true,
      "seam": "verify-floor",
      "shard": "",
      "pkgDirs": [
        "crates/core"
      ],
      "pkgs": [],
      "sites": [],
      "contract": {
        "budgets": [
          "5m cap on the core run, 10m cap on the workspace run (per tasks.md)"
        ],
        "forbidden": [
          "floor declared green with any generator test failing or skipped",
          "skipping the xray syntax check while xray is installed"
        ],
        "seeding": [
          "n/a — runs the built test suite and the installed /usr/bin/xray binary"
        ],
        "states": [
          "core-green",
          "workspace-green",
          "xray-accepts"
        ],
        "transitions": [
          {
            "input": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
            "state": "core-green",
            "effect": "set",
            "expect": "core crate suite green",
            "evidence": "tasks.md 3.1"
          },
          {
            "input": "timeout 10m cargo test --workspace -- --test-threads=4",
            "state": "workspace-green",
            "effect": "set",
            "expect": "workspace suite green",
            "evidence": "tasks.md 3.2"
          },
          {
            "input": "xray run -test on a minimal config carrying the new routing matcher",
            "state": "xray-accepts",
            "effect": "set",
            "expect": "xray accepts the config (no config error)",
            "evidence": "tasks.md 3.2; xray installed at /usr/bin/xray (verified this session)"
          }
        ],
        "join": "dispatch only after BOTH C2-v2ray-family and C3-singbox are closed — the C2∥C3 fork joins before this chunk (prev names only C3 because the schema is singular)"
      },
      "redTasks": [],
      "codeTasks": [
        "run timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 and timeout 10m cargo test --workspace -- --test-threads=4; then run the xray suffix-check from the floor (xray run -test -c /var/tmp/zapply/normalize-domain-suffix-emission/xray-suffix-check.json); report output verbatim"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 10m cargo test --workspace -- --test-threads=4",
      "coder": "zpatcher",
      "waiver": "NO-RED-WAIVER: non-Go Rust stack — no test-writer agent or red stage for Rust chunks. NO-TESTER-WAIVER: no tester agent for Rust; contract tests are the first of codeTasks, written by the coder into the production files inline mod tests. This chunk is command verification only, no new behavior."
    }
  ],
  "seams": [
    {
      "id": "suffix-helper",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: non-Go Rust stack — no red stage for Rust chunks. NO-TESTER-WAIVER: no test-writer/tester agent exists for Rust chunks; the contract tests below are the first of codeTasks, written into the production file's inline mod tests. Pure shared helper stripping exactly one leading `*.` from suffix-meaning values.",
      "contract": {
        "budgets": [
          "zero allocations: borrowed &str in, borrowed &str out"
        ],
        "forbidden": [
          "stripping a second `*.\"",
          "stripping a bare `*` not followed by a dot",
          "allocating: signature is &str -> &str returning a borrowed slice"
        ],
        "names": [
          "strip_suffix_wildcard — pub(crate) fn in crates/core/src/config/common.rs"
        ],
        "seeding": [
          "direct calls from a new #[cfg(test)] mod tests inside crates/core/src/config/common.rs; no AppSettings/RoutingRuleSet needed"
        ],
        "states": [
          "returned-unchanged",
          "stripped-once"
        ],
        "transitions": [
          {
            "input": "plain name 'google.com'",
            "state": "returned-unchanged",
            "effect": "no-op",
            "expect": "returned unchanged",
            "evidence": "validate_domain_pattern accepts plain names, models/validation.rs:96-116"
          },
          {
            "input": "'*.google.com'",
            "state": "stripped-once",
            "effect": "set",
            "expect": "returns 'google.com'",
            "evidence": "design.md decision: strip exactly one leading `*.\""
          },
          {
            "input": "'*.*.example.com'",
            "state": "stripped-once",
            "effect": "forced",
            "expect": "*.example.com — one strip only, residual `*` survives",
            "evidence": "design.md 'exactly one'; unreachable via editor because validation.rs:118-120 rejects >1 star, pinned by models/validation.rs:313"
          },
          {
            "input": "'' or bare '*'",
            "state": "returned-unchanged",
            "effect": "no-op",
            "expect": "returned unchanged (no `*.` prefix present)",
            "evidence": "invalid inputs per validation.rs:97 and :105; generator behavior for them unchanged"
          }
        ]
      },
      "codeTasks": [
        "add strip_suffix_wildcard to config/common.rs plus inline mod tests covering wildcard ('*.google.com' -> 'google.com') and plain names"
      ]
    },
    {
      "id": "v2ray-family-suffix-emission",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: non-Go Rust stack — no red stage for Rust chunks. NO-TESTER-WAIVER: no test-writer/tester agent for Rust chunks; contract tests are the first of codeTasks in the production files' inline mod tests. All xray and v2ray domain emission lives in v2ray.rs (xray delegates via generate_v2ray_family_config, xray.rs:33); normalizes routing Domain rules, custom DNS DomainSuffix rules, derived DNS lists, and every TUN exclusion path.",
      "contract": {
        "budgets": [
          "no numeric budgets: value-for-value emission mapping"
        ],
        "forbidden": [
          "any emitted routing domain[] or dns domains[] value contains '*' for any input passing validate_domain_pattern (validation.rs:96-122 caps wildcards at one leading '*.')",
          "any unprefixed bare TUN-exclusion value in xray/v2ray routing rules or DNS server domains lists",
          "keyword, full, or geosite emission changes",
          "bootstrap 'full:' entries (v2ray.rs:769-777) or hosts pinning changes",
          "exclusion-derived routing or DNS entries when tun.enabled is false"
        ],
        "names": [
          "strip_suffix_wildcard (from seam 1.1)",
          "JSON keys unchanged: \"domain\", \"domains\", \"outboundTag\", \"server\", \"type\":\"field\"",
          "constants unchanged: DNS_DIRECT_TAG, AUTO_SPLIT_REMOTE_TAG, AUTO_SPLIT_DOMESTIC_TAG"
        ],
        "seeding": [
          "settings: default_settings() (in v2ray.rs mod tests) then field writes: settings.tun.enabled = true; settings.tun.exclude_domains = vec![...]; settings.dns.enabled = true; settings.dns.use_custom_rules = bool; settings.dns.servers = vec![DnsServerConfig{tag, protocol, address, port: None, detour}]; settings.dns.rules = vec![DnsRule{match_condition: DnsRuleMatch::DomainSuffix{suffix}, server_tag}]",
          "routing rules: RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: ... }, action: RuleAction::Proxy|Direct, enabled: true, group: None, via_node: None }",
          "generators: V2rayGenerator.generate(&[vless_node()], &rules, &settings); XrayGenerator.generate(...); generate_v2ray_family_config(&[ss_node()], &rules, &settings, V2rayFamilyBackend::Xray) — generate() takes &[RoutingRule], no RoutingRuleSet needed"
        ],
        "states": [
          "routing-domain-emitted",
          "dns-domains-emitted",
          "tun-routing-exclusion",
          "tun-dns-exclusion",
          "derived-dns-list",
          "no-exclusion"
        ],
        "transitions": [
          {
            "input": "routing Domain pattern '*.google.com'",
            "state": "routing-domain-emitted",
            "effect": "set",
            "expect": "routing.rules[0].domain == [\"domain:google.com\"]",
            "evidence": "spec delta xray routing; breaks test_domain_routing_rule v2ray.rs:1586-1606 (expected update)"
          },
          {
            "input": "routing Domain pattern plain 'example.com'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"domain:example.com\"]",
            "evidence": "pinned today at v2ray.rs:2316-2321 and :3191-3198"
          },
          {
            "input": "routing DomainKeyword 'sina'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"sina\"]",
            "evidence": "pinned by test_domain_keyword_routing_rule v2ray.rs:1608-1628 — unchanged"
          },
          {
            "input": "routing DomainFull 'example.com'",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "domain == [\"full:example.com\"]",
            "evidence": "pinned by test_domain_full_routing_rule v2ray.rs:1630-1650 — unchanged"
          },
          {
            "input": "tun.enabled + exclude_domains ['example.com']",
            "state": "tun-routing-exclusion",
            "effect": "set",
            "expect": "routing rule {\"type\":\"field\",\"domain\":[\"domain:example.com\"],\"outboundTag\":\"direct\"}",
            "evidence": "spec delta: never unprefixed; breaks test_xray_tun_exclusion_ip_and_domain v2ray.rs:2422-2451 (expected update)"
          },
          {
            "input": "tun.enabled + exclude_domains ['*.corp.example']",
            "state": "tun-routing-exclusion",
            "effect": "set",
            "expect": "routing exclusion domain == [\"domain:corp.example\"]",
            "evidence": "helper applied to exclusion values, design.md"
          },
          {
            "input": "tun disabled + non-empty exclude_domains",
            "state": "no-exclusion",
            "effect": "no-op",
            "expect": "no exclusion-derived routing rule at all",
            "evidence": "pinned at v2ray.rs:2603-2618 (xray) and :2620-2635 (v2ray); must stay"
          },
          {
            "input": "custom DNS DomainSuffix '.cn'",
            "state": "derived-dns-list",
            "effect": "no-op",
            "expect": "server domains == [\"domain:.cn\"]",
            "evidence": "pinned at v2ray.rs:2249-2257; leading-dot suffix has no `*.` so helper passes it through"
          },
          {
            "input": "custom DNS DomainSuffix '*.google.com'",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "server domains == [\"domain:google.com\"]",
            "evidence": "spec delta DNS rules"
          },
          {
            "input": "custom DNS DomainKeyword / DomainFull",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "bare keyword / 'full:example.com' entries",
            "evidence": "pinned at v2ray.rs:2290-2293 — unchanged"
          },
          {
            "input": "derived: enabled Domain Proxy '*.google.com' (use_custom_rules=false)",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "remote server domains contains \"domain:google.com\"",
            "evidence": "v2ray.rs:907 emission site; spec delta derived DNS"
          },
          {
            "input": "derived: enabled Domain Direct '*.ru.example'",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "domestic server domains contains \"domain:ru.example\"",
            "evidence": "same site, v2ray.rs:907/930-941"
          },
          {
            "input": "derived: plain patterns",
            "state": "routing-domain-emitted",
            "effect": "no-op",
            "expect": "\"domain:example.com\" / \"domain:ru.example\" as today",
            "evidence": "pinned at v2ray.rs:3191-3198 — stays green"
          },
          {
            "input": "!use_custom_rules && tun.enabled && exclude_domains ['example.com']",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "domestic_domains gains \"domain:example.com\" (v2ray.rs:922-928 fold)",
            "evidence": "breaks test_xray_tun_exclusion_dns v2ray.rs:2506-2535 (expected update)"
          },
          {
            "input": "use_custom_rules && tun.enabled && exclude_domains ['corp.example']",
            "state": "derived-dns-list",
            "effect": "set",
            "expect": "split-horizon server domains contains \"domain:corp.example\" (all four push sites in attach_exclude_domains)",
            "evidence": "breaks test_excluded_domains_bind_to_the_direct_detoured_server v2ray.rs:2537-2575 (expected update)"
          }
        ]
      },
      "codeTasks": [
        "route the 6 emission sites through strip_suffix_wildcard: build_routing_rule Domain arm (v2ray.rs:612-616), TUN exclusion routing rule (:551-557), custom DNS DomainSuffix arm (:885), derived Domain arm (:907), exclude_domains fold (:925-927), attach_exclude_domains (:979,:998,:1001,:1004 — compute the mapped Vec once at fn top and reuse)",
        "update the four breaking tests (test_domain_routing_rule, test_xray_tun_exclusion_ip_and_domain, test_xray_tun_exclusion_dns, test_excluded_domains_bind_to_the_direct_detoured_server) and add wildcard-input cases for derived lists and custom DNS rules",
        "add one XrayGenerator.generate-level assertion in xray.rs mod tests that a '*.google.com' rule yields \"domain:google.com\"",
        "add an assertion helper walking every emitted domain/domains value to assert none contains '*' (task 2.1 wording)"
      ]
    },
    {
      "id": "singbox-suffix-emission",
      "tasks": [
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: non-Go Rust stack — no red stage for Rust chunks. NO-TESTER-WAIVER: no test-writer/tester agent for Rust chunks; contract tests are the first of codeTasks in singbox.rs's inline mod tests. sing-box already emits domain_suffix keys; this seam normalizes the values (strip one leading `*.`) across routing, custom DNS rules, derived DNS lists, and both TUN exclusion paths.",
      "contract": {
        "budgets": [
          "no numeric budgets: value-for-value emission mapping"
        ],
        "forbidden": [
          "any emitted domain_suffix value contains '*' for any input passing validate_domain_pattern",
          "keyword/full/geosite emission changes (domain_keyword, domain, rule_set arms)",
          "hosts_rule changes (:132-140 — hosts domains are exact-match keys, never suffixed)",
          "route.rule_set / .srs plumbing changes (apply_local_rule_sets untouched)"
        ],
        "names": [
          "strip_suffix_wildcard (from seam 1.1)",
          "JSON keys unchanged: \"domain_suffix\", \"domain_keyword\", \"domain\", \"server\", \"outbound\""
        ],
        "seeding": [
          "settings: default_settings() (in singbox.rs mod tests) then field writes: settings.tun.enabled = true; settings.tun.exclude_domains = vec![...]; settings.dns.enabled = true; settings.dns.use_custom_rules = bool; settings.dns.rules = vec![DnsRule{match_condition: DnsRuleMatch::DomainSuffix{suffix}, server_tag}]",
          "routing rules: RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::Domain { pattern: ... }, action, enabled: true, group: None, via_node: None }",
          "generator: SingboxGenerator.generate(&[ss_node()], &rules, &settings)"
        ],
        "states": [
          "route-domain-suffix-emitted",
          "dns-rule-domain-suffix-emitted",
          "derived-domain-suffix-emitted",
          "tun-route-exclusion",
          "tun-dns-exclusion",
          "no-exclusion"
        ],
        "transitions": [
          {
            "input": "routing Domain pattern '*.google.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "set",
            "expect": "route.rules[0].domain_suffix == [\"google.com\"]",
            "evidence": "spec delta sing-box routing; extend test_singbox_domain_route_stays_suffix singbox.rs:1418-1437"
          },
          {
            "input": "routing Domain pattern plain 'example.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix == [\"example.com\"]",
            "evidence": "pinned at singbox.rs:1436 — stays green"
          },
          {
            "input": "routing DomainKeyword 'sina'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_keyword == [\"sina\"]",
            "evidence": "pinned by test_singbox_domain_keyword_route :1440-1459 — unchanged"
          },
          {
            "input": "routing DomainFull 'example.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain == [\"example.com\"]",
            "evidence": "pinned by test_singbox_domain_full_route :1462-1481 — unchanged"
          },
          {
            "input": "custom DNS DomainSuffix '*.google.com'",
            "state": "tun-dns-exclusion",
            "effect": "set",
            "expect": "dns rule domain_suffix == [\"google.com\"]",
            "evidence": "spec delta DNS rules; :565-568 emission site"
          },
          {
            "input": "custom DNS DomainSuffix '.google.com'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix == [\".google.com\"]",
            "evidence": "pinned at :1986 — leading dot has no `*.`; unchanged"
          },
          {
            "input": "derived: enabled Domain Proxy '*.google.com'",
            "state": "derived-domain-suffix-emitted",
            "effect": "set",
            "expect": "remote derived rule domain_suffix contains \"google.com\"",
            "evidence": ":598-601 push site, :631 emission"
          },
          {
            "input": "derived: enabled Domain Direct '*.example.com'",
            "state": "derived-domain-suffix-emitted",
            "effect": "set",
            "expect": "domestic derived rule domain_suffix == [\"example.com\"]",
            "evidence": "spec delta scenario (domestic DNS rule for `*.example.com` direct rule)"
          },
          {
            "input": "tun.enabled + exclude_domains ['*.example.com']",
            "state": "tun-route-exclusion",
            "effect": "set",
            "expect": "route rule domain_suffix == [\"example.com\"] AND dns rule domain_suffix == [\"example.com\"]",
            "evidence": "spec delta TUN exclusions; sites :705-710 and :547-552"
          },
          {
            "input": "tun.enabled + exclude_domains plain 'example.com' / 'corp.example'",
            "state": "route-domain-suffix-emitted",
            "effect": "no-op",
            "expect": "domain_suffix values unchanged; direct-detoured server binding unchanged",
            "evidence": "pinned by test_singbox_tun_exclusion_domain :2380-2403 and test_singbox_excluded_domains_use_the_direct_detoured_server :2406-2438 — stay green"
          },
          {
            "input": "tun disabled (or lists empty)",
            "state": "no-exclusion",
            "effect": "no-op",
            "expect": "no exclusion-derived route or dns rules at all",
            "evidence": "pinned by test_singbox_no_exclusion_when_tun_disabled :2459-2494 and test_singbox_no_exclusion_when_lists_empty :2481+; must stay"
          }
        ]
      },
      "codeTasks": [
        "route the 5 emission sites through strip_suffix_wildcard: build_route_rule Domain arm (:836-839), custom DNS DomainSuffix arm (:565-568), derived Domain pushes (:598-601), TUN exclusion DNS rule (:547-552), TUN exclusion route rule (:705-710)",
        "extend focused tests: wildcard routing, wildcard custom DNS suffix, wildcard derived Proxy/Direct, wildcard TUN exclusion; plus an assertion that no emitted domain_suffix value contains '*'"
      ]
    },
    {
      "id": "verify-floor",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: non-Go Rust stack — no red stage for Rust chunks. NO-TESTER-WAIVER: no tester agent for Rust chunks; command verification only, no new behavior.",
      "contract": {
        "budgets": [
          "5m cap on the core run, 10m cap on the workspace run (per tasks.md)"
        ],
        "forbidden": [
          "floor declared green with any generator test failing or skipped",
          "skipping the xray syntax check while xray is installed"
        ],
        "seeding": [
          "n/a — runs the built test suite and the installed /usr/bin/xray binary"
        ],
        "states": [
          "core-green",
          "workspace-green",
          "xray-accepts"
        ],
        "transitions": [
          {
            "input": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
            "state": "core-green",
            "effect": "set",
            "expect": "core crate suite green",
            "evidence": "tasks.md 3.1"
          },
          {
            "input": "timeout 10m cargo test --workspace -- --test-threads=4",
            "state": "workspace-green",
            "effect": "set",
            "expect": "workspace suite green",
            "evidence": "tasks.md 3.2"
          },
          {
            "input": "xray run -test on a minimal config carrying the new routing matcher",
            "state": "xray-accepts",
            "effect": "set",
            "expect": "xray accepts the config (no config error)",
            "evidence": "tasks.md 3.2; xray installed at /usr/bin/xray (verified this session)"
          }
        ]
      },
      "codeTasks": [
        "run the two cargo commands and the xray run -test check; report output verbatim"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "Every generator SHALL emit suffix-meaning domain conditions with backend matchers that match the named domain and all its subdomains, in routing rules, DNS rules, derived DNS server domain lists, and TUN exclusions. Xray and v2ray SHALL emit `domain:<name>`; sing-box SHALL emit `domain_suffix` with `<name>`. A leading `*.` SHALL be removed before emission. A domain keyword SHALL remain a substring matcher, and a full domain SHALL remain an exact matcher. No emitted suffix value SHALL contain `*`.",
      "tests": [
        "config::common::tests strip_suffix_wildcard cases (new: wildcard, plain, one-strip-only, empty/bare *)",
        "test_domain_routing_rule (updated: domain:google.com + no-* walk)",
        "XrayGenerator wildcard routing test (new, xray.rs mod tests)",
        "test_domain_keyword_routing_rule (pin, unchanged)",
        "test_domain_full_routing_rule (pin, unchanged)",
        "custom DNS wildcard DomainSuffix case (new, v2ray.rs)",
        "derived DNS wildcard cases Proxy + Direct (new, v2ray.rs)",
        "test_singbox_domain_route_stays_suffix (extended: wildcard case)",
        "test_singbox_dns_custom_rules_domain_keyword_and_full (pin, unchanged)",
        "test_singbox_domain_keyword_route (pin)",
        "test_singbox_domain_full_route (pin)",
        "singbox custom DNS wildcard suffix case (new)",
        "singbox derived wildcard cases Proxy + Direct (new)"
      ]
    },
    {
      "shall": "the routing rule SHALL contain `\"domain\": [\"domain:google.com\"]`",
      "tests": [
        "test_domain_routing_rule (updated)",
        "XrayGenerator wildcard routing test (new, xray.rs mod tests)"
      ]
    },
    {
      "shall": "the route rule SHALL contain `\"domain_suffix\": [\"google.com\"]`",
      "tests": [
        "test_singbox_domain_route_stays_suffix (extended: wildcard case)"
      ]
    },
    {
      "shall": "xray's domestic server SHALL contain `domain:example.com` and sing-box's domestic DNS rule SHALL contain `domain_suffix` `example.com`",
      "tests": [
        "derived DNS wildcard Direct case (new, v2ray.rs mod tests)",
        "derived domestic wildcard case (new, singbox.rs mod tests)"
      ]
    },
    {
      "shall": "xray and v2ray SHALL emit `\"domain\": [\"sina\"]` and sing-box SHALL emit `\"domain_keyword\": [\"sina\"]`",
      "tests": [
        "test_domain_keyword_routing_rule (pin)",
        "test_singbox_domain_keyword_route (pin)",
        "test_singbox_dns_custom_rules_domain_keyword_and_full (pin)"
      ]
    },
    {
      "shall": "When TUN is enabled, the system SHALL generate backend rules that keep configured processes and destinations out of the tunnel, mapped to each backend's native mechanism. Process-name exclusion SHALL be emitted for sing-box only. Destination exclusion (CIDR and domain) SHALL be emitted for both backends and SHALL precede user routing rules. Excluded DNS SHALL resolve directly through the first DNS server detoured to `direct`, or the first configured server when none is detoured. Excluded domains SHALL match the named domain and its subdomains: sing-box SHALL emit `domain_suffix`; xray SHALL emit `domain:<name>` in routing rules and DNS server `domains` lists, never the unprefixed name.",
      "tests": [
        "test_xray_tun_exclusion_ip_and_domain (updated: domain:example.com)",
        "test_xray_tun_exclusion_dns (updated)",
        "test_excluded_domains_bind_to_the_direct_detoured_server (updated)",
        "test_singbox_tun_exclusion_domain (extended: wildcard exclude_domains case)",
        "test_singbox_tun_exclusion_domain (pin)",
        "test_singbox_excluded_domains_use_the_direct_detoured_server (pin)"
      ]
    },
    {
      "shall": "`domain:example.com` SHALL be bound to the DNS server detoured to `direct`, or to the first configured server when none is detoured",
      "tests": [
        "test_xray_tun_exclusion_dns (updated)",
        "test_excluded_domains_bind_to_the_direct_detoured_server (updated)"
      ]
    },
    {
      "shall": "no generated routing rule or DNS server `domains` list SHALL contain the unprefixed string `wb.ru`",
      "tests": [
        "test_xray_tun_exclusion_ip_and_domain (updated: unprefixed-value walk asserts domain: prefix everywhere)"
      ]
    },
    {
      "shall": "sing-box `route.rules` SHALL include, ahead of user rules, `{ \"process_name\": [\"cloudflared\"], \"outbound\": \"direct\" }`",
      "tests": [
        "test_singbox_tun_exclusion_process_name (pin, unchanged)"
      ]
    },
    {
      "shall": "`route.rules` SHALL include `{ \"domain_suffix\": [\"example.com\"], \"outbound\": \"direct\" }` ahead of user rules and `dns.rules` SHALL route those domains to the direct-detoured server, or the first configured server when none is detoured",
      "tests": [
        "test_singbox_tun_exclusion_domain (pin)",
        "test_singbox_excluded_domains_use_the_direct_detoured_server (pin)",
        "test_singbox_tun_exclusion_ip_and_domain (extended)"
      ]
    },
    {
      "shall": "`routing.rules` SHALL include, ahead of user rules, direct rules for the CIDR and `domain:example.com`",
      "tests": [
        "test_xray_tun_exclusion_ip_and_domain (updated)"
      ]
    },
    {
      "shall": "neither generator SHALL emit exclusions derived from `exclude_processes`, `exclude_domains`, or `exclude_routes`",
      "tests": [
        "test_xray_no_exclusion_when_tun_disabled (pin)",
        "test_v2ray_never_emits_exclusion (pin)",
        "test_singbox_no_exclusion_when_tun_disabled (pin)",
        "test_singbox_no_exclusion_when_lists_empty (pin)"
      ]
    }
  ],
  "testHarness": [
    "default_settings() — crates/core/src/config/test_fixtures.rs:5 — builds default AppSettings",
    "vless_node() — crates/core/src/config/test_fixtures.rs:9 — builds VLESS ProxyNode with WebSocket transport and TLS",
    "vmess_node() — crates/core/src/config/test_fixtures.rs:30 — builds VMess ProxyNode with TCP transport",
    "ss_node() — crates/core/src/config/test_fixtures.rs:43 — builds Shadowsocks ProxyNode with aes-256-gcm",
    "trojan_node() — crates/core/src/config/test_fixtures.rs:53 — builds Trojan ProxyNode with TCP transport and TLS",
    "xhttp_node() — crates/core/src/config/test_fixtures.rs:67 — builds VLESS ProxyNode with XHTTP transport and REALITY TLS",
    "xray_vless_with_xtls() — crates/core/src/config/xray.rs:200 — builds VLESS ProxyNode with TCP transport and xtls-rprx-vision",
    "ws_vless_with_host_header(headers) — crates/core/src/config/xray.rs:217 — builds VLESS ProxyNode with WebSocket transport and custom headers",
    "proxy_rule(pattern, via_node) — crates/core/src/config/v2ray.rs:1128 — builds Proxy RoutingRule with RuleMatch::Domain(pattern)",
    "udp_server(tag) — crates/core/src/config/v2ray.rs:3115 — builds UDP DnsServerConfig with specified tag",
    "direct_rule(pattern) — crates/core/src/config/v2ray.rs:3125 — builds Direct RoutingRule with RuleMatch::Domain(pattern)"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4 && printf '%s' '{\"inbounds\":[{\"port\":1080,\"protocol\":\"socks\"}],\"outbounds\":[{\"protocol\":\"freedom\"}],\"routing\":{\"rules\":[{\"type\":\"field\",\"domain\":[\"domain:google.com\"],\"outboundTag\":\"direct\"}]}}' > /var/tmp/zapply/normalize-domain-suffix-emission/xray-suffix-check.json && xray run -test -c /var/tmp/zapply/normalize-domain-suffix-emission/xray-suffix-check.json",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
