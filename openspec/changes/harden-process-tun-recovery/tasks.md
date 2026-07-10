## 1. Process manager totality

- [ ] 1.1 `stop()`: when `child` is `None` and state is `Error`, transition to `Stopped`; keep silent `Ok(())` for already-`Stopped`
- [ ] 1.2 `shutdown()`: drop the `child.is_some()` gate — always set `auto_restart = false` and call `stop()`
- [ ] 1.3 Unit test: manager driven to `Error` (missing binary) then `stop()` → state `Stopped`

## 2. Privilege grant preflight

- [ ] 2.1 In `grant()`, run `file_caps_supported` for the route helper and, when present, the `v2ray-rs-run` wrapper before invoking pkexec; fail with `PrivilegeError::Unsupported` naming the path
- [ ] 2.2 Unit test for the preflight branch (mount detection is already unit-tested; cover the new call sites)

## 3. Fwmark single source of truth

- [ ] 3.1 Make `XRAY_TUN_FWMARK` (core) and `XRAY_FWMARK` (netctl) `pub`
- [ ] 3.2 Add `v2ray-rs-core` as a netctl dev-dependency and one test asserting the constants are equal

## 4. CI

- [ ] 4.1 Add `sing-box` to the pacman install line in the CI test job so `singbox_check` runs against the real binary

## 5. Verification

- [ ] 5.1 `cargo test --workspace` green; `cargo clippy` clean; confirm netctl release binary links without core
