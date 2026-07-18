## 1. Instance stamp

- [x] 1.1 `update_started()` sets `build_version = BUILD_VERSION` alongside the timestamp/pid
- [x] 1.2 Unit test: stamp created with an old version string is rewritten with the current one on `update_started`

## 2. Legacy relocation

- [x] 2.1 `relocate_generated_dir()` / `relocate_geodata_dir()`: shared `relocate_legacy_dir` creates the destination (0o700) before moving
- [x] 2.2 When the destination is already populated, delete the legacy source files instead of returning early and keeping them
- [x] 2.3 Unit tests: cross-directory relocation succeeds into a previously missing destination; populated destination still clears the legacy dir

## 3. Spec sync

- [x] 3.1 Apply the corrected "Privileged route helper for xray" requirement to `openspec/specs/tun-mode/spec.md` (applied at archive)

## 4. Verification

- [x] 4.1 `cargo test --workspace` green
- [ ] 4.2 Manual: seed `data_dir/generated/` with a dummy file, start the app, confirm it disappears and `runtime_dir/generated/` receives it (covered by unit tests; live check happens on next app start against the real May-dated leftovers)
