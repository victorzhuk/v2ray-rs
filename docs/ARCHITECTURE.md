# Architecture

v2ray-rs is a Linux desktop GUI wrapper for v2ray/xray/sing-box. It manages
subscriptions, generates backend config files, handles process lifecycle,
geo-routing, DNS, and TUN mode. No protocol logic lives here — all of that is
delegated to the system-installed CLI binary. The app's job is orchestration.

## Crate dependency graph

```
                    ┌─────────┐
                    │   ui    │ (GTK4/Relm4 binary)
                    └────┬────┘
           ┌─────────────┼──────────────┐
           ▼             ▼              ▼
       ┌───────┐   ┌──────────────┐  ┌──────┐
       │process│◄──│subscription  │  │ tray │
       └───┬───┘   └──────┬───────┘  └──┬───┘
           │              │              │
           └──────────────┼──────────────┘
                          ▼
                      ┌──────┐
                      │ core │ (models, config, persistence, geodata)
                      └──────┘
  (subscription → process for the Real Delay probe)

Privileged helpers (separate binaries, no crate deps on core):
  v2ray-rs-netctl  — route helper (CAP_NET_ADMIN via setcap)
  v2ray-rs-run     — setuid-root process wrapper (drops to bypass user)
```

## Runtime data flow

```
Subscription URL/File
  → fetch (reqwest, 30s/60s timeout) + local file read
  → decode (base64 or plaintext) → split lines
  → parse URIs (vless://, vmess://, ss://, trojan://)
  → reconcile against existing nodes (preserve enable flags)
  → Vec<ProxyNode> stored in subscriptions.json

AppSettings + Vec<ProxyNode> + Vec<RoutingRule>
  → ConfigWriter::write_config()
  → ConfigGenerator::generate() [per-backend: v2ray/xray/singbox]
  → atomic_write() → runtime_dir/generated/{v2ray,xray,sing-box}.json

ProcessManager::start()
  → [TUN mode] check CAP_NET_ADMIN on binary
  → tokio::process::Command::new(backend_binary).arg("run").arg("-c").arg(config)
  → stdout/stderr → LogBuffer (ring, 10K lines) + broadcast channel
  → [xray TUN] wait_for_device() + netctl xray-up → routes programmed
  → ProcessState transitions broadcast to UI + tray
```

## Crates

### `core` (`v2ray-rs-core`)

Domain models, persistence, and config generation. Every other crate depends
on this; it has no in-workspace dependencies.

Key types: `ProxyNode` (tagged enum: Vless/Vmess/Shadowsocks/Trojan),
`Subscription`, `RoutingRuleSet`, `AppSettings`, `TunConfig`, `DnsConfig`,
`AppPaths`, `ConfigWriter`, `ConfigGenerator` trait.

`AppPaths` maps the five XDG dirs (config, data, cache, runtime, state) to
well-known file paths. Generated backend configs land in
`runtime_dir/generated/`. All five dirs are created with 0o700 permissions.
`AppProfile` (Production/Development/Test/Custom) gives each profile its own
qualifier so dev and prod never share paths.

`ConfigWriter` calls the right `ConfigGenerator` impl (v2ray/xray/singbox),
serialises the result to JSON, and delegates to `atomic_write()` in
`crates/core/src/fs.rs`: write to a `NamedTempFile` in the same directory,
`sync_all`, then `persist` (rename). The generated file gets 0o600.

`RoutingManager` wraps rule CRUD with auto-persist. `GeodataManager` handles
geodata downloads and metadata: `.dat` whole-file for v2ray/xray, per-tag `.srs`
rule-sets for sing-box. `GeodataIndexManager` builds a searchable tag index (proto
parse for `.dat`, rule-set enumeration for `.srs`). `instance.rs` enforces
single-instance via an exclusive `flock` on `runtime_dir/v2ray-rs.lock`.

### `subscription` (`v2ray-rs-subscription`)

Depends on `core` and `process` (the latter for `ProbeRunner`, used by
`real_delay.rs`). Handles subscription lifecycle end-to-end.

`SubscriptionService` drives refreshes with a cap of 4 concurrent HTTP
requests (reqwest + rustls, 30s connect / 60s total timeout). `update.rs`
(`reconcile_nodes`) merges a freshly fetched node list against the stored one,
preserving per-node enable flags and user renames. `ping.rs` measures TCP
connect latency concurrently via `tokio::spawn`. `real_delay.rs`
(`measure_real_delay`) tests actual proxied delay. `observatory.rs` queries
the running v2ray/xray process for its own observatory results.

### `process` (`v2ray-rs-process`)

Depends on `core`, `tokio`, `nix`. Manages the backend process lifecycle.

