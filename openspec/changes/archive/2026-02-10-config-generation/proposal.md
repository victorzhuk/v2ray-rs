# Proposal: config-generation

## Why

The core value proposition of the app is generating valid JSON configuration files for v2ray, xray, and sing-box from user-managed subscriptions and routing rules. Users should never need to hand-edit JSON configs. The app must produce correct, backend-specific configs and regenerate them when inputs change.

## What Changes

- Generate valid v2ray JSON configuration from proxy nodes and routing rules
- Generate valid xray JSON configuration (v2ray-compatible with extensions)
- Generate valid sing-box JSON configuration (different schema)
- Write configs to well-known, user-accessible paths
- Automatically regenerate configs when subscriptions or routing rules change
- Validate generated configs against backend expectations
- Support inbound proxy configuration (SOCKS5/HTTP local proxy ports)

## Capabilities

### New Capabilities
- `config-generator`: Generate valid JSON configuration files for v2ray, xray, and sing-box backends from proxy nodes and routing rules

### Modified Capabilities
(none)

## Impact

- New module: `config` with per-backend generators
- Dependencies: serde_json
- Depends on: core-data-models, backend-detection (to know target format), subscription-management (provides nodes), routing-rule-manager (provides rules)
- Feeds into: process-management (provides config path to launch backend)
