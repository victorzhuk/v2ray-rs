## Context

See proposal.md for motivation. Stored routing rules deserialize without validation, while editor saves validate routing matches. DNS keyword input and row rendering lack equivalent validation.

## Goals / Non-Goals

**Goals:**
- Prevent new wildcard keyword rules.
- Preserve stored invalid values and make their invalid state actionable.

**Non-Goals:**
- Rewrite stored rules.
- Convert a wildcard keyword into a suffix rule.
- Change generator emission of stored invalid rules.

## Decisions

- Reject every `*` in domain keywords with a message directing users to the Domain rule type for suffix matching.
- Reuse core keyword validation in routing and DNS UI paths.
- Render the validation message as an invalid stored rule row's subtitle, replacing the normal action or server subtitle.

## Risks / Trade-offs

- Existing invalid rules remain active data until users edit or delete them; silent conversion would change routing intent.

## Implementation plan (human-readable)

Tier: standard. Mode: existing-service-strict. baseSha `41d05f4`. Review lenses: `spec`, `quality` (no auth/SQL/boundary/hot-path triggers in the diff). Four chunks, two waves; all chunks are Rust `rust-coder` chunks.

**Waves (dependency is on wave order, not prev/sharedPkg):** wave 1 = the two core shards, cut from `zapply/<change>` at base; wave 2 = the two ui shards, cut only after wave 1 shards merged — the ui helper tests assert core's new `ValidationError::WildcardDomainKeyword` and its exact Display, so ui shards must contain wave-1 code.

### Chunk 1 `core-wildcard-validation` — task 1.1, wave 1, shard `core-validation`, parallel

- Sites: `crates/core/src/models/validation.rs` (`validate_domain_keyword`), `crates/core/src/models/routing.rs` (mutation tests), NEW `crates/core/tests/domain_keyword_validation.rs`.
- Contract (states `accepted` / `rejected_whitespace_or_empty` / `rejected_wildcard`): empty or any-whitespace keyword → `InvalidDomainKeyword` (existing, first check — precedence fixed by insertion point); non-blank keyword containing `*` anywhere (`*.ru`, `*`, `a*b`, `sina*`) → `WildcardDomainKeyword`; both → whitespace wins; plain keyword → Ok. `add_validated` / `add_at` / `edit_rule` with wildcard keyword → Err, no mutation, `rules().len()` unchanged.
- New production member: variant `ValidationError::WildcardDomainKeyword(String)` directly after `InvalidDomainKeyword`, `#[error("invalid domain keyword '{0}': a keyword is a plain substring and cannot contain '*' ; use the Domain rule type for wildcard suffixes like '*.example.com'")]` — exact text lives only in the thiserror attribute, consumed via Display; never hand-formatted elsewhere.
- Tests (coder writes them first — NO-RED-WAIVER/NO-TESTER-WAIVER: no test-writer or tester agent exists for this Rust stack): `wildcard_keyword_is_rejected` (exact variant + exact Display string), `plain_keyword_is_accepted`, `rule_match_dispatch_rejects_wildcard_keyword`, `validated_mutations_reject_wildcard_keyword`; extend inline `test_validate_domain_keyword` table with `("*.ru", false)`, `("a*b", false)`.
- Verify: `cargo check -p v2ray-rs-core && timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4` (subscription run = import-skip consequence guard).

### Chunk 2 `stored-rule-recoverability` — task 1.2, wave 1, shard `core-persistence`, parallel

- Sites: `crates/core/src/persistence/routing.rs` (inline tests optional), NEW `crates/core/tests/routing_rules_wildcard_roundtrip.rs` (first line `#![cfg(feature = "test-utils")]`).
- Contract: load of stored `{"type":"domain_keyword","keyword":"*.ru"}` → no error, no rewrite; save+load round-trip → set byte-identical (`RoutingRuleSet: PartialEq`); missing file → empty set. Seeding only via `RoutingRuleSet::add()` (pub, non-validating) or `serde_json::from_str`; `add_validated` forbidden for this state.
- No production change expected — persistence is already validation-free; if the test fails, fix load/save to stay validation-free, never weaken the test.
- Test: `stored_wildcard_keyword_rule_loads_and_roundtrips`. Verify: `timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4`.

### Chunk 3 `routing-row-invalid-display` — task 2.1, wave 2, shard `ui-routing`, parallel

- Sites: `crates/ui/src/preferences/routing.rs` (`build_routing_rule_row`, new inline `#[cfg(test)]` mod).
- Contract (states `row_clean` / `row_invalid`): stored rule passing `validate_rule_match` → normal action subtitle; stored rule failing any check (`DomainKeyword "*.ru"`, blank keyword, bad geosite — superset of wildcard by design) → subtitle = `err.to_string()` from core, `row.add_css_class("error")`, title stays `format_match(...)` so the row still reads `Domain Keyword: *.ru`. Render is read-only; helper stays pure (no gtk/adw types, no message literals).
- New private fn `rule_row_validation_error(rule: &RoutingRule) -> Option<String>` = `validate_rule_match(&rule.match_condition).err().map(|e| e.to_string())`, next to `format_match`. Pattern: `network.rs` error css + subtitle.
- Tests: `rule_row_validation_error_flags_wildcard_keyword`, `rule_row_validation_error_passes_valid_rules` (plain inline tests, no gtk event loop). Verify: `cargo check -p v2ray-rs-ui && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4`.

### Chunk 4 `dns-keyword-validation-ui` — task 2.2, wave 2, shard `ui-dns`, parallel

- Sites: `crates/ui/src/preferences/dns.rs` (`dns_rule_from_inputs` arm, `render_dns_rules` row build, existing inline `mod tests`), `crates/core/src/models/dns.rs` (`test_dns_rule_match_domain_keyword_roundtrip`).
- Contract (states `input_rejected_inline` / `input_saved` / `stored_row_invalid` / `stored_row_clean`): dialog Domain Keyword value with `*`, whitespace, or empty → Err from `dns_rule_from_inputs` → existing dialog machinery shows inline error and disables Save (both guards stay: `update_validation` AND the `validate()` re-check in `connect_response`); plain value → save proceeds; stored wildcard keyword row → subtitle = validation message replacing `Server: {tag}`, `add_css_class("error")`; non-keyword rows and plain keywords unchanged. Stored values round-trip verbatim — no rewrite on render or edit-open.
- New private fn `dns_rule_match_validation_error(m: &DnsRuleMatch) -> Option<String>` — DomainKeyword arm delegates to `validate_domain_keyword`, other arms None; text always from core Display.
- Tests: `dns_rule_match_validation_error_rejects_wildcard_and_whitespace`, `dns_rule_match_validation_error_passes_plain_keyword_and_other_matches`; extend core `test_dns_rule_match_domain_keyword_roundtrip` with a stored `*.cn` round-trip case (regression proof for "stored DNS keyword rules stay unchanged").
- Verify: `cargo check -p v2ray-rs-ui && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4`.

**Floor:** `timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 && timeout 10m cargo test --workspace -- --test-threads=4` (floor covers tasks 3.1/3.2; tasks 3.1/3.2 are the verify layer, owned by no seam).

**Plan review:** pass (zarchitect, 2 rounds). Round-1 blocker (DNS stored-unchanged SHALL mapped to a non-wildcard test) fixed by the dns.rs round-trip extension. Remaining warnings: wave dependency lives in this prose + seam summaries (dispatcher enforces wave order); chunk 4 crosses into `crates/core/src/models/dns.rs`.

