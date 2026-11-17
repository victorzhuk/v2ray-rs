# Design

## Context

See `proposal.md` for motivation. Across `core`, `process`, `subscription`, `tray`, and `ui`, several runtime lifecycles interact across async tasks and GTK/D-Bus boundaries:
1. `InstanceLock::acquire` currently opens `v2ray-rs.lock` with `.truncate(true)` before `flock` and `read_holder_pid` calls `AppPaths::resolve` (which rewrites `instance.json` before lock acquisition!), corrupting the holder record on contention; meanwhile `minimize_to_tray` hides the window even on trayless Wayland compositors (Hyprland/Sway/GNOME without AppIndicator) where a second launch exits `75`, stranding the user.
2. `crates/ui/src/connection.rs` pre-resolves every candidate hostname in a subscription upfront before TUN connect (leaking 50–500 proxy hostnames in plaintext DNS), blocks up to 3s per hostname on dead fail-closed routes during `Error` reconnects, cannot cancel `os_lookup`/`probe_pins` on `Stop`, leaves parked `xray` TUN routes installed if the user switches to non-TUN, and retains parked failed `ProcessManager`s after failover succeeds.
3. `crates/process/src/pid.rs` treats `read_link("/proc/<pid>/exe")` `EACCES` as `NotOurProcess`, which fails on Linux kernels with `fs.suid_dumpable = 2` whenever `xray` or `sing-box` holds `cap_net_admin+ep`.
4. `RealDelaySession` and `AvailabilitySession` send unmarked traffic from an ephemeral backend (`mark = 0`), which an active TUN session captures into the currently connected node instead of measuring each tested outbound; large subscriptions also open `N` simultaneous loopback sockets and hit `RLIMIT_NOFILE`.
5. Multiple GTK4/libadwaita dialogs (`routing.rs`, `nodes.rs`, `subscriptions.rs`, `dns.rs`, `network.rs`, `tun.rs`, `wizard.rs`) close `adw::AlertDialog` before validating, clamp `None` ports to `1`, walk private `AdwExpanderRow` child trees, or call MT-unsafe `libc::setlocale(LC_ALL, ...)` after Tokio/GTK threads are spawned.

## Goals / Non-Goals

**Goals:**
- Never strand a running desktop instance: focus via D-Bus on default paths and check `StatusNotifierWatcher` before hiding on close.
- Bound TUN pre-connect DNS disclosure (`<= 8` lookahead only for strict `xray` TUN, lazy per-candidate otherwise) while keeping strict `xray` failover leak-free and `Stop` instantaneous.
- Ensure reliable orphan cleanup under `setcap` + `fs.suid_dumpable=2`, accurate `RuntimeSnapshot` dirty tracking, bounded probe resource use, and non-destructive GTK form validation.

**Non-Goals:**
- Moving `connection.rs` supervision into `v2ray-rs-process` or replacing direct persistence calls with `WorkspaceStore` (isolated in `split-workspace-boundaries` so behavioral fixes verify cleanly first).

## Decisions

### 1. Non-clobbering `InstanceLock` + `gio::Application` D-Bus activation + `StatusNotifierWatcher` check (`d4`)
- **Choice:**
  - Open `v2ray-rs.lock` with `OpenOptions::new().create(true).read(true).write(true)` (no `.truncate(true)`) and write `instance.json` ONLY after `flock(LOCK_EX | LOCK_NB)` succeeds. Read `instance.json` directly on contention without calling stamp-writing helpers.
  - When no `--*-dir` / `V2RAY_RS_*_DIR` overrides are active and a graphical session bus exists, register `adw::Application` with `ApplicationFlags::default()` under profile App ID `com.github.v2ray-rs[.qualifier]` so a secondary launch invokes `activate` on the primary instance (presenting and focusing its window) and exits `0`. When `--*-dir` overrides are active (where App ID alone does not disambiguate custom directories) or D-Bus registration/activation is unavailable, fall back to `InstanceLock` holder PID message + exit `75`.
  - Before hiding the main window on `close-request` with `minimize_to_tray = true`, query `org.kde.StatusNotifierWatcher` (`org.freedesktop.StatusNotifierWatcher` fallback) property `IsStatusNotifierHostRegistered` on the session bus with a short timeout; if `false` or unreachable, proceed with normal quit instead of `set_visible(false)`.
