## 1. Capability probes

- [x] 1.1 `crates/process/src/privilege.rs`: `has_net_admin` spawns `getcap` with a 5 s bound (kill on expiry) and maps `ENOENT` to an error naming libcap; pure `probe_error_text` unit-tested for timeout, not-found, non-zero exit; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green
- [x] 1.2 Pure `caps_check_needed(euid: u32) -> bool` used by the manager's gates; unit test for 0 and 1000

## 2. Manager preflight

- [x] 2.1 xray TUN helper check before spawn in `start_with_connection`: absolute + exists, `access(X_OK)` (relocated path → relogin error), `has_net_admin(helper)`; new `ProcessError` variants; stub tests with a non-executable temp helper → relogin/permission error and no ` session ` record
- [x] 2.2 Backend `nosuid` check via `file_caps_supported(binary dir)` before the capability probe, error text from `PrivilegeError::Unsupported`; pure predicate test with a synthetic `/proc/self/mounts` string through `mount_for_path`
- [x] 2.3 `ProcessError::is_host_level()` true for capability, probe, helper, mount and version errors (version class is the existing `TunBackendTooOld` variant until `check-backend-versions-geodata` generalizes it), false for config check, spawn and readiness errors; unit test over every variant

## 3. Connection and app

- [x] 3.1 `crates/ui/src/connection.rs` stops the candidate loop on `is_host_level()` errors, reports one `Error` with that error's text, and emits `AppMsg::TunGrantRequired(generation)` first for missing backend/helper capability; stub test: two candidates with a backend lacking caps → one start attempt, one terminal `Error`
- [x] 3.2 `app.rs` shows the `Error` toast with a "Grant TUN privileges" button opening Preferences on the TUN page when the current generation received `TunGrantRequired`; pure helper `error_toast_action(generation, grant_generation) -> Option<ToastAction>` unit-tested

## 4. Verification
- [x] 4.1 `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4` green
- [ ] 4.2 Live: remove the helper capability (`sudo setcap -r` on the helper), connect xray TUN → no backend spawned, toast with "Grant TUN privileges" opens Preferences → TUN
