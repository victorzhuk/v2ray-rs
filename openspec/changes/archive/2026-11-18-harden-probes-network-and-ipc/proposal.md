# Proposal

## Why

Loopback probes, redirect checks, subprocess output readers, and IPC boundaries still leave local exposure, memory growth, and protocol-validation gaps. Tightening these runtime boundaries prevents false-positive health checks, unauthenticated local Clash API access, thread-lifetime probe kills, unbounded log allocations, and descriptor leaks across the setuid wrapper.

## What Changes

- Parse health-check and Real Delay test URLs strictly (`url::Url`, no userinfo, fragments, control bytes, or trailing IPv6 garbage), include non-default ports in HTTP `Host` headers, and require `2xx` for `CONNECT` and `2xx`/`3xx` (never `1xx`) for plain `GET` probes.
- Require a per-batch random bearer `secret` on ephemeral `sing-box` Clash API probe configs and send `Authorization: Bearer <secret>` from the Real Delay probe client.
- Remove thread-bound `PR_SET_PDEATHSIG` from `ProbeRunner::start` so multi-threaded Tokio worker retirement cannot send spurious `SIGTERM` to live probe backends.
- Extend private/local redirect blocking in subscription and geodata HTTP clients to cover RFC 6598 CGNAT (`100.64.0.0/10`), RFC 2544 benchmarking (`198.18.0.0/15`), and IPv4-mapped IPv6 equivalents, and restrict geodata redirects to public HTTPS GitHub hosts.
- Bound individual stdout/stderr line reads (16 KiB cap) in `ProcessManager` and `tun::run_helper`, and cap retained helper output lines.
- Write pasted JSON subscription blobs via owner-only `0600` atomic writes without an initial umask-permissive window, ensure `InstanceLock::acquire` creates `0700` parent directories, and report unknown lock-holder PIDs distinctly instead of substituting `std::process::id()`.
- Close inherited file descriptors `>= 3` inside `v2ray-rs-run` before `execvp`.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `connection-health`: Require strict probe URL parsing, RFC-compliant `Host` headers, `2xx` `CONNECT` tunnel responses, and non-`1xx` HTTP probe status codes.
- `real-delay-latency-test`: Require per-batch bearer authentication on ephemeral `sing-box` Clash API controllers, reject userinfo/fragments in `validate_test_url`, and drop thread-bound `PR_SET_PDEATHSIG` on probe children.
- `subscription-update`: Block redirects to CGNAT (`100.64.0.0/10`) and benchmarking (`198.18.0.0/15`) ranges alongside private and local targets.
- `geodata-management`: Enforce HTTPS-only public GitHub allowlisted hosts on geodata HTTP redirects.
- `process-lifecycle`: Cap subprocess stdout/stderr line allocations, bound helper log buffers, enforce `0700` lock parent directories, and avoid reporting the caller's own PID on lock contention when `instance.json` is unreadable.
- `app-persistence`: Write pasted subscription blobs atomically with `0600` permissions from creation.
- `tun-process-bypass`: Close inherited file descriptors `>= 3` in `v2ray-rs-run` before executing the target command.

## Impact

- `crates/core`: `models/validation.rs`, `config/probe.rs`, `geodata.rs`, `instance.rs`
- `crates/process`: `session/probe.rs`, `probe.rs`, `manager.rs`, `tun.rs`
- `crates/subscription`: `fetch.rs`, `real_delay.rs`
- `crates/ui`: `subscriptions/dialogs.rs`, `app/bootstrap.rs`
- `crates/run`: `main.rs`
- No new external crate dependencies (`url`, `rand`, `libc`, `nix`, and `v2ray_rs_core::fs::atomic_write` are already in the workspace).
