# Proposal: add-tun-exclude-routing

## Why

When TUN mode is active the backend becomes the system default route, so *all*
outbound traffic is captured into the tunnel. Self-tunnelling tools — the
motivating case is `cloudflared tunnel`, but the problem generalises to any tool
that must reach its endpoint directly — break, because their traffic is forced
through the proxy. Users need an exclude list so chosen processes and
destinations bypass the TUN.

The bypass primitive already exists: xray's privileged route helper installs a
policy rule that diverts fwmark-255 packets past the tunnel, and xray's
`direct`/`freedom` outbound is already stamped with that mark. sing-box matches
connections to their owning process natively. This change exposes both as a
config-only exclude list — no new privileges.

## What Changes

- Add two fields to `TunConfig` (persisted in the `[tun]` section):
  `exclude_processes` (process basenames) and `exclude_domains` (domain
  suffixes). The existing `exclude_routes` (CIDRs) is retained.
- **sing-box** (native; covers app-launched *and* already-running tools): emit
  route rules `{ process_name: [...], outbound: "direct" }` and
  `{ domain_suffix: [...], outbound: "direct" }` ahead of the user rules, plus
  matching DNS rules so excluded names/domains resolve directly. CIDRs continue
  via the existing `route_exclude_address`.
- **xray** (destination-based; process-name matching is impossible for TUN
  traffic): emit routing rules `{ type: "field", ip: [...], outboundTag: "direct" }`
  and `{ type: "field", domain: [...], outboundTag: "direct" }` ahead of the user
  rules when TUN is enabled — these bypass the tunnel via the existing
  fwmark-stamped `direct` outbound. xray now honours `exclude_routes` (previously
  ignored). Excluded domains are bound to the direct/domestic DNS server so their
  resolution does not traverse the tunnel.
- **UI**: the TUN preferences page gains an *Excluded domains* list (both
  backends) and an *Excluded applications* list (sing-box only, with a note that
  xray cannot match by process name); the existing *Excluded routes* (CIDR) list
  is ungated for xray.

Per-process exclusion on the xray backend (for app-launched tools) is a separate,
sequenced change (`add-tun-process-bypass`); this change is config-only with no
new privilege surface.

## Capabilities

### Modified Capabilities
- `config-generator`: when TUN is enabled, the sing-box and xray generators emit
  process / domain / CIDR exclusion rules (and matching DNS rules) that keep the
  selected traffic out of the tunnel.
- `app-persistence`: the `[tun]` section gains `exclude_processes` and
  `exclude_domains`, both backward-compatible (absent ⇒ empty).
- `tun-preferences-ui`: the TUN page exposes the excluded-domains and
  excluded-applications lists and ungates the excluded-routes list for xray.

## Impact

- **Modified models**: `crates/core/src/models/tun.rs` (two new fields, wire
  struct, `Default`, `validate` using `validate_domain_pattern`).
- **Modified generators**: `crates/core/src/config/singbox.rs` (route + DNS
  exclusion rules), `crates/core/src/config/v2ray.rs` (xray routing + DNS
  exclusion).
- **Modified UI**: `crates/ui/src/preferences/tun.rs` (two new list groups, xray
  ungating).
- **No new dependencies, no new privileges.** sing-box exclusion is native; xray
  exclusion reuses the existing fwmark-255 bypass.
- **Docs**: `CLAUDE.md` (TunConfig fields), `CHANGELOG.md` (Added entry).