- **Rationale:** Gives expected desktop UX on standard launches while preserving isolated `--*-dir` CLI behavior and preventing invisible unrecoverable processes on trayless compositors.

### 2. Bounded lookahead vs lazy TUN hostname pinning + `tokio::select!` cancellation (`d5`)
- **Choice:**
  - When `tun.enabled && backend == Xray && tun.strict_route`, pre-resolve at most the first `8` planned candidates (plus any `via_node`s they reference) before candidate 0 installs strict routes; if all 8 fail, briefly tear down routes only if user opted out of strict block or rely on `last_good_pins` for remaining candidates without leaking unmarked DNS.
  - When `tun.enabled` with `sing-box` or non-strict `xray` (no parked fail-closed routes between candidates), resolve each candidate lazily right before its config is written.
  - When `parked_tun` fail-closed routes are already installed (e.g. automatic reconnect from `Error`), skip live `os_lookup` and `probe_pins` entirely and populate from `last_good_pins`.
  - Wrap `os_lookup` and `probe_pins` in `tokio::select!` against `cmd_rx` so `Stop` cancels in-flight network waits immediately.
- **Rationale:** Cuts pre-connect plaintext DNS queries from `O(N)` (hundreds of nodes) to `1` in the common case and `<= 8` on strict `xray`, eliminates 3s reconnect stalls on dead routes, and keeps strict kill-switch guarantees intact.

### 3. Fallback `/proc/<pid>` verification for `setcap` binaries
- **Choice:** Store `start_time_ticks: u64` (field 22 of `/proc/<pid>/stat`), `binary_name`, and `config_path` in `backend.pid` via atomic `NamedTempFile` (`0600`). In `verify_pid_identity`, if `read_link("/proc/<pid>/exe")` fails with `PermissionDenied`, verify all three of: `start_time_ticks` exact match, `/proc/<pid>/comm` matches binary filename prefix (15-byte comm limit), and `/proc/<pid>/cmdline` NUL-delimited args contain `-c` / `run` + exact `config_path`.
- **Rationale:** On `fs.suid_dumpable = 2` (default on hardened Arch/Ubuntu), capability-bearing processes make `/proc/<pid>/exe` root-readable (`EACCES` for owner), while `stat`, `comm`, and `cmdline` remain readable and `(pid, start_time_ticks, config_path)` uniquely identifies the process within a boot.

### 4. Gate probes under active TUN and chunk probe batches (`<= 64`)
- **Choice:** Disable/reject `Test Real Delay` and `Check Availability` while `launched_snapshot` has TUN active, showing a clear toast. Hold `REAL_DELAY_LOCK` across `runner.stop().await`, chunk node slices into batches of `64`, scale outer caller timeout to `per_batch_timeout * batch_count`, reuse one `tonic::transport::Channel` per batch with `ready().await`, and exit Observatory polling as soon as all batch tags report status.
- **Rationale:** Prevents misleading measurements routed through the active TUN tunnel, avoids `EMFILE` socket exhaustion on 500-node subscriptions, and eliminates 5s dead-wait tails on dead Xray nodes.

### 5. Reactive validation in GTK4 `AlertDialog`s and pre-thread `LC_MESSAGES` init
- **Choice:** Attach live `changed` validators on dialog inputs to toggle `dialog.set_response_enabled("save", is_valid)` (or intercept before closing via custom window/controller) so invalid entries never destroy user input; initialize `gettextrs` with `LocaleCategory::LcMessages` in `main()` before creating the Tokio runtime or GTK threads, and require restart for language changes instead of calling `setlocale(LC_ALL, ...)` in a multithreaded process (`s3`).
- **Rationale:** `setlocale` in a multithreaded Linux process is undefined behavior (glibc env/locale data race), and `LC_ALL` also breaks float formatting (`1.5` -> `1,5`).

## Risks / Trade-offs

- **[Risk] Language changes in Preferences require app restart instead of live `setlocale` mutation** -> GTK widgets built before language change already required rebuild/restart to re-render labels; showing a restart notice eliminates UB cleanly.
- **[Risk] Strict `xray` TUN failover beyond candidate 8 cannot run live OS DNS without parked routes** -> Candidates 9+ still use `last_good_pins` or IP literals, and 8 candidates cover realistic failover depth without leaking full subscription inventories.
