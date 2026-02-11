# Proposal: Routing Rule Manager

## Why

Advanced routing is a key differentiator — users want granular control over which traffic goes through the proxy vs direct. The app must support GeoIP-based routing (e.g., route Russian traffic directly), GeoSite domain categories, and custom domain/IP rules. This replaces manual JSON editing of complex routing configurations.

## What Changes

- Define routing rule types: GeoIP, GeoSite, domain match, IP CIDR, with actions (proxy, direct, block)
- Support rule priority and ordering
- Download, store, and auto-update GeoIP and GeoSite databases (v2ray-compatible .dat files or sing-box .db files)
- Provide CRUD operations for routing rules
- Validate rules and provide feedback on conflicts or errors
- Persist rules as part of user configuration

## Capabilities

### New Capabilities
- `routing-rules`: CRUD for routing rules with GeoIP, GeoSite, domain, and IP-based matching and priority ordering
- `geodata-management`: Download, store, and auto-update GeoIP/GeoSite databases

### Modified Capabilities
(none)

## Impact

- New module: `routing`
- Dependencies: reqwest (for geodata downloads), potentially maxminddb or custom .dat parser
- Depends on: core-data-models (routing rule model)
- Feeds into: config-generation (rules are embedded in generated configs)