For a foreign agent without the kernel: worktree assertion (`git -C <worktree> rev-parse --show-toplevel` must equal the worktree, else stop); every path worktree-absolute; `git -C` for every git call; commit contract (subject ≤72, body lines ≤100, imperative, lowercase, type from `feat fix refactor perf docs test build ci chore revert`); sole-occupant staging `git add -A -- ':!openspec'`, shared worktree explicit pathspec, never stage a file you did not edit this run; one verify run per code change; a conventional commit per finished task; a contract test once written is read-only.

## Plan appendix (machine-readable)

## Plan appendix

```json
{
  "v": 2,
  "change": "reject-wildcard-domain-keywords",
  "baseSha": "41d05f4ca37358585ba45ef2d30a9894793fd407",
  "generatedAt": "2026-09-18T18:32:27.559Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "waves": "Waves: chunk wave 1 = core chunks (parallel shards, cut from zapply/<change> at base); wave 2 = ui chunks (parallel shards, cut only after wave 1 shards merged into zapply/<change>) — the ui helper tests assert core's new ValidationError::WildcardDomainKeyword and its exact Display, so the ui shards must include wave-1 code. Dependency is on wave order, not prev/sharedPkg.",
  "chunks": [
    {
      "id": "core-wildcard-validation",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "core-keyword-validation",
      "shard": "core-validation",
      "pkgDirs": [
        "crates/core/src/models",
        "crates/core/tests"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/validation.rs",
          "symbol": "validate_domain_keyword",
          "anchor": "pub fn validate_domain_keyword(keyword: &str) -> Result<(), ValidationError> {",
          "change": "Reject \"*\" in validate_domain_keyword with a ValidationError message/variant stating that keywords are plain substrings and directing users to the Domain rule type; update test_validate_domain_keyword with wildcard rejection and plain keyword acceptance cases."
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/routing.rs",
          "symbol": "tests::test_edit_rule_invalid_match",
          "anchor": "fn test_edit_rule_invalid_match() {",
          "change": "Add tests asserting that RoutingRuleSet::add_validated, add_at, and edit_rule reject RuleMatch::DomainKeyword containing \"*\" with ValidationError, but accept plain keywords."
        },
        {
          "task": "1.1",
          "file": "crates/core/tests/domain_keyword_validation.rs",
          "symbol": "wildcard_keyword_is_rejected, plain_keyword_is_accepted, rule_match_dispatch_rejects_wildcard_keyword, validated_mutations_reject_wildcard_keyword",
          "anchor": "wildcard_keyword_is_rejected / plain_keyword_is_accepted / rule_match_dispatch_rejects_wildcard_keyword / validated_mutations_reject_wildcard_keyword",
          "change": "NEW integration test file creating: wildcard_keyword_is_rejected, plain_keyword_is_accepted, rule_match_dispatch_rejects_wildcard_keyword, validated_mutations_reject_wildcard_keyword",
          "new": true
        }
      ],
      "contract": {
        "states": [
          "accepted",
          "rejected_whitespace_or_empty",
          "rejected_wildcard"
        ],
        "transitions": [
          {
            "input": "empty string or any keyword containing a whitespace char (\"\", \" \", \"has space\")",
            "state": "rejected_whitespace_or_empty",
            "effect": "set",
            "evidence": "crates/core/src/models/validation.rs:128-133 (existing check, unchanged; covers the dns-preferences-ui delta's whitespace clause)"
          },
          {
            "input": "non-blank keyword containing '*' anywhere (\"*.ru\", \"*\", \"a*b\", \"sina*\")",
            "state": "rejected_wildcard",
            "effect": "set",
            "evidence": "openspec routing-rules delta Requirement 'Keyword rules reject wildcards'; design.md decision 1; new check appended in validate_domain_keyword (validation.rs:128-133)"
          },
          {
            "input": "keyword with BOTH whitespace and '*' (\" * \")",
            "state": "rejected_whitespace_or_empty",
            "effect": "set",
            "evidence": "check order in validate_domain_keyword: whitespace test first (validation.rs:129), '*' test added after it - precedence is fixed by insertion point"
          },
          {
            "input": "plain keyword: non-empty, no whitespace, no '*' (\"sina\", \"sina.com\", \".example\")",
            "state": "accepted",
            "effect": "clear",
            "evidence": "routing-rules delta scenario 'plain keyword accepted (sina stored)'"
          },
          {
            "input": "RoutingRuleSet::add_validated(rule with DomainKeyword \"*.ru\")",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:111-115 gates via validate_rule_match; Err returned, rule NOT pushed, rules().len() unchanged"
          },
          {
            "input": "RoutingRuleSet::add_at(i, rule with DomainKeyword \"*.ru\")",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:117-126 (validate before insert)"
          },
          {
            "input": "RoutingRuleSet::edit_rule(id, Some(DomainKeyword \"*.ru\"), _)",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:128-149 (validate_rule_match before assigning match_condition)"
          }
        ],
        "forbidden": [
          "validate_domain_keyword returning Ok(()) for any input containing '*'",
          "add_validated/add_at/edit_rule persisting a wildcard keyword rule or leaving a partially applied edit on Err",
          "WildcardDomainKeyword returned for inputs with no '*' (empty/whitespace keep InvalidDomainKeyword, validation.rs:14-15)",
          "any caller (UI helper, test) hand-formatting the message string - the exact text lives only in the thiserror attribute and is consumed via Display"
        ],
        "seeding": [
          "validator states: call pub v2ray_rs_core::models::validate_domain_keyword and validate_rule_match directly (re-exported at crates/core/src/models/mod.rs:44-48)",
          "mutation-gate states: RoutingRuleSet::new() + RoutingRule struct literal (all fields pub, routing.rs:7-18) + add_validated/add_at/edit_rule - the only pub path; the rules field is private so field-write seeding is impossible"
        ],
        "budgets": [
          "validation is a single pass, O(len(keyword)), no allocation on the Ok path (one String only on Err)",
          "chunk A test run bounded by timeout 5m with --test-threads=4"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TESTS FIRST: write crates/core/tests/domain_keyword_validation.rs (new plain integration test, no feature gate; import via v2ray_rs_core::models::{...}, precedent crates/core/tests/singbox_check.rs:7-10). Tests: wildcard_keyword_is_rejected - assert_eq!(validate_domain_keyword(\"*.ru\"), Err(ValidationError::WildcardDomainKeyword(\"*.ru\".to_string()))) plus exact Display equality with \"invalid domain keyword '*.ru': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'\"; also \"*\", \"a*b\", \"sina*\" each Err(WildcardDomainKeyword(_)); plain_keyword_is_accepted - \"sina\", \"sina.com\", \".example\" -> Ok(()); rule_match_dispatch_rejects_wildcard_keyword - validate_rule_match(&RuleMatch::DomainKeyword { keyword: \"*.ru\".into() }) is Err; validated_mutations_reject_wildcard_keyword - fresh RoutingRuleSet, then add_validated, add_at(0, ..), edit_rule(..) each with a wildcard-keyword rule return Err(WildcardDomainKeyword(_)) and leave rules().len() == 0.",
        "Implement in crates/core/src/models/validation.rs: add variant WildcardDomainKeyword(String) directly after InvalidDomainKeyword (:14-15) with #[error(\"invalid domain keyword '{0}': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'\")]; in validate_domain_keyword (:128-133) append `if keyword.contains('*') { return Err(ValidationError::WildcardDomainKeyword(keyword.to_string())); }` after the existing whitespace branch; update the fn doc comment (keyword is a plain substring: non-empty, no whitespace, no '*'); extend the inline test_validate_domain_keyword table with (\"*.ru\", false) and (\"a*b\", false).",
        "Verify chunk A core: timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 (runs unit + both new integration files), then timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4 (import-skip consequence guard)."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "cargo check -p v2ray-rs-core && timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: no test-writer agent exists for this Rust stack — the integration/inline tests are the coder's first codeTasks. NO-TESTER-WAIVER: no tester agent exists for this Rust stack — the coder runs the chunk verify commands itself."
    },
    {
      "id": "stored-rule-recoverability",
      "taskIds": [
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "stored-rule-recoverability",
      "shard": "core-persistence",
      "pkgDirs": [
        "crates/core/src/persistence",
        "crates/core/tests"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/core/src/persistence/routing.rs",
          "symbol": "tests::test_routing_rules_save_load_roundtrip",
          "anchor": "fn test_routing_rules_save_load_roundtrip() {",
          "change": "Add persistence regression test verifying stored wildcard keyword rules (e.g. \"*.ru\") deserialize without error and round-trip through save_routing_rules and load_routing_rules without being modified."
        },
        {
          "task": "1.2",
          "file": "crates/core/tests/routing_rules_wildcard_roundtrip.rs",
          "symbol": "stored_wildcard_keyword_rule_loads_and_roundtrips",
          "anchor": "stored_wildcard_keyword_rule_loads_and_roundtrips",
          "change": "NEW integration test file creating: stored_wildcard_keyword_rule_loads_and_roundtrips",
          "new": true
        }
      ],
      "contract": {
        "states": [
          "stored_wildcard_loaded",
          "roundtrip_identical",
          "missing_file_empty"
        ],
        "transitions": [
          {
            "input": "load_routing_rules(paths) where routing-rules.json contains {\"type\":\"domain_keyword\",\"keyword\":\"*.ru\"}",
            "state": "stored_wildcard_loaded",
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/routing.rs:15-23 (serde-only load, no validation); routing-rules delta 'SHALL load without error and SHALL NOT be rewritten automatically'"
          },
          {
            "input": "save_routing_rules(paths, set holding DomainKeyword \"*.ru\") followed by load_routing_rules",
            "state": "roundtrip_identical",
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/routing.rs:6-13 (serde_json::to_string_pretty + atomic_write, no validation); RoutingRuleSet derives PartialEq (routing.rs:52) so the whole set is comparable"
          },
          {
            "input": "load_routing_rules(paths) with no routing-rules.json present",
            "state": "missing_file_empty",
            "effect": "clear",
            "evidence": "crates/core/src/persistence/routing.rs:17-19"
          }
        ],
        "forbidden": [
          "load or save returning an error for a stored wildcard keyword rule",
          "save serializing a transformed keyword (star stripped, keyword converted to a suffix/domain rule)",
          "any write to the file triggered by load (no auto-migration on open)"
        ],
        "seeding": [
          "AppPaths::from_paths(tempdir_path, tempdir_path) (crates/core/src/persistence/mod.rs:119; gated cfg(any(test, feature = \"test-utils\")) so the tests/ file must enable the feature) with a kept-alive tempfile::TempDir (regular dep of v2ray-rs-core, importable from tests/)",
          "getting the wildcard rule INTO the set only via RoutingRuleSet::add() (pub non-validating, routing.rs:62-64) or serde_json::from_str::<RoutingRuleSet> of the stored JSON shape; add_validated is FORBIDDEN for this state (seam core-keyword-validation rejects it)"
        ],
        "budgets": [
          "exactly one atomic write per save (tempfile + persist via crate::fs::atomic_write); the round-trip test must finish under 5s; chunk A overall bounded by timeout 5m, --test-threads=4"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TESTS FIRST: write crates/core/tests/routing_rules_wildcard_roundtrip.rs with first line #![cfg(feature = \"test-utils\")] (without the feature the file compiles empty, keeping plain cargo test green). Test stored_wildcard_keyword_rule_loads_and_roundtrips: let tmp = tempfile::TempDir::new().unwrap(); let paths = v2ray_rs_core::persistence::AppPaths::from_paths(tmp.path().to_path_buf(), tmp.path().to_path_buf()); seed with serde_json::from_str::<RoutingRuleSet>(\"{\\\"rules\\\":[{\\\"id\\\":\\\"00000000-0000-0000-0000-000000000001\\\",\\\"match_condition\\\":{\\\"type\\\":\\\"domain_keyword\\\",\\\"keyword\\\":\\\"*.ru\\\"},\\\"action\\\":\\\"direct\\\",\\\"enabled\\\":true}]}\") or equivalently a RoutingRule literal + RoutingRuleSet::add(); save_routing_rules then load_routing_rules -> assert_eq!(loaded, original_set) with keyword still exactly \"*.ru\"; save the loaded set again and reload once more to prove idempotence.",
        "No production change expected - persistence/routing.rs:6-23 is already validation-free by design. If the test fails, fix load/save to stay validation-free; never weaken the test or special-case the keyword.",
        "Verify: timeout 5m cargo test -p v2ray-rs-core --features test-utils --test routing_rules_wildcard_roundtrip -- --test-threads=4"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: no test-writer agent exists for this Rust stack — the integration/inline tests are the coder's first codeTasks. NO-TESTER-WAIVER: no tester agent exists for this Rust stack — the coder runs the chunk verify commands itself."
    },
    {
      "id": "routing-row-invalid-display",
      "taskIds": [
        "2.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "routing-row-invalid-display",
      "shard": "ui-routing",
      "pkgDirs": [
        "crates/ui/src/preferences"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/preferences/routing.rs",
          "symbol": "build_routing_rule_row",
          "anchor": ".subtitle(format_action(&rule.action))",
          "change": "Add pure row validation helper; in build_routing_rule_row, if rule validation fails, replace action subtitle with the validation error message and apply \"error\" CSS class to the row; add unit tests for pure validation helper."
        }
      ],
      "contract": {
        "states": [
          "row_clean",
          "row_invalid"
        ],
        "transitions": [
          {
            "input": "stored rule passing validate_rule_match (DomainKeyword \"sina\", GeoIp \"RU\")",
            "state": "row_clean",
            "effect": "clear",
            "evidence": "crates/ui/src/preferences/routing.rs:222-231 current row build: title format_match (:968-980), subtitle format_action (:960-965)"
          },
          {
            "input": "stored rule with DomainKeyword \"*.ru\"",
            "state": "row_invalid",
            "effect": "set",
            "evidence": "routing-rules delta 'SHALL mark each stored invalid rule with error styling and use the validation message as its subtitle until the user edits or deletes it'; design.md decision 3 (subtitle replaces the action subtitle); css+subtitle pattern network.rs:541-542"
          },
          {
            "input": "stored rule failing any other validate_rule_match check (DomainKeyword \"\", GeoSite \"Google\")",
            "state": "row_invalid",
            "effect": "set",
            "evidence": "helper delegates to validate_rule_match (crates/core/src/models/validation.rs:205-227) - generic by design, a superset of the spec's wildcard case (risk 3)"
          }
        ],
        "forbidden": [
          "row build mutating ctx.rule_set (render is read-only over &RoutingRule)",
          "an invalid row keeping the format_action subtitle, or a valid row receiving the error css class",
          "helper signature referencing gtk/adw types (must stay pure for headless inline tests) or composing its own message text instead of err.to_string() from core"
        ],
        "seeding": [
          "RoutingRule struct literals (all fields pub, crates/core/src/models/routing.rs:7-18) constructed directly inside the inline #[cfg(test)] mod - no RoutingRuleSet, no persistence, no dialog"
        ],
        "budgets": [
          "O(1) per row: exactly one validate_rule_match call per build_routing_rule_row; the full list render stays O(n) rows, no extra pass"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TESTS FIRST: append an inline #[cfg(test)] mod to crates/ui/src/preferences/routing.rs with rule_row_validation_error_flags_wildcard_keyword (rule literal with DomainKeyword \"*.ru\" -> Some(msg) where msg contains \"plain substring\" and \"Domain rule type\") and rule_row_validation_error_passes_valid_rules (DomainKeyword \"sina\" and GeoIp \"RU\" -> None). Plain tests, no crate::gtk_test::run needed.",
        "Implement private fn rule_row_validation_error(rule: &RoutingRule) -> Option<String> next to format_match (routing.rs:968): validate_rule_match(&rule.match_condition).err().map(|e| e.to_string()). In build_routing_rule_row (routing.rs:222-231): match on the helper - None => today's .subtitle(format_action(&rule.action)); Some(msg) => after building, row.set_subtitle(&msg) and row.add_css_class(\"error\") (pattern network.rs:541-542). Title stays format_match(...) so a stored row still reads 'Domain Keyword: *.ru'. No dialog changes.",
        "Verify: cargo check -p v2ray-rs-ui then timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "cargo check -p v2ray-rs-ui && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: no test-writer agent exists for this Rust stack — the integration/inline tests are the coder's first codeTasks. NO-TESTER-WAIVER: no tester agent exists for this Rust stack — the coder runs the chunk verify commands itself."
    },
    {
      "id": "dns-keyword-validation-ui",
      "taskIds": [
        "2.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "dns-keyword-validation-ui",
      "shard": "ui-dns",
      "pkgDirs": [
        "crates/ui/src/preferences",
        "crates/core/src/models"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "dns_rule_from_inputs",
          "anchor": "2 => DnsRuleMatch::DomainKeyword { keyword: value },",
          "change": "Validate DomainKeyword with validate_domain_keyword before returning DnsRule in dns_rule_from_inputs, blocking save and displaying error inline in the dialog."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "build_dns_page",
          "anchor": ".subtitle(format!(\"Server: {}\", rule.server_tag))",
          "change": "Add pure DNS rule validation helper; in DNS rules list row rendering, if validation fails, replace server subtitle with validation error and add \"error\" CSS class; add unit tests for the helper in mod tests."
        },
        {
          "task": "2.2",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "tests::test_dns_rule_match_domain_keyword_roundtrip",
          "anchor": "fn test_dns_rule_match_domain_keyword_roundtrip() {",
          "change": "Extend the existing round-trip test with a stored wildcard keyword case: DnsRuleMatch::DomainKeyword { keyword: \"*.cn\" } serializes/deserializes unchanged (proves the dns delta SHALL \"Stored DNS keyword rules with such a value SHALL stay unchanged\")."
        }
      ],
      "contract": {
        "states": [
          "input_rejected_inline",
          "input_saved",
          "stored_row_invalid",
          "stored_row_clean"
        ],
        "transitions": [
          {
            "input": "DNS rule dialog Match Type 'Domain Keyword' (combo index 2) with value \"*.ru\"",
            "state": "input_rejected_inline",
            "effect": "set",
            "evidence": "dns-preferences-ui delta 'SHALL NOT save a value containing * ... showing the validation message inline'; dns_rule_from_inputs arm 2 (dns.rs:941-963) returns Err(core Display); update_validation shows error_label + set_response_enabled(\"save\", false) (dns.rs:1663-1714; set_validation_message :886-894)"
          },
          {
            "input": "dialog value containing whitespace or empty (\"foo bar\", \"\")",
            "state": "input_rejected_inline",
            "effect": "set",
            "evidence": "dns-preferences-ui delta whitespace clause; core validate_domain_keyword rejects ANY whitespace char (crates/core/src/models/validation.rs:128-133); empty value already Err at dns.rs:946-948"
          },
          {
            "input": "dialog value \"sina\" with a resolvable server tag",
            "state": "input_saved",
            "effect": "clear",
            "evidence": "upsert_dns_rule runs only after validate() returns Ok (dns.rs:1698-1714); plain-keyword acceptance mirrors the routing delta's 'plain keyword accepted' scenario"
          },
          {
            "input": "stored DnsRuleMatch::DomainKeyword { keyword: \"*.ru\" } rendered by render_dns_rules",
            "state": "stored_row_invalid",
            "effect": "set",
            "evidence": "dns-preferences-ui delta 'Their rows SHALL be marked invalid with error styling and use the validation message as their subtitle'; row build dns.rs:1066-1070: title stays \"Domain Keyword: *.ru\", subtitle = validation message replacing \"Server: {tag}\", row.add_css_class(\"error\") (pattern network.rs:541-542)"
          },
          {
            "input": "stored DomainSuffix / GeoSite / DomainFull rule, or plain keyword \"sina\"",
            "state": "stored_row_clean",
            "effect": "clear",
            "evidence": "dns_rule_match_validation_error returns None for non-keyword arms; subtitle stays format!(\"Server: {}\", rule.server_tag) (dns.rs:1069)"
          }
        ],
        "forbidden": [
          "upsert_dns_rule running for a rejected value (BOTH guards must stay: Save disabled in update_validation AND the validate() re-check inside connect_response, dns.rs:1698-1707)",
          "rewriting or normalizing a stored keyword during render or when opening the edit dialog (value round-trips verbatim)",
          "the helper validating non-keyword variants or carrying its own message literal - text always comes from core ValidationError Display"
        ],
        "seeding": [
          "DnsRule { match_condition: DnsRuleMatch::..., server_tag: \"remote\".to_string() } literals inside the existing inline mod at dns.rs:1903; dialog-level behavior is asserted through dns_rule_from_inputs's Err/Ok values (pure-helper scope per task 2.2), not through gtk widgets"
        ],
        "budgets": [
          "O(1) per validation call and per row; render_dns_rules stays O(n) rules with no extra pass"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TESTS FIRST: append to the existing #[cfg(test)] mod in crates/ui/src/preferences/dns.rs (:1903): dns_rule_match_validation_error_rejects_wildcard_and_whitespace (DomainKeyword \"*.ru\" -> Some(msg containing \"plain substring\" and \"Domain rule type\"); DomainKeyword \"has space\" -> Some(_)) and dns_rule_match_validation_error_passes_plain_keyword_and_other_matches (DomainKeyword \"sina\" -> None; DomainSuffix \".ru\", GeoSite \"google\", DomainFull \"example.com\" -> None). Plain tests, no gtk.",
        "Extend crates/core/src/models/dns.rs test_dns_rule_match_domain_keyword_roundtrip with a wildcard stored case: DnsRuleMatch::DomainKeyword { keyword: \"*.cn\".to_string() } survives a serde round trip byte-identically (derive PartialEq compare) — no production change, regression proof only.",
        "Implement private fn dns_rule_match_validation_error(m: &DnsRuleMatch) -> Option<String> next to dns_rule_from_inputs (dns.rs:941): match m { DnsRuleMatch::DomainKeyword { keyword } => validate_domain_keyword(keyword).err().map(|e| e.to_string()), _ => None }. Wire twice: (a) in dns_rule_from_inputs after constructing match_condition, `if let Some(msg) = dns_rule_match_validation_error(&match_condition) { return Err(msg); }` - the existing dialog machinery then shows the inline error and disables Save; (b) in render_dns_rules row build (dns.rs:1066-1070) - Some(msg) => row.add_css_class(\"error\") and subtitle = msg replacing the Server subtitle; None => unchanged row.",
        "Verify (chunk B close-out): cargo check -p v2ray-rs-ui then timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "cargo check -p v2ray-rs-ui && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: no test-writer agent exists for this Rust stack — the integration/inline tests are the coder's first codeTasks. NO-TESTER-WAIVER: no tester agent exists for this Rust stack — the coder runs the chunk verify commands itself."
    }
  ],
  "seams": [
    {
      "id": "core-keyword-validation",
      "tasks": [
        "1.1"
      ],
      "summary": "Chunk A / 2 tasks total (1.1 here, 1.2 in seam stored-rule-recoverability); package v2ray-rs-core; runs FIRST - chunk B (ui) depends on this seam's behavior. Sites, i.e. every test file the coder may add or change in chunk A: crates/core/tests/domain_keyword_validation.rs (NEW, required), crates/core/tests/routing_rules_wildcard_roundtrip.rs (NEW, required, gated), crates/core/src/models/validation.rs (extend inline test_validate_domain_keyword table), crates/core/src/persistence/routing.rs (inline tests optional). NO-RED-WAIVER: no test-writer agent exists for this Rust stack - the integration tests above are written by the coder as the FIRST codeTasks, not sealed red tests. NO-TESTER-WAIVER: no tester agent exists for this Rust stack - the coder runs the chunk verify commands itself. Exact identifiers: new enum variant ValidationError::WildcardDomainKeyword(String) placed directly after InvalidDomainKeyword (validation.rs:14-15), thiserror attribute #[error(\"invalid domain keyword '{0}': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'\")]; exact Display for input *.ru: invalid domain keyword '*.ru': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'. ValidationError already derives Debug+Error+PartialEq (validation.rs:6) so assert_eq! works. Whitespace/empty rejection already exists (validation.rs:128-133, rejects ANY whitespace char) and already satisfies the DNS delta's whitespace clause - do not add a second rule. Tasks 3.1/3.2 are the verify layer: literal commands live in verifyCommands/fullFloor. Decision recorded: subscription JSON import stays untouched; its push_rule (json_import.rs:506-524) validates via validate_rule_match, so wildcard keyword entries become skipped-with-reason on import - accepted consequence (risk 1); DNS import path (json_import.rs:691-715) does not validate keywords and stays as-is (risk 6).",
      "contract": {
        "states": [
          "accepted",
          "rejected_whitespace_or_empty",
          "rejected_wildcard"
        ],
        "transitions": [
          {
            "input": "empty string or any keyword containing a whitespace char (\"\", \" \", \"has space\")",
            "state": "rejected_whitespace_or_empty",
            "effect": "set",
            "evidence": "crates/core/src/models/validation.rs:128-133 (existing check, unchanged; covers the dns-preferences-ui delta's whitespace clause)"
          },
          {
            "input": "non-blank keyword containing '*' anywhere (\"*.ru\", \"*\", \"a*b\", \"sina*\")",
            "state": "rejected_wildcard",
            "effect": "set",
            "evidence": "openspec routing-rules delta Requirement 'Keyword rules reject wildcards'; design.md decision 1; new check appended in validate_domain_keyword (validation.rs:128-133)"
          },
          {
            "input": "keyword with BOTH whitespace and '*' (\" * \")",
            "state": "rejected_whitespace_or_empty",
            "effect": "set",
            "evidence": "check order in validate_domain_keyword: whitespace test first (validation.rs:129), '*' test added after it - precedence is fixed by insertion point"
          },
          {
            "input": "plain keyword: non-empty, no whitespace, no '*' (\"sina\", \"sina.com\", \".example\")",
            "state": "accepted",
            "effect": "clear",
            "evidence": "routing-rules delta scenario 'plain keyword accepted (sina stored)'"
          },
          {
            "input": "RoutingRuleSet::add_validated(rule with DomainKeyword \"*.ru\")",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:111-115 gates via validate_rule_match; Err returned, rule NOT pushed, rules().len() unchanged"
          },
          {
            "input": "RoutingRuleSet::add_at(i, rule with DomainKeyword \"*.ru\")",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:117-126 (validate before insert)"
          },
          {
            "input": "RoutingRuleSet::edit_rule(id, Some(DomainKeyword \"*.ru\"), _)",
            "state": "rejected_wildcard",
            "effect": "no-op",
            "evidence": "crates/core/src/models/routing.rs:128-149 (validate_rule_match before assigning match_condition)"
          }
        ],
        "forbidden": [
          "validate_domain_keyword returning Ok(()) for any input containing '*'",
          "add_validated/add_at/edit_rule persisting a wildcard keyword rule or leaving a partially applied edit on Err",
          "WildcardDomainKeyword returned for inputs with no '*' (empty/whitespace keep InvalidDomainKeyword, validation.rs:14-15)",
          "any caller (UI helper, test) hand-formatting the message string - the exact text lives only in the thiserror attribute and is consumed via Display"
        ],
        "seeding": [
          "validator states: call pub v2ray_rs_core::models::validate_domain_keyword and validate_rule_match directly (re-exported at crates/core/src/models/mod.rs:44-48)",
          "mutation-gate states: RoutingRuleSet::new() + RoutingRule struct literal (all fields pub, routing.rs:7-18) + add_validated/add_at/edit_rule - the only pub path; the rules field is private so field-write seeding is impossible"
        ],
        "budgets": [
          "validation is a single pass, O(len(keyword)), no allocation on the Ok path (one String only on Err)",
          "chunk A test run bounded by timeout 5m with --test-threads=4"
        ]
      },
      "codeTasks": [
        "TESTS FIRST: write crates/core/tests/domain_keyword_validation.rs (new plain integration test, no feature gate; import via v2ray_rs_core::models::{...}, precedent crates/core/tests/singbox_check.rs:7-10). Tests: wildcard_keyword_is_rejected - assert_eq!(validate_domain_keyword(\"*.ru\"), Err(ValidationError::WildcardDomainKeyword(\"*.ru\".to_string()))) plus exact Display equality with \"invalid domain keyword '*.ru': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'\"; also \"*\", \"a*b\", \"sina*\" each Err(WildcardDomainKeyword(_)); plain_keyword_is_accepted - \"sina\", \"sina.com\", \".example\" -> Ok(()); rule_match_dispatch_rejects_wildcard_keyword - validate_rule_match(&RuleMatch::DomainKeyword { keyword: \"*.ru\".into() }) is Err; validated_mutations_reject_wildcard_keyword - fresh RoutingRuleSet, then add_validated, add_at(0, ..), edit_rule(..) each with a wildcard-keyword rule return Err(WildcardDomainKeyword(_)) and leave rules().len() == 0.",
        "Implement in crates/core/src/models/validation.rs: add variant WildcardDomainKeyword(String) directly after InvalidDomainKeyword (:14-15) with #[error(\"invalid domain keyword '{0}': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'\")]; in validate_domain_keyword (:128-133) append `if keyword.contains('*') { return Err(ValidationError::WildcardDomainKeyword(keyword.to_string())); }` after the existing whitespace branch; update the fn doc comment (keyword is a plain substring: non-empty, no whitespace, no '*'); extend the inline test_validate_domain_keyword table with (\"*.ru\", false) and (\"a*b\", false).",
        "Verify chunk A core: timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 (runs unit + both new integration files), then timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4 (import-skip consequence guard)."
      ],
      "entry": [
        "validate_domain_keyword",
        "validate_rule_match",
        "ValidationError",
        "RoutingRuleSet::add_validated",
        "RoutingRuleSet::add_at",
        "RoutingRuleSet::edit_rule"
      ]
    },
    {
      "id": "stored-rule-recoverability",
      "tasks": [
        "1.2"
      ],
      "summary": "Chunk A, task 1.2 (same chunk/package as seam core-keyword-validation; same sites list applies - routing_rules_wildcard_roundtrip.rs is this seam's required test file). Regression-only: persistence load/save are ALREADY validation-free (persistence/routing.rs:6-23, serde + atomic_write), so no production change is expected - the test proves stored wildcard keyword rules load unchanged and survive a save/load round trip byte-identically. NO-RED-WAIVER: no test-writer agent exists for this Rust stack - the round-trip test is written by the coder as the first codeTask of this seam. NO-TESTER-WAIVER: no tester agent exists for this Rust stack - the coder runs the feature-enabled core test command itself.",
      "contract": {
        "states": [
          "stored_wildcard_loaded",
          "roundtrip_identical",
          "missing_file_empty"
        ],
        "transitions": [
          {
            "input": "load_routing_rules(paths) where routing-rules.json contains {\"type\":\"domain_keyword\",\"keyword\":\"*.ru\"}",
            "state": "stored_wildcard_loaded",
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/routing.rs:15-23 (serde-only load, no validation); routing-rules delta 'SHALL load without error and SHALL NOT be rewritten automatically'"
          },
          {
            "input": "save_routing_rules(paths, set holding DomainKeyword \"*.ru\") followed by load_routing_rules",
            "state": "roundtrip_identical",
            "effect": "no-op",
            "evidence": "crates/core/src/persistence/routing.rs:6-13 (serde_json::to_string_pretty + atomic_write, no validation); RoutingRuleSet derives PartialEq (routing.rs:52) so the whole set is comparable"
          },
          {
            "input": "load_routing_rules(paths) with no routing-rules.json present",
            "state": "missing_file_empty",
            "effect": "clear",
            "evidence": "crates/core/src/persistence/routing.rs:17-19"
          }
        ],
        "forbidden": [
          "load or save returning an error for a stored wildcard keyword rule",
          "save serializing a transformed keyword (star stripped, keyword converted to a suffix/domain rule)",
          "any write to the file triggered by load (no auto-migration on open)"
        ],
        "seeding": [
          "AppPaths::from_paths(tempdir_path, tempdir_path) (crates/core/src/persistence/mod.rs:119; gated cfg(any(test, feature = \"test-utils\")) so the tests/ file must enable the feature) with a kept-alive tempfile::TempDir (regular dep of v2ray-rs-core, importable from tests/)",
          "getting the wildcard rule INTO the set only via RoutingRuleSet::add() (pub non-validating, routing.rs:62-64) or serde_json::from_str::<RoutingRuleSet> of the stored JSON shape; add_validated is FORBIDDEN for this state (seam core-keyword-validation rejects it)"
        ],
        "budgets": [
          "exactly one atomic write per save (tempfile + persist via crate::fs::atomic_write); the round-trip test must finish under 5s; chunk A overall bounded by timeout 5m, --test-threads=4"
        ]
      },
      "codeTasks": [
        "TESTS FIRST: write crates/core/tests/routing_rules_wildcard_roundtrip.rs with first line #![cfg(feature = \"test-utils\")] (without the feature the file compiles empty, keeping plain cargo test green). Test stored_wildcard_keyword_rule_loads_and_roundtrips: let tmp = tempfile::TempDir::new().unwrap(); let paths = v2ray_rs_core::persistence::AppPaths::from_paths(tmp.path().to_path_buf(), tmp.path().to_path_buf()); seed with serde_json::from_str::<RoutingRuleSet>(\"{\\\"rules\\\":[{\\\"id\\\":\\\"00000000-0000-0000-0000-000000000001\\\",\\\"match_condition\\\":{\\\"type\\\":\\\"domain_keyword\\\",\\\"keyword\\\":\\\"*.ru\\\"},\\\"action\\\":\\\"direct\\\",\\\"enabled\\\":true}]}\") or equivalently a RoutingRule literal + RoutingRuleSet::add(); save_routing_rules then load_routing_rules -> assert_eq!(loaded, original_set) with keyword still exactly \"*.ru\"; save the loaded set again and reload once more to prove idempotence.",
        "No production change expected - persistence/routing.rs:6-23 is already validation-free by design. If the test fails, fix load/save to stay validation-free; never weaken the test or special-case the keyword.",
        "Verify: timeout 5m cargo test -p v2ray-rs-core --features test-utils --test routing_rules_wildcard_roundtrip -- --test-threads=4"
      ],
      "entry": [
        "save_routing_rules",
        "load_routing_rules",
        "AppPaths"
      ]
    },
    {
      "id": "routing-row-invalid-display",
      "tasks": [
        "2.1"
      ],
      "summary": "Chunk B (tasks 2.1+2.2; package v2ray-rs-ui; prev: chunk A - this is a behavior dependency, not sharedPkg: the helper tests below assert core's new WildcardDomainKeyword rejection and exact Display, so chunk B starts only after chunk A merges; the two chunks touch disjoint files). Site, i.e. every test file the coder may add or change for this task: crates/ui/src/preferences/routing.rs (new inline #[cfg(test)] mod + helper + build_routing_rule_row change). NOT an entry seam: the helper is a private pure fn consumed in-module by build_routing_rule_row (routing.rs:222-231); the contract a test can observe is the helper's return value, so site tests are inline per the existing dns.rs:1903 precedent - there is no separately loaded file to name, and full row rendering is out of the task's pure-helper test scope. NO-RED-WAIVER: no test-writer agent exists for this Rust stack - inline tests are the coder's first codeTask. NO-TESTER-WAIVER: no tester agent exists for this Rust stack - the coder runs the ui chunk verify commands. Exact identifiers: fn rule_row_validation_error(rule: &RoutingRule) -> Option<String> (private, pure, NO gtk/adw types so tests run headless), placed next to format_match (routing.rs:968-980); row marking uses row.add_css_class(\"error\") + subtitle text, the exact pattern already shipped at crates/ui/src/preferences/network.rs:541-542 and tun.rs:641-643. The add/edit dialog needs NO change: it already validates every constructed match via validate_rule_match and shows e.to_string() inline (routing.rs:751-756, value_entry.add_css_class(\"error\") + error_label), so wildcard rejection there is inherited from seam core-keyword-validation; dialog value trim already happens at routing.rs:744-747.",
      "contract": {
        "states": [
          "row_clean",
          "row_invalid"
        ],
        "transitions": [
          {
            "input": "stored rule passing validate_rule_match (DomainKeyword \"sina\", GeoIp \"RU\")",
            "state": "row_clean",
            "effect": "clear",
            "evidence": "crates/ui/src/preferences/routing.rs:222-231 current row build: title format_match (:968-980), subtitle format_action (:960-965)"
          },
          {
            "input": "stored rule with DomainKeyword \"*.ru\"",
            "state": "row_invalid",
            "effect": "set",
            "evidence": "routing-rules delta 'SHALL mark each stored invalid rule with error styling and use the validation message as its subtitle until the user edits or deletes it'; design.md decision 3 (subtitle replaces the action subtitle); css+subtitle pattern network.rs:541-542"
          },
          {
            "input": "stored rule failing any other validate_rule_match check (DomainKeyword \"\", GeoSite \"Google\")",
            "state": "row_invalid",
            "effect": "set",
            "evidence": "helper delegates to validate_rule_match (crates/core/src/models/validation.rs:205-227) - generic by design, a superset of the spec's wildcard case (risk 3)"
          }
        ],
        "forbidden": [
          "row build mutating ctx.rule_set (render is read-only over &RoutingRule)",
          "an invalid row keeping the format_action subtitle, or a valid row receiving the error css class",
          "helper signature referencing gtk/adw types (must stay pure for headless inline tests) or composing its own message text instead of err.to_string() from core"
        ],
        "seeding": [
          "RoutingRule struct literals (all fields pub, crates/core/src/models/routing.rs:7-18) constructed directly inside the inline #[cfg(test)] mod - no RoutingRuleSet, no persistence, no dialog"
        ],
        "budgets": [
          "O(1) per row: exactly one validate_rule_match call per build_routing_rule_row; the full list render stays O(n) rows, no extra pass"
        ]
      },
      "codeTasks": [
        "TESTS FIRST: append an inline #[cfg(test)] mod to crates/ui/src/preferences/routing.rs with rule_row_validation_error_flags_wildcard_keyword (rule literal with DomainKeyword \"*.ru\" -> Some(msg) where msg contains \"plain substring\" and \"Domain rule type\") and rule_row_validation_error_passes_valid_rules (DomainKeyword \"sina\" and GeoIp \"RU\" -> None). Plain tests, no crate::gtk_test::run needed.",
        "Implement private fn rule_row_validation_error(rule: &RoutingRule) -> Option<String> next to format_match (routing.rs:968): validate_rule_match(&rule.match_condition).err().map(|e| e.to_string()). In build_routing_rule_row (routing.rs:222-231): match on the helper - None => today's .subtitle(format_action(&rule.action)); Some(msg) => after building, row.set_subtitle(&msg) and row.add_css_class(\"error\") (pattern network.rs:541-542). Title stays format_match(...) so a stored row still reads 'Domain Keyword: *.ru'. No dialog changes.",
        "Verify: cargo check -p v2ray-rs-ui then timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4"
      ]
    },
    {
      "id": "dns-keyword-validation-ui",
      "tasks": [
        "2.2"
      ],
      "summary": "Chunk B, task 2.2 (same chunk/package as seam routing-row-invalid-display; same serial-after-chunk-A constraint). Site, i.e. every test file the coder may add or change for this task: crates/ui/src/preferences/dns.rs (helper + dns_rule_from_inputs arm + render_dns_rules row build + tests appended to the EXISTING inline mod at dns.rs:1903). NOT an entry seam (same reasoning as routing-row-invalid-display: private pure helper, inline tests per dns.rs:1903 precedent). NO-RED-WAIVER: no test-writer agent exists for this Rust stack - inline tests are the coder's first codeTask. NO-TESTER-WAIVER: no tester agent exists for this Rust stack - the coder runs the ui chunk verify commands. Exact identifiers: fn dns_rule_match_validation_error(m: &DnsRuleMatch) -> Option<String> (private, pure, no gtk types) placed next to dns_rule_from_inputs (dns.rs:941-963); reuses core v2ray_rs_core::models::validate_domain_keyword per design.md decision 2. Dialog rejection rides EXISTING machinery with zero new widgets: dns_rule_from_inputs returns Err(message), and show_dns_rule_dialog's validate/update_validation closures already render it inline via set_validation_message (dns.rs:886-894) and disable Save via dialog.set_response_enabled(\"save\", false) (wiring at dns.rs:1663-1714); the save handler re-runs validate() before upsert, so a rejected value can never be saved. Row marking mirrors seam routing-row-invalid-display with the 'error' css class + subtitle replacing format!(\"Server: {}\", rule.server_tag) (row build dns.rs:1066-1070).",
      "contract": {
        "states": [
          "input_rejected_inline",
          "input_saved",
          "stored_row_invalid",
          "stored_row_clean"
        ],
        "transitions": [
          {
            "input": "DNS rule dialog Match Type 'Domain Keyword' (combo index 2) with value \"*.ru\"",
            "state": "input_rejected_inline",
            "effect": "set",
            "evidence": "dns-preferences-ui delta 'SHALL NOT save a value containing * ... showing the validation message inline'; dns_rule_from_inputs arm 2 (dns.rs:941-963) returns Err(core Display); update_validation shows error_label + set_response_enabled(\"save\", false) (dns.rs:1663-1714; set_validation_message :886-894)"
          },
          {
            "input": "dialog value containing whitespace or empty (\"foo bar\", \"\")",
            "state": "input_rejected_inline",
            "effect": "set",
            "evidence": "dns-preferences-ui delta whitespace clause; core validate_domain_keyword rejects ANY whitespace char (crates/core/src/models/validation.rs:128-133); empty value already Err at dns.rs:946-948"
          },
          {
            "input": "dialog value \"sina\" with a resolvable server tag",
            "state": "input_saved",
            "effect": "clear",
            "evidence": "upsert_dns_rule runs only after validate() returns Ok (dns.rs:1698-1714); plain-keyword acceptance mirrors the routing delta's 'plain keyword accepted' scenario"
          },
          {
            "input": "stored DnsRuleMatch::DomainKeyword { keyword: \"*.ru\" } rendered by render_dns_rules",
            "state": "stored_row_invalid",
            "effect": "set",
            "evidence": "dns-preferences-ui delta 'Their rows SHALL be marked invalid with error styling and use the validation message as their subtitle'; row build dns.rs:1066-1070: title stays \"Domain Keyword: *.ru\", subtitle = validation message replacing \"Server: {tag}\", row.add_css_class(\"error\") (pattern network.rs:541-542)"
          },
          {
            "input": "stored DomainSuffix / GeoSite / DomainFull rule, or plain keyword \"sina\"",
            "state": "stored_row_clean",
            "effect": "clear",
            "evidence": "dns_rule_match_validation_error returns None for non-keyword arms; subtitle stays format!(\"Server: {}\", rule.server_tag) (dns.rs:1069)"
          }
        ],
        "forbidden": [
          "upsert_dns_rule running for a rejected value (BOTH guards must stay: Save disabled in update_validation AND the validate() re-check inside connect_response, dns.rs:1698-1707)",
          "rewriting or normalizing a stored keyword during render or when opening the edit dialog (value round-trips verbatim)",
          "the helper validating non-keyword variants or carrying its own message literal - text always comes from core ValidationError Display"
        ],
        "seeding": [
          "DnsRule { match_condition: DnsRuleMatch::..., server_tag: \"remote\".to_string() } literals inside the existing inline mod at dns.rs:1903; dialog-level behavior is asserted through dns_rule_from_inputs's Err/Ok values (pure-helper scope per task 2.2), not through gtk widgets"
        ],
        "budgets": [
          "O(1) per validation call and per row; render_dns_rules stays O(n) rules with no extra pass"
        ]
      },
      "codeTasks": [
        "TESTS FIRST: append to the existing #[cfg(test)] mod in crates/ui/src/preferences/dns.rs (:1903): dns_rule_match_validation_error_rejects_wildcard_and_whitespace (DomainKeyword \"*.ru\" -> Some(msg containing \"plain substring\" and \"Domain rule type\"); DomainKeyword \"has space\" -> Some(_)) and dns_rule_match_validation_error_passes_plain_keyword_and_other_matches (DomainKeyword \"sina\" -> None; DomainSuffix \".ru\", GeoSite \"google\", DomainFull \"example.com\" -> None). Plain tests, no gtk.",
        "Extend crates/core/src/models/dns.rs test_dns_rule_match_domain_keyword_roundtrip with a wildcard stored case: DnsRuleMatch::DomainKeyword { keyword: \"*.cn\".to_string() } survives a serde round trip byte-identically (derive PartialEq compare) — no production change, regression proof only.",
        "Implement private fn dns_rule_match_validation_error(m: &DnsRuleMatch) -> Option<String> next to dns_rule_from_inputs (dns.rs:941): match m { DnsRuleMatch::DomainKeyword { keyword } => validate_domain_keyword(keyword).err().map(|e| e.to_string()), _ => None }. Wire twice: (a) in dns_rule_from_inputs after constructing match_condition, `if let Some(msg) = dns_rule_match_validation_error(&match_condition) { return Err(msg); }` - the existing dialog machinery then shows the inline error and disables Save; (b) in render_dns_rules row build (dns.rs:1066-1070) - Some(msg) => row.add_css_class(\"error\") and subtitle = msg replacing the Server subtitle; None => unchanged row.",
        "Verify (chunk B close-out): cargo check -p v2ray-rs-ui then timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The system SHALL reject a domain keyword rule whose value contains `*`, with an error stating that a keyword is a plain substring and pointing users to the Domain rule type for wildcard suffixes.",
      "tests": [
        "domain_keyword_validation::wildcard_keyword_is_rejected",
        "domain_keyword_validation::rule_match_dispatch_rejects_wildcard_keyword",
        "domain_keyword_validation::validated_mutations_reject_wildcard_keyword",
        "models::validation::tests::test_validate_domain_keyword"
      ]
    },
    {
      "shall": "Rules already stored with such a keyword SHALL load without error and SHALL NOT be rewritten automatically.",
      "tests": [
        "routing_rules_wildcard_roundtrip::stored_wildcard_keyword_rule_loads_and_roundtrips"
      ]
    },
    {
      "shall": "The routing rule list SHALL mark each stored invalid rule with error styling and use the validation message as its subtitle until the user edits or deletes it.",
      "tests": [
        "preferences::routing::tests::rule_row_validation_error_flags_wildcard_keyword",
        "preferences::routing::tests::rule_row_validation_error_passes_valid_rules"
      ]
    },
    {
      "shall": "The DNS rule dialog SHALL validate a Domain Keyword value with the same rule as routing keyword rules and SHALL NOT save a value containing `*` or whitespace, showing the validation message inline.",
      "tests": [
        "preferences::dns::tests::dns_rule_match_validation_error_rejects_wildcard_and_whitespace",
        "preferences::dns::tests::dns_rule_match_validation_error_passes_plain_keyword_and_other_matches"
      ]
    },
    {
      "shall": "Stored DNS keyword rules with such a value SHALL stay unchanged.",
      "tests": [
        "models::dns::tests::test_dns_rule_match_domain_keyword_roundtrip"
      ]
    },
    {
      "shall": "Their rows SHALL be marked invalid with error styling and use the validation message as their subtitle.",
      "tests": [
        "preferences::dns::tests::dns_rule_match_validation_error_rejects_wildcard_and_whitespace"
      ]
    },
    {
      "shall": "the system SHALL reject the rule and SHALL NOT persist it",
      "tests": [
        "domain_keyword_validation::validated_mutations_reject_wildcard_keyword"
      ]
    },
    {
      "shall": "the rules SHALL load, the stored value SHALL stay `*.ru`, and its row SHALL show the validation error as its subtitle",
      "tests": [
        "routing_rules_wildcard_roundtrip::stored_wildcard_keyword_rule_loads_and_roundtrips",
        "preferences::routing::tests::rule_row_validation_error_flags_wildcard_keyword"
      ]
    },
    {
      "shall": "the system SHALL store the rule",
      "tests": [
        "domain_keyword_validation::plain_keyword_is_accepted",
        "preferences::routing::tests::rule_row_validation_error_passes_valid_rules"
      ]
    },
    {
      "shall": "the dialog SHALL show an inline error and the rule SHALL NOT be saved",
      "tests": [
        "preferences::dns::tests::dns_rule_match_validation_error_rejects_wildcard_and_whitespace"
      ]
    },
    {
      "shall": "the settings SHALL load unchanged and the rule's row SHALL show the validation error as its subtitle",
      "tests": [
        "models::dns::tests::test_dns_rule_match_domain_keyword_roundtrip",
        "preferences::dns::tests::dns_rule_match_validation_error_rejects_wildcard_and_whitespace"
      ]
    }
  ],
  "testHarness": [
    "test_validate_domain_keyword — crates/core/src/models/validation.rs:364 — table-driven input test runner for validate_domain_keyword",
    "make_rule — crates/core/src/models/routing.rs:144 — builds a RoutingRule with random Uuid and specified action for testing rule set mutations",
    "test_edit_rule_invalid_match — crates/core/src/models/routing.rs:519 — test harness verifying edit_rule rejection on invalid RuleMatch",
    "super::super::test_paths() — crates/core/src/persistence/routing.rs:32 (crates/core/src/persistence/mod.rs:163) — creates isolated temp AppPaths for persistence round-trip tests",
    "test_routing_rules_save_load_roundtrip — crates/core/src/persistence/routing.rs:31 — round-trip save and load test for RoutingRuleSet JSON persistence",
    "mod tests — crates/ui/src/preferences/dns.rs:1904 — test module with examples of both GTK-harnessed (gtk_test::run) and pure unit tests (test_strategy_family_note_per_backend, test_private_dns_warning)",
    "test_dns_rule_match_domain_keyword_roundtrip — crates/core/src/models/dns.rs:949 — serde round-trip of DnsRuleMatch (extend with *.cn stored case)"
  ],
  "floor": "timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 && timeout 10m cargo test --workspace -- --test-threads=4",
  "planReview": {
    "verdict": "pass",
    "rounds": 2
  }
}
```
