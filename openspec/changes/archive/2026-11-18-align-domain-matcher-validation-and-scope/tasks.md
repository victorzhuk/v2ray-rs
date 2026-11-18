# Tasks

## 1. Core Domain-Matcher Validation & Effective DNS Scopes

- [x] 1.1 Add `WildcardDomainFull`, `InvalidDomainSuffix`, `validate_domain_full`, `validate_domain_suffix`, and `validate_dns_rule_match` in `crates/core/src/models/validation.rs`, wire `RuleMatch::DomainFull` and `DnsConfig::validate` (`crates/core/src/models/dns.rs`) to them while keeping persistence load validation-free, and verify with `timeout 5m cargo test -p v2ray-rs-core --features test-utils -- --test-threads=4 models::` and `persistence::`
- [x] 1.2 Deduplicate `strip_suffix_wildcard` in `crates/core/src/config/effective_dns.rs` via `super::common::strip_suffix_wildcard`, emit `domain:` / `keyword:` / `full:` prefixes and `canonical_rule_set_tag`-filtered `sing-box` GeoSite scopes plus non-final TUN `exclude_domains` scopes, and verify with `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 config::effective_dns`

## 2. Subscription JSON Import & Preferences UI Parity

- [x] 2.1 Replace `valid_domain_suffix` and `dns_rule_match_valid` in `crates/subscription/src/json_import.rs` with `validate_dns_rule_match`, add import tests for rejected `full:*.example.com` (routing and DNS) and accepted `domain:*.example.com` / `domain:cn`, and verify with `timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4 json_import`
- [x] 2.2 Delegate `dns_rule_match_validation_error` in `crates/ui/src/preferences/dns/rules.rs` to `validate_dns_rule_match`, update DNS rule validation/badging tests in `crates/ui/src/preferences/dns/mod.rs` and routing `DomainFull` row-error coverage in `crates/ui/src/preferences/routing.rs`, and verify with `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4`

## 3. Release Hygiene & Cross-Crate Verification

- [x] 3.1 Update `[Unreleased]` in `CHANGELOG.md` for `DomainFull` wildcard rejection, `DomainSuffix` wildcard support, DNS rule row badging, and `Effective DNS` scope prefixes, and verify `CHANGELOG.md` formatting
- [x] 3.2 Run `cargo clippy --workspace --all-targets -- -D warnings` and `timeout 10m cargo test --workspace -- --test-threads=4` and verify zero warnings or test failures
