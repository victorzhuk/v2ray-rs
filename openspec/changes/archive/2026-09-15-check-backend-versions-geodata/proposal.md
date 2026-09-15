## Why

Version gating is asymmetric: xray TUN checks a minimum version but an unreadable `version` output skips the check silently, and sing-box has no gate at all — the generated config already avoids fields removed in 1.13.0, so an older sing-box fails only inside `check` with a backend-internal message, or worse, loads. Separately, v2ray/xray connects whose routing or DNS rules reference GeoIP/GeoSite tags discover missing `geoip.dat`/`geosite.dat` only when the backend rejects the config, with no path to fix it from the error.

## What Changes

- Generalize the xray-only version probe to both backends; add `SINGBOX_MIN_VERSION` (1, 13, 0) so every sing-box start fails before spawn with the installed and required versions when older (`ProcessError::BackendTooOld { backend, installed, required }`, generalizing the existing `TunBackendTooOld`).
- An unreadable backend version (xray TUN or sing-box) adds a log warning `warning: could not read <backend> version; minimum-version check skipped` and the start proceeds — the config check still guards.
- v2ray/xray connects whose enabled routing rules, active imported-profile rules, or DNS rules reference GeoIP/GeoSite fail before spawn when `geoip.dat`/`geosite.dat` is missing, with a "Download geodata" toast action running the same download as Preferences' "Update Now". sing-box is not gated (remote rule-set fallback).

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "TUN mode availability per backend" gains the unreadable-xray-version warning.
- `process-lifecycle`: new requirement for the sing-box minimum version.
- `geodata-management`: new requirement that v2ray/xray connects need the referenced `.dat` files.

## Impact

- `crates/process/src/manager.rs` — generalized version probe, sing-box gate, `BackendTooOld` rename (is_host_level classification from `gate-tun-capability-preflight` stays true).
- `crates/ui/src/app.rs` — `missing_geodata` predicate, geodata toast with download action.
- Capability gates, helper checks and recovery surfacing are separate changes (`gate-tun-capability-preflight`, `surface-tun-recovery-failures`). No config, persistence, or dependency change.
