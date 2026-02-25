# Tasks: complex-dns-support

## 1. Expand DNS domain model

- [x] 1.1 Rewrite `DnsProtocol` enum with variants: Udp, Tcp, Doh, Dot, Doq, H3; add `server_address()` formatting for each
- [x] 1.2 Replace `DnsServer` with `DnsServerConfig` struct: tag, protocol, address, port (Option<u16>), detour (Option<String>)
- [x] 1.3 Add `DnsStrategy` enum: PreferIpv4, PreferIpv6, Ipv4Only, Ipv6Only
- [x] 1.4 Add `DnsRuleMatch` enum (GeoSite, DomainSuffix) and `DnsRule` struct (match condition + server tag)
- [x] 1.5 Add `FakeIpConfig` struct: enabled, inet4_range, inet6_range with defaults
- [x] 1.6 Add `HostOverride` struct: domain, ip
- [x] 1.7 Rewrite `DnsConfig`: enabled, strategy, servers (Vec<DnsServerConfig>), rules (Vec<DnsRule>), fakeip (FakeIpConfig), disable_cache, client_subnet (Option<String>), hosts (Vec<HostOverride>), use_custom_rules toggle
- [x] 1.8 Implement `DnsConfig::default()` with two default servers (remote/domestic) matching current behavior
- [x] 1.9 Add validation: unique server tags, valid IP for client_subnet, DNS rule references existing server tag
- [x] 1.10 Implement `DnsConfigWire` intermediate deserialization struct for backward-compatible migration from old `remote`/`domestic` format to new `servers` Vec
- [x] 1.11 Write unit tests: serde roundtrip, default config, server_address formatting for all 6 protocols, backward compat migration from old TOML, new format direct load

## 2. Update v2ray/xray config generation

- [x] 2.1 Rewrite `build_dns()` in v2ray.rs to emit multiple servers from `DnsServerConfig` list with protocol-appropriate address formats
- [x] 2.2 Add queryStrategy mapping from `DnsStrategy` (PreferIpv4→"UseIPv4", etc.)
- [x] 2.3 Add per-server `domains` from DNS rules (or auto-derived from routing rules when use_custom_rules=false)
- [x] 2.4 Add hosts section generation from `HostOverride` list
- [x] 2.5 Add disableCache and clientIp fields when configured
- [x] 2.6 Add protocol fallback for v2ray: DoT/DoQ/H3 → DoH with log warning; ignore detour field
- [x] 2.7 Write/update integration tests for v2ray DNS config generation with various configurations

## 3. Update sing-box config generation

- [x] 3.1 Rewrite `build_dns()` in singbox.rs to emit typed server objects from `DnsServerConfig` list (udp/tcp/tls/https/quic/h3)
- [x] 3.2 Add DNS rules generation from `DnsRule` list (or auto-derived when use_custom_rules=false)
- [x] 3.3 Add strategy mapping from `DnsStrategy` (PreferIpv4→"prefer_ipv4", etc.)
- [x] 3.4 Add FakeIP server and fakeip config section when enabled
- [x] 3.5 Add host overrides as hosts-type DNS server for sing-box
- [x] 3.6 Add disable_cache and client_subnet fields when configured
- [x] 3.7 Write/update integration tests for sing-box DNS config generation with various configurations

## 4. DNS Preferences page

- [x] 4.1 Add `build_dns_page()` function returning `adw::PreferencesPage` with DNS icon; wire into `show_preferences()` dialog
- [x] 4.2 Build DNS master toggle (SwitchRow) and strategy dropdown (ComboRow) in a top-level preferences group
- [x] 4.3 Build Servers preferences group: render server list as ActionRows, add button to open add/edit dialog
- [x] 4.4 Build server add/edit dialog: tag entry, protocol ComboRow, address entry, port SpinRow, detour ComboRow (visible only for sing-box backend)
- [x] 4.5 Build server remove with confirmation and DNS rule cleanup
- [x] 4.6 Build DNS Rules group: toggle between auto-derived and custom mode; render custom rule list with add/edit/remove
- [x] 4.7 Build DNS rule add/edit dialog: match type ComboRow (GeoSite/Domain Suffix), value entry, server tag ComboRow
- [x] 4.8 Build FakeIP group (conditional on sing-box backend): enable toggle, inet4_range and inet6_range entries
- [x] 4.9 Build Advanced group: disable_cache SwitchRow, client_subnet EntryRow with IP validation
- [x] 4.10 Build Hosts group: list of host entries with add/remove, add dialog with domain + IP entries
- [x] 4.11 Wire all controls to `emit()` callback for auto-persist via existing settings callback pattern
- [x] 4.12 Add backend-conditional visibility: hide FakeIP group when backend is not sing-box

## 5. Integration and testing

- [x] 5.1 Verify backward compat: load old-format settings.toml with `remote`/`domestic` fields, confirm server addresses are migrated (not lost) and new fields get defaults
- [x] 5.2 End-to-end test: configure DNS in preferences → verify generated v2ray config JSON is valid
- [x] 5.3 End-to-end test: configure DNS in preferences → verify generated sing-box config JSON is valid
- [x] 5.4 Test DNS settings change triggers config regeneration
