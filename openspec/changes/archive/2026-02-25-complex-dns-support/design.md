# Design: complex-dns-support

## Context

The app currently has a minimal `DnsConfig` in `crates/core/src/models/dns.rs`: a boolean toggle, two fixed server slots (remote/domestic), and a `DnsProtocol` enum with only `Plain` and `DoH`. The `build_dns()` functions in both v2ray and sing-box generators auto-derive DNS routing from the existing routing rules. There is no UI — the DNS config sits at defaults (Cloudflare DoH remote, Alibaba plain domestic).

Both v2ray/xray and sing-box support significantly richer DNS configurations:
- **V2ray/Xray**: Multiple server types (UDP, TCP, DoH, DoQ), queryStrategy, hosts, per-server domains/expectedIPs, disableCache, disableFallback, clientIp, fakedns
- **Sing-box**: Typed server objects (udp/tcp/tls/https/quic/h3), DNS rules with route/reject actions, fakeip with IP ranges, strategy (prefer_ipv4 etc.), client_subnet, independent_cache, reverse_mapping

The Preferences dialog (`preferences.rs`) currently has 3 pages: System, Network, Routing. DNS will be a 4th page.

## Goals / Non-Goals

**Goals:**
- Rich DNS model covering the intersection of v2ray/xray and sing-box DNS features
- Backend-aware config generation that maps the unified model to each backend's schema
- Full DNS Preferences page with backend-conditional UI (e.g., FakeIP only for sing-box)
- Backward-compatible deserialization — existing settings.toml files load without error
- Auto-derived DNS rules from routing rules as the default behavior (current behavior preserved)
- User-defined DNS rules that override the auto-derived ones

**Non-Goals:**
- DNS-as-inbound (dokodemo-door port 53 hijacking for transparent proxy)
- DNS over H2C (xray-specific niche, requires manual outbound+streamSettings wiring)
- Per-routing-rule DNS overrides (DNS rules remain a separate concern)
- DNS log/query inspection UI
- DoH/DoT/DoQ address resolution bootstrapping (sing-box `address_resolver` chains) — use IP addresses for now

## Decisions

### D1: Unified protocol enum with per-backend capability mapping

Introduce `DnsProtocol` variants: `Udp`, `Tcp`, `Doh`, `Dot`, `Doq`, `H3`. Not all backends support all protocols — unsupported ones fall back silently with a log warning.

| Protocol | V2ray address format | Xray address format | Sing-box server type |
|----------|---------------------|--------------------|--------------------|
| Udp      | `IP:port`           | `IP:port`          | `udp` type object  |
| Tcp      | `tcp://host:port`   | `tcp://host:port`  | `tcp` type object  |
| Doh      | `https://host/dns-query` | `https://host/dns-query` | `https` type object |
| Dot      | ❌ fallback→DoH     | `tls://host`       | `tls` type object  |
| Doq      | ❌ fallback→DoH     | `quic+local://host`| `quic` type object |
| H3       | ❌ fallback→DoH     | ❌ fallback→DoH    | `h3` type object   |

Default ports: UDP/TCP=53, DoH/H3=443 (path `/dns-query`), DoT=853, DoQ=853. Port is only emitted when non-default.

**Rationale**: A unified enum keeps the model backend-agnostic. V2ray-core only supports UDP/TCP/DoH. Xray adds DoT (`tls://`) and DoQ (`quic+local://`). H3 is sing-box-only. Unsupported protocols fall back to DoH with a warning log, rather than failing silently or producing invalid configs.

### D2: Named DNS servers with tags

Replace the fixed remote/domestic pair with `Vec<DnsServerConfig>` where each server has a `tag` (user-assigned name like "remote", "domestic", "adblock"), protocol, address, optional port, and optional `detour` (outbound tag to route DNS traffic through).

Default config ships with two pre-populated servers ("remote" and "domestic") matching current defaults.

**Rationale**: Named servers are required by both backends for DNS rule routing. Tags are internal identifiers used for UI and DNS rule authoring. In sing-box, they map directly to `server.tag`. In v2ray/xray, tags are used to group domain lists into per-server `domains` arrays (v2ray identifies servers by position, not tag).

### D3: DNS rules separate from routing rules

Add `Vec<DnsRule>` and a `use_custom_rules: bool` toggle to `DnsConfig`. Each DNS rule maps a match condition (domain suffix, geosite) to a DNS server tag. The `use_custom_rules` toggle is the **sole source of truth** for which mode is active:
- `use_custom_rules == false` → generator ignores `rules` and auto-derives from routing rules (current behavior)
- `use_custom_rules == true` → generator uses `rules` vec exclusively

