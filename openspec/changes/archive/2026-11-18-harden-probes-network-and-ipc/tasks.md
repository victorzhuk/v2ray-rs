# Tasks

## 1. Probe URL Validation and Health Probe Wire Hardening

- [x] 1.1 Update `validate_test_url` (and a shared parsed target helper) in `crates/core/src/models/validation.rs` to reject userinfo credentials, `#fragment` components, empty hosts, and ASCII control characters via `url::Url`, and verify unit tests cover valid `http`/`https` endpoints alongside rejected userinfo, fragment, and malformed IPv6 inputs
- [x] 1.2 Refactor `crates/process/src/session/probe.rs` to consume the validated URL target, emit `Host: <host>:<port>` on non-default HTTP ports, require `200..=299` on `CONNECT` and `200..=399` on `GET` (rejecting `1xx` interim and `3xx` `CONNECT` responses), and verify unit and loopback mock-proxy tests pass

## 2. Ephemeral Probe Controller Auth and Child Lifecycle

- [x] 2.1 Add per-batch cryptographic random bearer secret support to `SingboxProbeGenerator` in `crates/core/src/config/probe.rs` and send `Authorization: Bearer <secret>` in `crates/subscription/src/real_delay.rs`, verifying generated JSON and mock controller auth tests pass
- [x] 2.2 Remove the thread-scoped `PR_SET_PDEATHSIG` `pre_exec` hook in `crates/process/src/probe.rs` while preserving `kill_on_drop(true)`, `ProbeRunner::stop()`, and `Drop` `SIGKILL`, and verify `v2ray-rs-process` probe lifecycle tests pass

## 3. Redirect Policy Hardening for Subscriptions and Geodata

- [x] 3.1 Expand redirect IP filtering in `crates/subscription/src/fetch.rs` to reject RFC 6598 CGNAT (`100.64.0.0/10`) and RFC 2544 benchmarking (`198.18.0.0/15`) ranges (including IPv4-mapped IPv6), and verify redirect unit tests cover blocked and allowed hops
- [x] 3.2 Tighten the geodata `reqwest` redirect policy in `crates/core/src/geodata.rs` to require `https://`, non-private/non-local hosts, and GitHub distribution domains (`github.com`, `*.githubusercontent.com`), and verify unit tests reject off-domain, `http://`, CGNAT, and loopback redirects while accepting `objects.githubusercontent.com`

## 4. Bounded Subprocess Readers, Atomic Blobs, Instance Lock, and Setuid Wrapper

- [x] 4.1 Replace unbounded `BufReader::lines().next_line()` loops in `crates/process/src/manager.rs` and `crates/process/src/tun.rs` with a bounded 16 KiB per-line reader and a line-count cap on `tun::run_helper`, verifying unit tests with newline-free streams pass
- [x] 4.2 Switch `write_subscription_blob` in `crates/ui/src/subscriptions/dialogs.rs` to `v2ray_rs_core::fs::atomic_write` and verify created blob files have mode `0600` without an intermediate `std::fs::write` step
- [x] 4.3 Ensure `InstanceLock::acquire` in `crates/core/src/instance.rs` creates its parent directory with `0700` permissions and represents unreadable/missing `instance.json` holder PIDs as `None` (`unknown`) instead of `std::process::id()`, verifying lock contention tests pass
- [x] 4.4 Close inherited file descriptors `>= 3` (`close_range` with `/proc/self/fd` fallback) before `execvp` in `crates/run/src/main.rs` and verify a subprocess test confirms an un-`CLOEXEC` descriptor `>= 3` is closed in the target child
