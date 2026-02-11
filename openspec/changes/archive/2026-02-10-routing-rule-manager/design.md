## Context

Routing rules determine which traffic goes through the proxy (proxy action), goes directly (direct action), or is blocked. Rules can match by GeoIP country, GeoSite domain category, specific domain patterns, or IP CIDR ranges. The app needs to manage these rules and embed them into generated configs.

## Goals / Non-Goals

**Goals:**
- CRUD operations for routing rules with ordering/priority
- GeoIP and GeoSite database management (download, update, parse)
- Validation and conflict detection
- Persistence of user-defined rules

**Non-Goals:**
- No runtime traffic routing (that's the backend's job)
- No custom GeoIP/GeoSite database creation

## Decisions

1. **Rule model**: Use an enum `RuleMatch { GeoIp(CountryCode), GeoSite(Category), Domain(DomainPattern), IpCidr(IpNet) }` paired with `RuleAction { Proxy, Direct, Block }`. Rules stored in ordered Vec for priority.

2. **GeoIP/GeoSite sources**: Download from v2fly/geoip and v2fly/domain-list-community GitHub releases. Store in data directory. For sing-box, use SagerNet/sing-geoip and SagerNet/sing-geosite.

3. **Update strategy**: Check for geodata updates weekly by default (configurable). Download in background, swap files atomically.

4. **Predefined rule templates**: Ship with common presets (e.g., "RU direct", "CN direct", "ads block") that users can enable with one click.

## Risks / Trade-offs

- [Geodata format differences] v2ray uses .dat (protobuf), sing-box uses .db (custom) → Must handle both formats based on selected backend
- [Large geodata files] GeoIP database is ~5MB, GeoSite ~2MB → Cache aggressively, only re-download when version changes