This means `rules` can contain saved entries even when auto-derive mode is active — they're simply not used until the toggle is flipped.

**Rationale**: DNS rules and routing rules serve different purposes. An explicit toggle avoids ambiguity (vs. checking if rules vec is empty). Auto-derivation is a good default but power users need to override DNS routing independently.

### D4: FakeIP as optional sing-box-only feature

`FakeIpConfig { enabled: bool, inet4_range: String, inet6_range: String }` with defaults matching sing-box conventions (`198.18.0.0/15`, `fc00::/18`). Config generators skip FakeIP for v2ray/xray. UI shows FakeIP section only when sing-box is the selected backend.

**Rationale**: FakeIP is a sing-box concept. Xray has fakedns but works differently (as a DNS server address, not a config section). Keeping it sing-box-only simplifies the model.

### D5: DNS Preferences as a new PreferencesPage

Add `build_dns_page()` taking `(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback)` — same signature as `build_system_page`/`build_network_page`. Returns `adw::PreferencesPage` with icon `network-transmit-symbolic`. The page is structured as preference groups:

1. **DNS** group — master enable toggle, strategy dropdown
2. **Servers** group — list of servers with add/edit/remove; edit dialog has protocol combo, address entry, port spin, detour combo (detour only shown when backend is sing-box)
3. **Rules** group — toggle between auto-derived and custom; custom rule list with add/edit/remove
4. **FakeIP** group — visible only when backend is sing-box; enable toggle + range entries
5. **Advanced** group — disable_cache toggle, client_subnet entry
6. **Hosts** group — static domain→IP entries with add/remove

**Rationale**: Follows the existing Preferences structure (page per concern). Backend-conditional visibility (FakeIP, detour) avoids confusing users with irrelevant options. Uses the same `(state, cb)` pattern as other settings pages for consistency.

### D6: Backward-compatible serde with migration layer

The old `DnsConfig` has fields `remote: DnsServer` and `domestic: DnsServer`. The new model uses `servers: Vec<DnsServerConfig>`. Simple `#[serde(default)]` would silently lose old DNS server addresses (old fields are unrecognized, `servers` defaults to empty → gets default servers instead of user's saved ones).

**Solution**: Use a `DnsConfigWire` intermediate deserialization struct that accepts both old and new field shapes:
- If `servers` is non-empty: use new format directly
- If `servers` is empty but `remote`/`domestic` exist: convert legacy fields to `DnsServerConfig` entries with tags "remote"/"domestic"
- If neither: use `DnsConfig::default()`
- All new fields (`strategy`, `rules`, `fakeip`, etc.) use `#[serde(default)]`

This preserves user DNS settings across the upgrade. On next save, the new format is written.

**Rationale**: Users who customized DNS via manual TOML edits (the only way until now) should not lose their settings. The wire type is a small amount of code for a significant correctness guarantee.

### D7: Domain match semantics — DomainSuffix, not globs

DNS rule domain matching uses `DomainSuffix` semantics (not glob patterns like `*.example.com`). A domain suffix `"example.com"` matches `example.com` itself and all subdomains (`www.example.com`, `mail.example.com`). This maps cleanly to both backends:
- V2ray/Xray: `"domain:example.com"` prefix in server `domains` list
- Sing-box: `"domain_suffix"` field in DNS rule objects

UI accepts user input like `example.com` and stores it as a suffix. No glob/wildcard processing.

**Rationale**: Both backends use suffix-based domain matching natively. Introducing glob patterns would require parsing/normalization and risk subtle misrouting.

## Risks / Trade-offs

- **[Model complexity]** The expanded model has many fields, most optional → Mitigated by sensible defaults and progressive disclosure in UI (advanced group collapsed by default)
- **[Backend feature mismatch]** H3 not supported by v2ray, FakeIP not supported by v2ray → Mitigated by fallback mapping (H3→DoH) and conditional UI/config generation
- **[DNS rule conflicts with auto-derivation]** User-defined DNS rules could conflict with auto-derived ones → Mitigated by making it explicit: either auto-derive OR use custom rules, not both simultaneously
- **[Hosts table scale]** Users might add many host overrides → Acceptable for a desktop app; use scrolled list with reasonable height limit
- **[Detour is sing-box only]** V2ray/xray DNS servers don't support per-server detour; DNS traffic egress is controlled via routing → Mitigated by ignoring detour field in v2ray/xray generators and hiding detour UI control for non-sing-box backends
- **[Protocol fallback opacity]** Silent DoT/DoQ→DoH fallback may confuse users → Mitigated by logging a warning during config generation when fallback occurs