`ProcessManager` owns the child process, a circular `LogBuffer` (10K lines),
and two `tokio::broadcast` channels for `ProcessEvent`: one for state
(StateChanged, ProcessExited), one higher-capacity channel for LogLine, so a
burst of backend output can never lag the state channel and drop a terminal
transition. State machine: Stopped → Starting → Running → Stopping → Stopped,
plus Running → Starting for a supervised in-place restart and Running/Starting
→ Error.

Crash recovery: any exit while the state is still Running counts as an
unexpected crash — including a signal death (OOM/segfault/external kill, which
report no exit code on Unix). Each crash is retried with a backoff that scales
with the number of recent crashes (2s, then 4s), up to 3 crashes per 60s before
entering Error. A successful restart re-enters Running with no user-visible
disconnect. Graceful stop: SIGTERM → 5s timeout → SIGKILL. ETXTBSY on spawn is
retried (overlayfs/Docker edge case).

The UI adds a second, bounded layer: after the manager gives up (Error), it
schedules up to 3 whole-connection retries (5s apart, re-planning candidates),
reset on a successful connect and cancelled on an explicit Disconnect.

`privilege.rs` reads binary file capabilities via `getcap` and grants them via
a single `pkexec` elevation (one shell invocation, paths passed as positional
args to avoid injection). `tun.rs` holds `TunRuntime` and the xray-specific
helpers: `wait_for_device()` polls `/sys/class/net/<iface>`, then invokes
`netctl xray-up`. sing-box programs its own routes via `auto_route`.

`PidFile` writes an ownership record (binary path + config path) and
`check_and_kill_orphaned()` kills stale processes from a previous run using
`kill(pid, 0)` as a liveness probe. `ProbeRunner` generates a minimal backend
config and runs it briefly to verify the binary works before a real connect.

### `netctl` (`v2ray-rs-netctl`)

Standalone binary with `CAP_NET_ADMIN` set via `setcap`. No in-workspace crate
dependencies — deliberately minimal (rtnetlink, tokio current-thread, clap).

Three subcommands, all idempotent and input-validated before any netlink call:

- `xray-up --iface --addr [--addr6] [--bypass-uid]`: brings the link up,
  assigns the address, adds `0.0.0.0/1` + `128.0.0.0/1` split routes (and
  IPv6 equivalents), installs a `uidrange` policy rule at priority 8998 when
  `--bypass-uid` is given so that traffic from the bypass user skips the tunnel.
- `xray-down --iface`: removes the device (no-op if already gone).
- `recover --iface --singbox|--xray`: cleans up leftover TUN state after an
  unclean shutdown; for sing-box also flushes its `auto_route` table/rules.

`xray-up` refuses any `--iface` that is not a TUN device (checked via
`/sys/class/net/<iface>/tun_flags`), and the device-delete in `xray-down` /
`recover` only ever deletes a TUN device. This stops a caller from pointing the
helper at `lo`, a physical NIC, or another tunnel's link. As defence-in-depth,
packaging restricts execution of `v2ray-rs-netctl` (0750) and `v2ray-rs-run`
(4750) to the `v2ray-rs` group, so they are not runnable by every local
process; the desktop user must be a member of that group for TUN mode.

### `tray` (`v2ray-rs-tray`)

Depends on `core` and `process`. Implements the StatusNotifierItem protocol via
`ksni`. Uses FreeDesktop theme-aware symbolic SVG icons (three embedded: disconnected/connected/error). Falls back to ARGB32 pixmap rendering when the DE has no theme lookup. `TrayService::spawn()` listens on the process broadcast channel and rebuilds the menu on state changes. `Notifier` sends desktop notifications via `notify-rust`.

### `ui` (`v2ray-rs-ui`)

Depends on all other library crates. The GTK4/Relm4 application binary entry
point is `run()`. `WorkspaceStore` is the public type exported for integration.

`App` (app.rs) uses a `gtk::Paned` (subscriptions above, logs below); routing
and settings live in an `adw::PreferencesDialog` off the hamburger menu.
`ToastOverlay` wraps content; a bottom `gtk::ActionBar` shows connection status.
`geodata_service.rs` drives async geodata downloads off the GTK main thread via
`glib::MainContext::spawn_local` + `tokio::task::spawn_blocking`. `wizard.rs`
handles first-run onboarding.

### `run` (`v2ray-rs-run`)

Minimal setuid-root wrapper (~130 lines, only `libc` dep). On exec it:

1. Resolves the `v2ray-rs-bypass` system user via `getpwnam` (fails if UID/GID is 0).
2. Drops all privileges (`setgroups(0)`, `setresgid`, `setresuid`) and verifies.
3. Strips dangerous loader env vars (`LD_PRELOAD`, `LD_LIBRARY_PATH`, etc.) and
   replaces `PATH` with a safe hardcoded value.
