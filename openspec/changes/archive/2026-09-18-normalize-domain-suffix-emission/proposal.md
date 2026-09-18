## Why

Domain patterns accepted by the rule editor are emitted with a literal `*`, which neither backend interprets as a wildcard. Xray TUN excluded domains are emitted as substring matches despite their suffix label.

## What Changes

- Normalize one leading `*.` from suffix-meaning values only when generating backend configuration.
- Emit backend-equivalent suffix matchers for routing, DNS, and TUN exclusions.
- Emit xray TUN excluded domains with `domain:` rather than as unprefixed substring values.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `config-generator`: preserve domain suffix meaning across xray, v2ray, and sing-box output.

## Impact

- Core configuration generators and their tests.
- No persistence or UI behavior changes.
