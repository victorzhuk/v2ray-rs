# Design

## Context

See `proposal.md` for motivation. Three modules currently duplicate or split domain-matcher checks and Effective DNS scoping:

- `crates/core/src/models/validation.rs` validates `RuleMatch::DomainFull` with `validate_domain_pattern`, which permits a leading `*.` intended only for suffix rules (`RuleMatch::Domain`). Generators emit `DomainFull` verbatim (`full:<domain>` on v2ray/xray, `domain: [<domain>]` on sing-box), where `*` never acts as a wildcard.
- `DnsRuleMatch` validation is split across three sites: `dns_rule_match_error` in `crates/core/src/models/dns.rs` (rejects `*.` on `DomainSuffix` and allows `*.` on `DomainFull`), `dns_rule_match_valid` in `crates/subscription/src/json_import.rs` (same mismatch), and `dns_rule_match_validation_error` in `crates/ui/src/preferences/dns/rules.rs` (checks only `DomainKeyword`, leaving stored invalid `GeoSite`, `DomainSuffix`, and `DomainFull` rows unbadged and skipping inline matcher checks before `DnsConfig::validate`).
- `crates/core/src/config/effective_dns.rs` duplicates `strip_suffix_wildcard`, strips matcher prefixes in `server_scope_singbox`, omits `canonical_rule_set_tag` filtering on `sing-box` GeoSite scopes, and omits TUN `exclude_domains` on non-final `sing-box` split-horizon servers.

## Goals / Non-Goals

**Goals:**
- Single-source exact-domain, suffix-domain, and `DnsRuleMatch` validation in `v2ray-rs-core::models::validation` and reuse it in `DnsConfig::validate`, `json_import`, and DNS preferences UI.
- Preserve existing persisted rules and TOML/JSON deserialization without silent rewrites, surfacing invalid stored rules via row error styling and validation subtitles.
- Align `EffectiveDns` domain scope tokens and `sing-box` split-horizon exclusions with actual generator behavior while deduplicating `strip_suffix_wildcard`.

**Non-Goals:**
- Changing backend JSON emission for valid routing, DNS, or TUN exclusion rules (already covered by `normalize-domain-suffix-emission`).
- Rewriting or migrating legacy stored `.cn` or `*.example.com` rules on disk.

## Decisions

- **Dedicated `validate_domain_full` and `validate_domain_suffix` in `models::validation`.**
  - `validate_domain_full(domain)` rejects `*` first with `ValidationError::WildcardDomainFull` (ASD-STE100 compliant message directing users to `Domain` or `Domain Suffix`), then validates non-empty multi-label hostname syntax via `validate_domain_pattern(domain)`. Alternative considered: reusing `valid_host_domain` from `dns.rs`; rejected because `valid_host_domain` allows single-label hostnames like `localhost` and underscores, whereas routing/DNS `DomainFull` has always required dotted domain labels matching `validate_domain_pattern` minus wildcards.
  - `validate_domain_suffix(suffix)` strips at most one leading `*.` (`let rest = suffix.strip_prefix("*.").unwrap_or(suffix)`), then requires `!rest.is_empty()`, `!rest.starts_with('.')`, `!rest.ends_with('.')`, no empty `..` labels, and ASCII alphanumeric or `-` characters per label, returning `ValidationError::InvalidDomainSuffix`. Unlike `validate_domain_pattern`, it continues to accept single-label TLD suffixes (`cn`, `ru`, `*.ru`) used in custom DNS rules (`dns.rs`, `effective_dns.rs`).
  - `validate_dns_rule_match(m: &DnsRuleMatch) -> Result<(), ValidationError>` dispatches all four variants (`GeoSite` -> `validate_geosite_category`, `DomainSuffix` -> `validate_domain_suffix`, `DomainKeyword` -> `validate_domain_keyword`, `DomainFull` -> `validate_domain_full`). `dns.rs`, `json_import.rs`, and `crates/ui/src/preferences/dns/rules.rs` all call it directly, deleting `dns_rule_match_error`, `valid_domain_suffix`, and `dns_rule_match_valid`.

- **Consistent matcher prefixes in `EffectiveDns` scopes.**
  - Both `v2ray`/`xray` (`user_dns_entries`, `collect_routing_domains`) and `sing-box` (`server_scope_singbox`) format domain scope items as `domain:<stripped_suffix>`, `keyword:<keyword>`, and `full:<domain>`. GeoSite items remain `geosite:<category>` on `v2ray`/`xray` and `canonical_rule_set_tag(&format!("geosite-{category}"))` on `sing-box` (matching `singbox.rs::dns_geosite_tag`, so negated categories like `geolocation-!cn` omitted from `sing-box` `dns.rules` are also omitted from `EffectiveDns`).
  - Alternative considered: keeping unprefixed `<keyword>` on `v2ray`/`xray`; rejected because valid keywords can contain dots (`sina.com`), which collide visually with bare bootstrap/static hostnames (`example.com`, `router.local`) in `Effective DNS` rows and `backend.log`.
  - In `singbox_dns`, `server_scope_singbox` takes `split_horizon_tag: Option<&str>` (from `plan.split_horizon_server()`): when `settings.tun.enabled` and `Some(tag) == split_horizon_tag`, `settings.tun.exclude_domains` are prepended as `domain:<stripped>` before custom/derived rules (matching `singbox.rs:570-575`), while `Some(tag) == final_tag` still collapses scope to `DnsScope::All`.
  - Delete private `effective_dns::strip_suffix_wildcard` and import `super::common::strip_suffix_wildcard`.

## Risks / Trade-offs

- [Existing unit test `dns_rule_match_validation_error_passes_plain_keyword_and_other_matches` in `crates/ui/src/preferences/dns/mod.rs:1167` passes `suffix: ".ru"`] -> That test asserted the old stub behavior (`_ => None`) where `dns_rule_match_validation_error` ignored `DomainSuffix`; update it to assert `ru` and `*.example.com` pass while `.ru`, `GOOGLE`, and `DomainFull("*.example.com")` return validation errors.
- [Stored legacy `.google.com` or `.cn` DNS suffixes in user `settings.toml`] -> Deserialization remains validation-free (`test_dns_config_toml_deserialization` stays green), generators still emit stored rules unchanged until edited, and `render_dns_rules` now visibly badges the row so the user can see why saving DNS settings requires removing the leading dot.
