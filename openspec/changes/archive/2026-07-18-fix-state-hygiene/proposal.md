# State hygiene: stale instance stamp, failing legacy relocation, route-helper spec drift

## Why

Three small but real defects found while diagnosing the TUN incident:

1. `InstanceStamp::update_started()` refreshes `last_started_at`/`last_started_pid` but never `build_version`, so `state/instance.json` reports the version from the *first ever* run (a live install shows `0.7.4` while running `0.13.1`). Anything that reads the stamp for diagnostics — including a human debugging "which build produced this state" — is misled.
2. `relocate_generated_dir()` (and `relocate_geodata_dir()`, same pattern) never creates the destination directory. `runtime_dir/generated/` does not exist at `ensure_dirs()` time (only `ConfigWriter` creates it later), so the cross-device move fails with ENOENT on every start and the legacy `data_dir/generated/` — containing full backend configs with node credentials — lingers indefinitely. Observed live: config files from months ago still sitting in `~/.local/share/v2ray-rs/generated/`.
3. Spec drift: the `tun-mode` "Privileged route helper for xray" requirement still describes `0.0.0.0/1` + `128.0.0.0/1` split routes, but netctl has since moved to a default route in dedicated table 2023 plus policy rules (fwmark bypass at pref 9000, suppress-prefixlen main lookup at 9001, TUN-table lookup at 9002, optional uid-range bypass at 8998). The canonical spec must describe the shipped mechanism.

## What Changes

- `update_started()` also refreshes `build_version` (and rewrites the stamp when the stored version differs).
- Legacy relocation creates the destination directory before moving files, and removes the legacy source files it successfully migrated; a relocation that finds the destination already populated deletes the legacy leftovers instead of keeping credential-bearing copies around.
- `tun-mode` spec: route-helper requirement rewritten to match netctl's table-2023 + policy-rules implementation (no code change).

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `runtime-profiles`: new requirement covering instance-stamp freshness and legacy-file relocation/cleanup.
- `tun-mode`: "Privileged route helper for xray" corrected to the policy-routing mechanism.

## Impact

- `crates/core/src/instance.rs` — `update_started()`.
- `crates/core/src/persistence/mod.rs` — `relocate_generated_dir()`, `relocate_geodata_dir()`, destination-dir creation, leftover cleanup.
- `openspec/specs/tun-mode/spec.md` — text only.
