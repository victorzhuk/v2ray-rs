# Design

## Context

See `proposal.md` for motivation. `v2ray-rs` spans an unprivileged GTK/Tokio GUI process, ephemeral `sing-box`/`xray`/`v2ray` probe children, `reqwest` subscription/geodata fetchers, and the setuid-root `v2ray-rs-run` wrapper. Several narrow boundary checks remained hand-rolled or incomplete after earlier hardening passes.

## Goals / Non-Goals

**Goals:**
- Unify probe URL validation and wire-request formatting without adding an HTTP client dependency to `v2ray-rs-process`.
- Authenticate ephemeral `sing-box` Clash API controllers per probe batch.
- Prevent spurious `SIGTERM` delivery to probe children when multi-threaded Tokio retires worker threads.
- Close CGNAT/benchmarking redirect gaps and enforce GitHub host allowlists on geodata redirects.
- Bound per-line heap allocation on subprocess pipes, remove the `0644 -> 0600` permission window on pasted subscription blobs, fix `InstanceLock` PID reporting, and close `fd >= 3` in `v2ray-rs-run`.

**Non-Goals:**
- Blocking direct user-configured private/LAN subscription URLs on the initial request hop, or disabling explicit `HTTPS_PROXY` bootstrap variables on user subscription refreshes.
- Adding external dependencies to `v2ray-rs-run` (which remains `libc`-only).

## Decisions

### 1. Shared URL parsing in `core` for `validate_test_url` and `session::probe`
- **Choice:** Parse and validate probe targets via `url::Url` in `v2ray-rs-core` (returning a small validated `ProbeTarget` struct or reusing `validate_test_url` rules: `http`/`https` only, non-empty `host_str()`, empty `username()`/`password()`, `fragment().is_none()`, no ASCII control characters). `session::probe` consumes that validated target, formats `Host: <host>:<port>` whenever `http://` uses a port other than `80`, and requires `200..=299` on `CONNECT` and `200..=399` on `GET`.
- **Why over keeping `split_authority`:** `v2ray-rs-process` already depends on `v2ray-rs-core`, which already depends on `url`. Delegating URL parsing to `core` eliminates duplicate hand-rolled authority/IPv6 splitting while keeping `v2ray-rs-process` free of `reqwest` or TLS stacks.

### 2. Per-batch random bearer token for `SingboxProbeGenerator`
- **Choice:** Extend `SingboxProbeGenerator` (or probe config generation options) with an optional `api_secret: Option<&str>` (always populated with a 16-byte random hex secret from `rand::rng()` during `sing-box` Real Delay / Availability batches). Emit `"secret": secret` under `experimental.clash_api` and pass `.bearer_auth(secret)` on `query_singbox_delay`.
- **Why over UNIX domain sockets:** `sing-box` `external_controller` expects TCP `host:port` across supported 1.13+ builds; bearer token authentication is natively supported by `clash_api` with zero compatibility risk.

### 3. Drop `PR_SET_PDEATHSIG` in `ProbeRunner::start`
- **Choice:** Remove the `unsafe { cmd.as_std_mut().pre_exec(... set_pdeathsig ...) }` block from `crates/process/src/probe.rs`, keeping `.kill_on_drop(true)`, `ProbeRunner::stop()` (`SIGTERM` -> 5s -> `SIGKILL`), and `Drop for ProbeRunner` (`SIGKILL`).
- **Why:** Linux `prctl(PR_SET_PDEATHSIG)` binds to the **calling kernel task (TID)**, not the thread group leader. `v2ray-rs-ui` runs a multi-threaded Tokio runtime (`Runtime::new()`), where idle blocking/worker threads exit legitimately and prematurely kill active probe children.

### 4. Shared non-global IP filter and GitHub host allowlist on geodata redirects
- **Choice:** Expand `ip_is_private_or_local` to cover `100.64.0.0/10` (`(octets[0] == 100) && (octets[1] & 0xc0) == 64`) and `198.18.0.0/15` (`octets[0] == 198 && (octets[1] & 0xfe) == 18`), plus IPv4-mapped IPv6 addresses. In `geodata.rs`, enforce `next.scheme() == "https"`, public non-local host, and host match against `github.com` or `*.githubusercontent.com` (`raw.githubusercontent.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`).
- **Why:** Geodata URLs are fixed GitHub endpoints; redirects only legitimately traverse GitHub's release asset CDN over HTTPS.

### 5. Bounded chunked line reader for child stdout/stderr
- **Choice:** Implement a small async helper `read_bounded_line(reader: &mut BufReader<R>, buf: &mut Vec<u8>, max_bytes: usize)` using `AsyncBufReadExt::fill_buf` + `consume`: scan up to `\n` or `max_bytes` (16 KiB), emit the UTF-8 lossy slice once either bound is hit, and cap `tun::run_helper`'s collected `Vec<String>` (e.g., 1,024 lines max).
- **Why:** `tokio::io::Lines::next_line` loops internally into an unbounded `Vec<u8>` until `\n` or EOF, allowing a single newline-free stream to OOM the GUI before `LogBuffer` line-count eviction runs.

### 6. Atomic `0600` blob writes, `Option<u32>` lock holder PID, and `close_range` in `v2ray-rs-run`
- **Choice:**
  - Replace `std::fs::write` + `set_permissions` in `write_subscription_blob` with `v2ray_rs_core::fs::atomic_write`.
  - Change `InstanceError::LockHeld { pid: Option<u32>, profile: String }` (formatting `"another instance is running (PID unknown)..."` when `None`) and use `create_dir_with_permissions(parent)` (`0700`) in `InstanceLock::acquire`.
  - In `crates/run/src/main.rs`, invoke `libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, 0 as libc::c_uint)` after privilege drop and before `execvp`, falling back to enumerating `/proc/self/fd` (and closing `fd >= 3` other than the directory fd) if `close_range` returns `ENOSYS`/`EINVAL` on older kernels.

## Risks / Trade-offs

- **[Risk] Stricter `validate_test_url` rejects persisted test URLs carrying `#fragment` or credentials** -> Mitigation: Only rejects clearly invalid probe URLs on settings validation; standard probe endpoints (`generate_204`, `cp.cloudflare.com`) have neither credentials nor fragments.
- **[Risk] Older Linux kernels without `close_range` syscall (pre-5.9)** -> Mitigation: Fallback `/proc/self/fd` sweep closes open descriptors cleanly without failing execution.
