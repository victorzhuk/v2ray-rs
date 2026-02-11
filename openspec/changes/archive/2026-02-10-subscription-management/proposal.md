# Proposal: Subscription Management

## Why

Users need to import proxy subscriptions via URL or file, parse them into individual proxy nodes, and keep them updated. Subscription management is the primary way users add proxy configurations to the app. Without it, users would need to manually create proxy entries.

## What Changes

- Fetch subscription content from URLs (base64-encoded or JSON)
- Parse subscription responses into individual proxy nodes (VLESS, VMess, Shadowsocks, Trojan URI schemes)
- Import subscriptions from local files
- Store multiple subscriptions with metadata (name, URL, last update time)
- Auto-update subscriptions on configurable intervals
- Allow enabling/disabling individual nodes within a subscription
- Handle network errors, invalid responses, and parsing failures gracefully

## Capabilities

### New Capabilities
- `subscription-import`: Fetch, parse, and store proxy subscriptions from URLs and files
- `subscription-update`: Auto-update subscriptions on configurable intervals with error handling

### Modified Capabilities
(none)

## Impact

- New module: `subscription`
- Dependencies: reqwest (HTTP), base64, url
- Depends on: core-data-models (proxy node and subscription models)
- Feeds into: config-generation (provides nodes for config output)
