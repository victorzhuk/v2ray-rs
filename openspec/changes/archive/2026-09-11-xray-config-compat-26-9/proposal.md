# Stop generating xray fields that current Xray-core deprecates or rejects

## Why

After upgrading the live install to Xray-core 26.9.9, the generated TUN config still passes `xray run -test` but logs:

```
[Warning] infra/conf: The "freedom.domainStrategy" setting is deprecated and will be removed. For compatibility, its value has been automatically migrated to "sockopt.domainStrategy".
```

The migration is not neutral. Since v26.9.8 (XTLS/Xray-core#6058) a non-`AsIs` `freedom.settings.domainStrategy` unconditionally overwrites `streamSettings.sockopt.domainStrategy` (`infra/conf/xray.go:352-362` at v26.9.9). The generator writes both — `"UseIP"` in `settings`, the query-strategy-derived `"UseIPv4"`/`"UseIPv6"` in `sockopt` — so the direct outbound silently resolves dual-stack regardless of the user's DNS strategy, and once the field is removed the config stops loading.

Separately, since v26.6.22 (XTLS/Xray-core#6226) `tlsSettings.allowInsecure: true` is a removed feature that fails the whole config (`infra/conf/transport_security.go:361`, replacement `pinnedPeerCertSha256` / `verifyPeerCertByName`). The generator writes `allowInsecure: !verify` for every TLS node, so any node imported with verification off makes the xray backend refuse to start with an error that does not name the node. None of the 308 nodes on the live install are affected today.

## What Changes

- The xray `freedom` outbound under TUN carries its resolution strategy only in `streamSettings.sockopt.domainStrategy`, derived from the query strategy like every other dialing outbound; `settings.domainStrategy` is no longer emitted.
- xray TLS outbounds never carry `allowInsecure`. A node with certificate verification disabled cannot be expressed for xray: generation for that candidate fails with an error naming the node and the reason, and connection planning moves on to the next candidate. The v2ray backend keeps emitting `allowInsecure`.
- The xray config check test suite fails on any deprecation warning printed by the installed binary, so the next deprecation is caught before users see it.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: "Generate v2ray-compatible configuration" gains TLS-verification scenarios per backend; "TUN mode DNS resolution is self-contained" moves the xray direct-outbound strategy into `sockopt`; new requirement that generated xray configs load without deprecation warnings.

## Impact

- `crates/core/src/config/v2ray.rs` — `harden_tun_outbounds`, TLS stream builder, backend-aware `allowInsecure`.
- `crates/core/src/config/xray.rs` — unchanged mapping, now the only source of the direct outbound's strategy.
- `crates/core/tests/xray_check.rs` — deprecation-warning assertion.
- Behavior: under TUN, direct dials follow the configured DNS strategy on every xray version instead of `UseIP` on 26.9.8+. Nodes with verification off stop being offered to xray instead of breaking the whole start.
