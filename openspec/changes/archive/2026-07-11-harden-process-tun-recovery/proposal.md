## Why

Two small robustness holes found in the session gap-scan: a `ProcessManager` parked in `Error` with no child can never be driven back to `Stopped` through its own `stop()`/`shutdown()` (both no-op when `child` is `None`), and the one-time TUN privilege grant checks the `nosuid` mount only for the backend binary — on a `nosuid` install tree the elevated `setcap`/setuid on the route helper and bypass wrapper silently doesn't take, and the failure surfaces later, opaquely, at route-programming time.

## What Changes

- `stop()` with no child transitions `Error → Stopped` instead of returning silently; `shutdown()` drops its `child.is_some()` gate so it always reaches `stop()`.
- The privilege grant preflights `file_caps_supported` for the route helper and (when present) the `v2ray-rs-run` wrapper, not just the backend, and fails fast naming the offending path.
- Non-spec riders in the same PR: deduplicate the xray fwmark constant (shared value asserted by a cross-crate dev-dependency test; netctl's runtime dependency footprint unchanged) and install `sing-box` in the CI test job so the `singbox_check` schema test stops silently skipping.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: "Stop backend process" gains a scenario for stop/shutdown requested while in `Error` with no running child.
- `tun-mode`: "TUN requires elevated capabilities granted once" — the file-capabilities-unsupported scenario extends from the backend binary to the route helper and bypass wrapper.

## Impact

- `crates/process/src/manager.rs` — `stop()`/`shutdown()`; one unit test.
- `crates/process/src/privilege.rs` — `grant()` preflight; reuses existing `file_caps_supported` and `PrivilegeError::Unsupported`.
- `crates/core/src/config/xray.rs` + `crates/netctl/src/net.rs` — fwmark constants made `pub`; netctl gains a dev-only dependency on `v2ray-rs-core` for one equality test (release binary unchanged).
- `.github/workflows/ci.yml` — add `sing-box` to the Arch container package install line.
