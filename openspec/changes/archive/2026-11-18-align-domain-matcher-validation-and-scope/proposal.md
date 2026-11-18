# Proposal

## Why

Splitting `fix-domain-matcher-emission` normalized suffix emission and rejected wildcard keywords, leaving three domain-matcher validation and Effective DNS scope gaps across core models, JSON import, and DNS preferences UI. Exact-match `DomainFull` rules still accept `*.example.com` and emit dead literal `*`, custom DNS `DomainSuffix` rules reject `*.example.com` while DNS rule UI only badges invalid keywords, and `sing-box` Effective DNS collapses suffix, keyword, and full-domain scopes into unprefixed strings.

## What Changes

- Reject `*` in `RuleMatch::DomainFull` and `DnsRuleMatch::DomainFull` across core validation, JSON profile import, and preference dialogs with guidance to use suffix matchers, while preserving stored invalid rules on load and surfacing validation errors on their rows.
- Unify `DnsRuleMatch` validation in `v2ray-rs-core` so `DomainSuffix` accepts an optional single leading `*.` alongside plain multi-label and single-label TLD suffixes (`example.com`, `ru`) while rejecting leading dots, `json_import` reuses the same validator, and DNS preference dialogs and stored rows validate and badge all four `DnsRuleMatch` variants regardless of the DNS master toggle.
- Preserve matcher kind in `EffectiveDns` domain scopes (`domain:<suffix>`, `keyword:<keyword>`, `full:<domain>`, plus valid backend GeoSite tags and `sing-box` TUN `exclude_domains` on non-final split-horizon servers) and remove the duplicate `strip_suffix_wildcard` helper in `effective_dns.rs`.

## Capabilities

### New Capabilities

### Modified Capabilities
- `routing-rules`: reject `*` in `DomainFull` routing rules while loading and badging stored invalid rules without rewrite
- `dns-configuration`: unify `DnsRuleMatch` validation (`DomainSuffix` optional `*.` + TLD support, `DomainFull` wildcard rejection) and preserve matcher kind and TUN exclusions in `EffectiveDns` scopes
- `dns-preferences-ui`: validate and badge all `DnsRuleMatch` variants inline in the DNS rule dialog and stored rule list
- `subscription-update`: enforce shared `DomainFull` and `DnsRuleMatch` validation during Xray JSON routing and DNS import

## Impact

- `crates/core/src/models/validation.rs` and `crates/core/src/models/dns.rs`: `WildcardDomainFull`, `InvalidDomainSuffix`, `validate_domain_full`, `validate_domain_suffix`, and shared `validate_dns_rule_match`.
- `crates/core/src/config/effective_dns.rs`: prefixed matcher tokens on `sing-box` and `v2ray`/`xray` keyword scopes, `canonical_rule_set_tag` GeoSite filtering and TUN `exclude_domains` scope on `sing-box`, and reuse of `crate::config::common::strip_suffix_wildcard`.
- `crates/subscription/src/json_import.rs`: replace local `dns_rule_match_valid` / `valid_domain_suffix` with `validate_dns_rule_match`.
- `crates/ui/src/preferences/dns/rules.rs` and `crates/ui/src/preferences/dns/mod.rs`: delegate `dns_rule_match_validation_error` to `validate_dns_rule_match` and update unit tests.