4. `execvp`s the command from argv[1..].

Used only when "Run with bypass" is requested for xray TUN mode, so traffic
from app-launched tools exits via the `uidrange` policy rule rather than the
tunnel.

## Privileged / TUN model

TUN mode requires two non-GUI binaries to hold elevated capabilities:

| Binary | How privileged | Capabilities |
|---|---|---|
| xray / v2ray / sing-box | `setcap` | `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` |
| `v2ray-rs-netctl` | `setcap`, `0750 root:v2ray-rs` | `cap_net_admin+ep` |
| `v2ray-rs-run` | `chown root:v2ray-rs` + `chmod 4750` | SUID (drops to bypass user) |

All three are granted in one `pkexec` elevation: a shell script with all paths
passed as positional parameters (`$1`..`$5`) so a user-controlled path with
shell metacharacters cannot inject commands.

`privilege::file_caps_supported()` reads `/proc/self/mounts` and refuses to
grant on a `nosuid` mount, surfacing the manual `setcap` command instead.

The xray start sequence in `ProcessManager`:
1. `has_net_admin(binary)` — fail early if capability is missing.
2. Spawn backend process.
3. `wait_for_device(iface, 10s)` — poll `/sys/class/net/<iface>`.
4. `netctl xray-up` — program address and split routes.

Stop: SIGTERM lets xray close its TUN fd (kernel drops device-scoped routes),
then `netctl xray-down` runs as a safeguard. The same teardown runs on every
exit path that leaves the tunnel — a crash restart, a partial `xray-up`
startup failure, and the crash give-up — so host-wide policy rules never
outlive the device.

`tun_session.json` in `state_dir` is written at TUN connect and removed only on
a clean stop; it is kept on a crash give-up (Error) so the next launch runs the
route-recovery pass even if in-process teardown didn't complete. Its presence at
startup triggers that pass.

The grant flow resolves the helper and SUID wrapper to absolute paths beside the
running executable before elevating; it never passes a bare/relative name to the
root `setcap`/`chown`/`chmod`, which would otherwise resolve against the process
CWD.

## On-disk layout

```
$XDG_CONFIG_HOME/v2ray-rs/
  settings.toml              — AppSettings (backend, ports, TUN, DNS, ...)

$XDG_DATA_HOME/v2ray-rs/
  subscriptions.json
  routing_rules.json
  custom_nodes.json
  presets/                   — user-created presets

$XDG_CACHE_HOME/v2ray-rs/
  geodata/                   — .dat files (v2ray/xray); rule-sets/*.srs (sing-box)
  geodata-index/             — searchable index over geodata

$XDG_RUNTIME_DIR/v2ray-rs/
  v2ray-rs.lock              — exclusive flock for single-instance
  backend.pid                — PID + ownership record
  generated/
    {v2ray,xray,sing-box}.json  — generated backend config (0o600)

$XDG_STATE_HOME/v2ray-rs/
  instance.json              — InstanceStamp (version, first/last started)
  tun_session.json           — TUN active marker
  latency_snapshot.json      — per-node TCP latency cache
```

Dev mode (`AppProfile::Development`) uses the qualifier `v2ray-rs-dev`,
keeping its paths fully separate from production.

## Cross-cutting patterns

- **Tagged-enum serde.** `ProxyNode`, `TransportSettings`, `RuleMatch`,
  `SubscriptionSource`, `DnsRuleMatch` all use `#[serde(tag = "...")]` for
  self-describing JSON/TOML. This makes it possible to extend variants without
  breaking existing stored data.

- **Atomic file writes.** Every persistence write goes through `atomic_write()`
  in `crates/core/src/fs.rs`: `NamedTempFile` in the target directory →
  `sync_all` → `persist` (rename). JSON files serialised via `serde_json`,
  settings via `toml`.

- **Workspace deps.** All external crate versions declared once in the root
  `[workspace.dependencies]` and used with `.workspace = true`. Edition 2024
  throughout.

- **Broadcast for state fan-out.** `ProcessManager` publishes `ProcessEvent`
  on `tokio::broadcast` channels (state and logs kept separate). The UI and the
  tray subscribe independently; neither polls.

- **Blocking work off the GTK thread.** Subprocess and blocking calls that would
  otherwise freeze the event loop — the `pkexec` privilege grant, the orphaned-
  backend reap on Connect, geodata downloads — run via
  `glib::MainContext::spawn_local` + `tokio::task::spawn_blocking`.

- **No protocol logic.** The app generates a JSON config file and hands it to
  the backend via `binary run -c config.json`. Everything else — handshakes,
  encryption, tunnelling — is the backend's problem.
