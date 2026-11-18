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

Shared journal crate (not a privileged helper): core and netctl
  → tun-session (versioned ownership journal)
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
  → stdout/stderr → strip_ansi → backend.log + LogBuffer (ring, 10K lines) + broadcast channel
  → [xray TUN] wait_for_device() + netctl xray-up → routes programmed
  → ProcessState transitions broadcast to UI + tray
```

## Crates

### `core` (`v2ray-rs-core`)

Domain models, persistence, and config generation. Every other crate depends
on this; its single in-workspace dependency is `v2ray-rs-tun-session`, the
shared TUN journal schema with its owner-only atomic writer.

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

`write_config()` takes a `connected_subnets: &[ipnet::IpNet]` snapshot as its
fourth argument. Settings validation runs first and is unchanged, then — for
sing-box only, and only when `tun.enabled` — `apply_connected_subnets()` merges
the snapshot into the generated `tun` inbound's `route_exclude_address`. Other
backends and TUN-off ignore it; every other caller passes `&[]`.

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
plus Running → Starting for a supervised in-place restart, Starting → Stopping
so a stop can cancel a start or a pending respawn, and Running/Starting → Error.
`stop()` reaches Stopped from every state, including Error with no child.

Crash recovery: any exit while the state is still Running counts as an
unexpected crash — including a signal death (OOM/segfault/external kill, which
report no exit code on Unix). The manager respawns in a loop with a backoff that
scales with the number of recent crashes (2s, then 4s). A respawn reuses the
inputs the last start already checked — no version, capability, or config probe —
and a failed respawn counts as a crash. 3 crashes per 60s end in Error. Nothing
is torn down between a crash and its respawn, or on give-up: the tunnel's routes
stay in place so traffic cannot leak around it, and only `stop()` releases them.
A successful restart re-enters Running with no user-visible disconnect. Graceful
stop: SIGTERM → 5s timeout → SIGKILL. ETXTBSY on spawn is retried
(overlayfs/Docker edge case).

The UI adds a second, bounded layer: after the manager gives up (Error), it
schedules up to 3 whole-connection retries (5s apart, re-planning candidates),
reset on a successful connect and cancelled on an explicit Disconnect. Only the
connection task reports terminal states: one Stopped per requested stop and one
Error once the last candidate has failed. Failing over to the next candidate is
never reported as Stopped, and a failed candidate keeps its routing state until
a stop releases it.

`privilege.rs` reads binary file capabilities via `getcap` and grants them via
a single `pkexec` elevation (one shell invocation, paths passed as positional
args to avoid injection). `tun.rs` holds `TunRuntime` and the xray-specific
helpers: `wait_for_device()` polls `/sys/class/net/<iface>`, then invokes
`netctl xray-up`. sing-box programs its own routes via `auto_route`. `tun.rs` also
holds `connected_subnets()`, one unprivileged `getifaddrs()` snapshot of the
host's directly attached prefixes, and `helper_access()`, the single classifier
that decides whether the stored route helper is usable — both described under
"Privileged / TUN model" below. Each
call's bound is a named constant: the device wait uses `DEVICE_TIMEOUT`, the
`netctl xray-up`/`xray-down` calls and the startup `recover` pass use
`HELPER_TIMEOUT` — all 10s (`recover_tun_session` reuses `HELPER_TIMEOUT`; there
is no separate recovery constant). A timed-out call is killed and reaped, and
its output goes to the process log stream.

Connection setup and teardown are serialized by a single lock held for a
connection's whole lifetime, and the startup route-recovery pass takes the same
lock. This is load-bearing rather than defensive: a teardown removes the policy
rules and the device the session installed, and the `--ifindex` pin only proves
device identity, not that the rules belong to the newest session. Without the
lock a prompt reconnect sets up a session that the previous teardown then
deletes, leaving a live backend with no tunnel. For the same reason each
connection carries a generation number and the app ignores state reported by a
superseded one.

`PidFile` writes an ownership record (binary path + config path) and
`check_and_kill_orphaned()` kills stale processes from a previous run using
`kill(pid, 0)` as a liveness probe. `ProbeRunner` (`crates/process/src/probe.rs`)
runs a temporary backend instance for Real Delay measurement;
`measure_real_delay` in `crates/subscription/src/real_delay.rs` is its only user.

`crates/process/src/session/` houses the GTK-free headless connection
supervisor: `mod.rs` drives candidate failover, connect-time hostname DNS pinning
(`resolve_pins` / `apply_pins`), log batching, and `SessionEvent` emission;
`health.rs` monitors proxy reachability and DNS failure windows; `probe.rs`
executes plain HTTP health probes; `backend_warning.rs` classifies startup
warnings and fatal failures.

### `netctl` (`v2ray-rs-netctl`)

Standalone binary with `CAP_NET_ADMIN` set via `setcap`. It shares the small
`tun-session` crate with core, without depending on core's config or geodata code.

Automatic operations require `--journal FILE --session-id UUID`. The helper checks
an owner-only regular journal file, schema version, session identity, boot, and
network namespace before mutation. `reserve --iface IFACE` runs before backend
spawn. It refuses occupied routing state or interface names, creates the dummy
link `v2ray-rs-own`, records its index, explicitly sets its session alias, and
verifies the resulting identity. Creation alone does not preserve the alias on
the tested kernel. An interruption before tagging requires operator recovery.
Helper commands are serialized per journal by an owner-only lock file in the
journal's directory (mode 0600, `flock`); the kernel link preserves the
reservation between commands and across app exits. `--force-legacy` operator
commands stay uncoordinated with automatic sessions, as before.

The remaining subcommands retain the following routing policy:

- `xray-up --iface --addr [--addr6] [--ifindex N] [--bypass-uid] [--capture-dns] [--strict] [--exclude CIDR]...`: brings the
  link up, assigns the address, and installs the tunnel default route into table
  2023 plus the policy rules that steer traffic into it:
  - 9000 — `fwmark 0xff` → `main`, so xray's own sockets reach the real default
    instead of looping back through the tunnel. This fwmark rule is the only
    loop guard: xray outbounds are not bound to an interface, so they follow
    the whole `main` table, VPN routes included;
  - 9001 — `main` with `suppress_prefixlength 0`, keeping LAN and link routes;
  - 9002 — everything else → table 2023.

  `--bypass-uid` adds a `uidrange` rule at 8998 so traffic from the bypass user
  skips the tunnel. `--capture-dns` adds a udp/tcp `dport 53` pair at 8999,
  matched on `fwmark 0/0xff`: rule 9001 would otherwise send DNS to a resolver on
  the local subnet out the LAN route, leaving host name resolution as the one
  thing the tunnel never sees, while the fwmark match keeps the proxy's own
  upstream queries on the 9000 bypass. Without `--strict`, because the rules
  resolve to table 2023, an unclean exit leaves entries that match nothing
  rather than blackholing DNS.

  `--exclude` (repeatable, from `exclude_routes`) adds one rule per prefix at
  8997 sending that destination to `main`, ahead of every other xray rule, so
  excluded traffic leaves the real interface even on port 53 under
  `--capture-dns`. Prefix length 0 is refused, host bits are cleared, and at
  most 256 prefixes are accepted. The 8997 rules are replaced on every
  `xray-up`, after the rest of the setup succeeds; IPv6 prefixes are skipped on
  a host with IPv6 disabled.

  `--strict` (set from `strict_route`) is the kill-switch. It adds an
  `unreachable` default to table 2023 for IPv4 and IPv6 at the highest metric,
  behind the tunnel route, so once the device is gone traffic fails closed
  instead of falling through to `main`. It also installs the IPv6 9000–9002
  rules (and 8998 with `--bypass-uid`) without `--addr6`, so IPv6 with no tunnel
  address is refused rather than sent out the real interface. On a host with
  IPv6 disabled (no `/proc/sys/net/ipv6`) the IPv6 fallback and rules are
  skipped.
- `xray-down --iface [--ifindex N]` and `recover --xray --iface IFACE` remove only
  the exact journaled rules and routes, then the verified TUN device and reservation.
  An unexpected rule, route, duplicate, link index, or alias stops automatic cleanup.
  Failures retain the journal. A completed journal supports a harmless repeated stop.
- `recover --singbox` refuses destructive automatic cleanup because the helper did
  not establish ownership of sing-box's self-managed routes. Sing-box startup
  remains helper-free. Before a clean stop is reported, the process layer reads
  netlink state without privileges and verifies that the device, table 2022, and
  rules selecting it are absent. The generated config sets table 2022 explicitly.
  Remaining state or failed inspection retains the journal; it is never flushed
  by this verification.

`xray-up` requires a TUN device and the observed `--ifindex` when a journal is
supplied. The helper records device identity before assigning its session alias.
The reservation, observed index, and alias must agree before automatic teardown.
The session UUID also travels separately from the path, so a delayed stop cannot
operate on a newer journal that replaced the old file.

Before each route or rule addition, the helper writes intent. After the kernel
acknowledges it, the helper records the actual returned message and clears intent.
The stored representation is native netlink bytes, valid only within the recorded
boot and namespace. Resuming prunes acknowledged objects already absent from the
kernel; verified completion leaves an empty resource ledger. Automatic deletion compares complete recorded objects,
including multiplicity. A mutation whose effect cannot be established remains
pending. Kernel-normalized default-route attributes are handled during addition;
subsequent ownership checks use the observed object, not a broad priority match.

The authorization boundary remains the `v2ray-rs` group: the helper is
`root:v2ray-rs` mode `0750`, and the bypass wrapper is mode `4750`. Explicit
operator commands can use `--force-legacy` without a journal to inspect and
recover older routing state. That override retains the old broad cleanup behavior
and must be preceded by identifying the affected resources. Automatic desktop
recovery never supplies it. Other privileged software can change kernel state;
unexpected changes make automatic recovery stop.

### `tun-session` (`v2ray-rs-tun-session`)

Shared journal types and bounded, owner-only atomic file I/O. It records the
schema version, session UUID, boot UUID, namespace device/inode, controller PID
and start time, device and reservation indices, phase, exact owned resources,
and any pending addition. It has no protocol, GUI, or netlink dependency.

### `tray` (`v2ray-rs-tray`)

Depends only on `core` (no `process` dependency). Implements the
StatusNotifierItem protocol via `ksni`. Uses FreeDesktop theme-aware symbolic
SVG icons (three embedded: disconnected/connected/error). Falls back to ARGB32
pixmap rendering when the DE has no theme lookup. `TrayService::spawn()` /
`TrayService::spawn_with_data_dir()` listens on a
`tokio::sync::broadcast::Receiver<ProcessEvent>` (fed by the UI bridge) and
rebuilds the menu on state changes. `Notifier` sends desktop notifications via
`notify-rust`.

### `ui` (`v2ray-rs-ui`)

Depends on all other library crates. The GTK4/Relm4 application binary entry
point is `run()`. `WorkspaceStore` (`crates/ui/src/workspace.rs`) is the public
authoritative store holding `Arc` snapshots of settings, subscriptions, manual
nodes, routing rules, and latency data with a single serialized background
writer thread.

`crates/ui/src/app/` splits `App` across `mod.rs` (state and update loop),
`bootstrap.rs` (startup, recovery, tray wiring), `session_bridge.rs` (connect,
failover, reconnect policy, and applying `SessionEvent::NodePins`),
`config_regen.rs` (coalesced regeneration), and `dialogs.rs` (quit/disconnect
and toast plans). `connection.rs` is the thin Relm4 bridge starting
`v2ray_rs_process::session::spawn`. The main window uses a `gtk::Paned`
(subscriptions/nodes above, logs below); routing and settings live in an
`adw::PreferencesDialog` (`crates/ui/src/preferences/` covering `system.rs`,
`network.rs`, `routing.rs`, `tun.rs`, and `dns/{mod.rs,servers.rs,rules.rs,hosts.rs,effective.rs}`)
off the hamburger menu. `subscriptions/` is split into `mod.rs`, `rows.rs`,
`dialogs.rs`, and `probes.rs`; `ToastOverlay` wraps content; a bottom
`gtk::ActionBar` shows connection status. `geodata_service.rs` drives async
geodata downloads off the GTK main thread via `glib::MainContext::spawn_local`
+ `tokio::task::spawn_blocking`. `wizard.rs` handles first-run onboarding,
`nodes.rs` edits manual nodes, `logs.rs` renders backend logs, and `i18n.rs`
initializes `LC_MESSAGES` gettext translations.

The session supervisor subscribes to backend log lines before `start()`
returns, so lines printed during startup reach the logs page.
`crates/process/src/session/backend_warning.rs` matches each line against known
per-backend patterns (xray/v2ray `is deprecated`, xray REALITY `potential MITM or
redirection`, sing-box `WARN` + `deprecated`) and raises a toast quoting the
line (leading timestamp dropped, 160 chars max), once per pattern per
connection.

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

All three are granted in one `pkexec` elevation: a shell script with every path,
capability string, group name, account name, and recorded identity passed as
quoted positional parameters (eleven for `InPlace`, ten for `Relocate`) so a
user-controlled value with shell metacharacters cannot inject commands.

`privilege::file_caps_supported()` reads `/proc/self/mounts` and refuses to
grant on a `nosuid` mount, surfacing the manual `setcap` command instead.

### Attached networks are excluded from sing-box capture

`sing-box`'s `auto_route` captures everything that does not match a more
specific host route, which swallows return traffic for directly attached
networks — a Docker bridge on `172.17.0.0/16` whose replies must arrive on
that bridge. `process::tun::connected_subnets()` therefore collects one
unprivileged snapshot of the host's directly attached prefixes with a single
`getifaddrs()`: `IFF_UP` records only, loopback, point-to-point interfaces and
the configured TUN interface excluded by name, `/0`, `/32` and `/128` prefixes
skipped, missing or non-contiguous masks an `InvalidData` error naming the
interface. The result is sorted and deduplicated.

The connection task collects it once, through `spawn_blocking`, before the
candidate loop and after lifecycle locking, queued-stop handling, and orphan
cleanup, and reuses the one snapshot across every candidate and crash respawn.
A Stop that arrives during collection is honored before any candidate starts.
A collection or join failure ends the connection in one terminal
`ProcessState::Error` starting with `Collect connected subnets: `, before any
config write, marker, or backend spawn, and no other node is attempted.
`App::regenerate_config_with` repeats the same gate and, on failure, keeps the
previously generated file and reports
`Failed to regenerate config: Collect connected subnets: {err}`.

`core::config::singbox::apply_connected_subnets()` merges that snapshot into
`route_exclude_address` after user-settings validation: explicit entries keep
their order and any redundancy they carry, automatic prefixes are appended
sorted and deduplicated and skipped where they duplicate an explicit entry,
and only address families present in `tun.addresses()` are added. A `/0`
automatic prefix is refused through the same `ExcludeRouteCoversAll` error a
typed `/0` gets. `settings.tun.exclude_routes` is never modified — the
snapshot exists only in the generated file — and the 256-entry limit continues
to apply to explicit saved settings alone.

Two limits are load-bearing. The snapshot is taken at connect time and at a
disconnected regeneration, so a bridge created mid-session is excluded only on
the next connect; nothing monitors routes or interfaces. And only prefixes the
host is directly attached to are excluded, never blanket RFC1918 ranges: a
destination outside every attached prefix stays captured by `auto_route`.

sing-box keeps programming its own routes. No privileged helper call is added,
and a sing-box TUN start still runs without the route helper installed.

### Route-helper readiness is classified once

`tun::helper_access(path)` is the single classifier over the exact stored
`TunRuntime.helper_path` — the path the launch path would execute, never a
fresh resolution. It returns `Ready`, `Missing`, `EnrollmentRequired`,
`ReloginRequired`, `Permissions`, `NoExec`, or `ProbeFailed(detail)`, and
`ProcessError::from_helper_access()` maps every non-`Ready` state onto a
host-level error. `ProcessManager` and the TUN preferences page both consume it,
so startup and the preferences page can no longer disagree about the same
file. Only xray is gated: sing-box skips the helper gates entirely.

The states follow from the collected facts — mount state, ownership and mode
verdict, live primary-plus-supplementary membership, account membership from
`getgrouplist()`, and `access(X_OK)` — with failed lookups staying errors
rather than becoming negative answers. `ReloginRequired` is reserved for a
genuine stale session: the account is in `v2ray-rs` but the running process
does not hold the group yet. Because access is group-only, one log-out and
log-in after enrollment is still required, and only that case asks for it.
Wrong owner, mode, or group is `Permissions`; an absent dedicated group is
`EnrollmentRequired`; a `noexec` mount is `NoExec`; unreadable mount data or an
unexpected execution error is `ProbeFailed`, never a logout instruction.

### Grant refuses unsafe targets and verifies its own result

`privilege::grant()` refuses a root caller (`RootEnrollment`) before planning or
elevating, since enrolling uid 0 in the helper group grants nothing. Before
elevation, and again inside the elevated script before the first mutation, each
mutation target — the backend in both plans, plus the helper and optional
`v2ray-rs-run` wrapper — must be a non-symlink regular file, root-owned, free of
group/other write bits, and unchanged in its recorded decimal `device:inode`
identity; every canonical parent through `/` must be a root-owned,
non-group/other-writable directory. A user-owned or writable target is refused
with `UnsafeGrantTarget` rather than promoted through `chown`, because a
previously opened writable descriptor would survive that change.

The elevated script re-resolves the invoking account name and requires the same
nonzero UID recorded before the prompt, ensures the dedicated group (only a
`getent` missing-key status 2 may create it), enrolls with `usermod -aG`, then
sets `root:v2ray-rs` ownership before `0750` on the helper, `4750` on the
wrapper, and the capability sets last, because `chown` clears both file
capabilities and the setuid bit.

Access is group-only and no ACL is added. Operator-configured ACLs are
preserved, but a mode repair can enlarge an existing group mask and activate
dormant named entries, so `getfacl -a -c -n -E` is read (bounded, `LC_ALL=C`)
before elevation and repeated under it. Malformed output, a missing `getfacl`,
or a target group mask of `r-x` broader than the current one refuses before any
mutation; existing entries are never stripped.

After a successful elevation the result is verified by identity rather than by
exit status: the installed helper must be a regular file whose identity,
ownership, and `0750` mode match, with `cap_net_admin` present and the account
enrolled; an optional wrapper must match `root:v2ray-rs` `4750`; the backend
keeps its identity and capability. A relocating plan installs a fresh inode, so
its script prints one `helper-identity=<device>:<inode>` record that stdout
must contain and nothing else, and the installed helper is checked against that
new identity rather than the replaced one. Both a ready helper and a genuine
`ReloginRequired` mean provisioning succeeded — the running session is never
required to have acquired the group.

The xray start sequence in `ProcessManager`:
1. `helper_access(stored helper_path)` — refuse a missing, unenrolled, mis-owned,
   `noexec`, or unprobeable helper, and `has_net_admin(binary)` — fail early if
   the capability is missing.
2. Spawn backend process.
3. `wait_for_device(iface, 10s)` — poll `/sys/class/net/<iface>`; the device's
   kernel ifindex is captured and pinned onto `TunRuntime`.
4. `netctl xray-up --ifindex N` — program addresses, table-2023 default routes, and policy rules.

Stop: SIGTERM lets xray close its TUN fd (kernel drops device-scoped routes),
then `netctl xray-down --ifindex N` removes the policy rules and the strict
fallback, refusing the link if the name was reused by another device.
`stop()` is the only place that teardown runs. A crash, a failed respawn, a
partial `xray-up`, and the crash give-up all leave routing in place, so with
`strict_route` the host stays fail-closed until the session is stopped or
released.

Releasing the kill-switch when no stop follows is the app's job. When an Error
is not followed by a reconnect — the retry budget is spent, the Error arrived
while a stop was in progress, or a Quit is pending — the app runs the
marker-driven route-recovery pass under the connection lock and clears the
marker only after successful cleanup. Failed or timed-out recovery remains pending
across launches. Cleanup errors are reported rather than treated as success. Disconnect with no live connection but a marker present, and Quit with a
leftover marker, take the same path. Quit while Stopping waits for Stopped, and
Connect is refused while a release runs.

`tun_session.v1.json` in `state_dir` is prepared before config generation and
backend start. The process layer reserves xray state before spawn, then records
setup through the helper. A clean stop and verified recovery remove the journal;
failures keep it. Sing-box records launch intent before spawning and records
completion only after its child has stopped and unprivileged inspection verifies
absence. An unstarted Prepared record or an already verified Complete record can
be retired without a helper. An interrupted Applying record requires operator
recovery. A different application process cannot reuse an unfinished
journal without recovery. The old `tun_session.json` is read for diagnostics but
is never silently overwritten, converted into an ownership claim, or used for
automatic destructive cleanup; automatic recovery refuses it. A leftover legacy
marker is retired only by the operator: verify kernel cleanup with
`netctl recover --xray --iface <iface> --force-legacy` (or `--singbox` when the
marker names sing-box), then remove `tun_session.json` manually. The versioned
`tun_session.v1.json` must never be renamed to the legacy name. Corrupt
journals are retained.

Downgrading requires verified network cleanup first. Keep an owner-only backup
of pending journals and do not rename the versioned file to the legacy name.
Older releases cannot enforce the new ownership contract. A namespace mismatch,
missing reservation, or unknown mutation outcome requires operator intervention.

The grant flow resolves the helper and SUID wrapper to absolute paths beside the
running executable before elevating; it never passes a bare/relative name to the
root `setcap`/`chown`/`chmod`, which would otherwise resolve against the process
CWD.

### Name resolution under TUN

Under TUN the backend must never lean on the OS resolver: with sing-box's
`auto_route` or netctl's rules in place, an unmarked resolver socket is captured
into the tunnel. Both generators therefore emit a full DNS plane even when the
DNS feature is off. For xray that plane is tagged `dns-internal` and routed to
the first proxy outbound, which creates a circularity for the proxy's own
hostname. Three mechanisms break it, in order of consultation:

1. **Connect-time pin.** `crates/process/src/session/mod.rs` resolves every
   hostname-addressed node through the OS resolver before the route helper runs
   and writes the answers, both families, into `dns.hosts` of the effective
   settings (emitting `SessionEvent::NodePins` back to `app/session_bridge.rs`). The xray generator
   keeps the family its `queryStrategy` uses (`Prefer*` collapses to one there)
   and drops a domain left with nothing usable, since a `hosts` hit is an
   authoritative answer rather than a fall-through; sing-box takes both families
   in its `hosts` server.
2. **Bootstrap resolvers (xray).** A pair of scoped server objects sharing
   `tag: "dns-direct"` — plain UDP `1.1.1.1` first, DoH second, `skipFallback`
   on both and `finalQuery` on the last — with `domains` limited to the proxy
   and DNS-server hostnames. A routing rule at index 0 sends that tag to
   `direct`, whose `sockopt.mark` reaches the real default route through the
   pref-9000 rule. It sits ahead of the port-53 hijack rule, which carries no
   `inboundTag` and would otherwise swallow the UDP entry. sing-box has its own
   equivalent (`sys-dns-bootstrap` + `route.default_domain_resolver`).
3. **Direct detour.** A DNS server with `detour: "direct"` gets the same tag on
   xray; every other value is the default route. Names in `tun.exclude_domains`
   bind to that server when one exists.

sing-box differs in two ways worth knowing. Its dial-time resolution does not
consult `dns.rules`, so a pinned hostname reaches the dial only when the
outbound names the pin: a proxy outbound whose hostname is in `dns.hosts`
carries `domain_resolver: "hosts"`, which needs no network and cannot be
circular. And a direct detour is expressed by *omitting* `detour` — a DNS
server that carries none is not dispatched through the proxy chain, and naming
the settings-free `direct` outbound makes the daemon refuse to start even
though `sing-box check` accepts it. That gap between validation and startup is
why the sing-box tests also start the real binary.

Applicability of the related settings, per backend. `idle_timeout_secs`
reaches only the v2ray/xray family — emitted as `policy.levels.0.connIdle` —
and is inert under sing-box, whose streams keep the backend's stock idle
timeout. The per-server DNS `detour` is honored on xray only as `direct`, and
only under TUN, where the tag and its routing rule are gated (mechanism 3
above); sing-box honors proxy detours, rewritten to the first proxy outbound,
and expresses `direct` by omission as covered just above; v2ray ignores the
field, since the `dns-direct` tag exists only on xray's TUN path.
`tun.dns_hijack` distinguishes only `hijack`: `native` and `disabled` produce
identical configs, because `hijack` alone emits the `dns-out` outbound plus
the port-53 rule (xray) or the `hijack-dns` route action (sing-box).

`capture_dns` for netctl is derived from the generated config, not from the
lookup: it is armed only when every hostname node has an override xray can
answer with, and the `TunRuntime` is built from the same effective settings the
config was generated from.

### Relocating the helper off a `nosuid` mount

An AppImage's squashfs is mounted `nosuid` by the kernel and its mount point
changes on every launch, so a bundled `v2ray-rs-netctl` can never hold
capabilities where it sits. The grant therefore has two plans, chosen by
`privilege::plan_grant()`:

| Plan | When | What the elevation does |
|---|---|---|
| `InPlace` | the bundled helper's mount honours file capabilities | `setcap` where the binaries already are |
| `Relocate` | it does not | copy the helper to `/usr/local/lib/v2ray-rs/`, then `setcap` there |

`tun::RELOCATE_DIR` is `/usr/local/lib/v2ray-rs`. It is root-owned and off
`$PATH`, so the invoking user cannot tamper with the directory and a relocated
copy cannot shadow a distribution package's `/usr/bin` binaries. `~/.local/lib`
is not an option: a user-writable directory breaks the trusted-path-prefix
assumption that file capabilities rest on.

`tun::pick_bin()` decides which copy runs. A bundled copy that can hold
capabilities always wins, so a stale relocated copy never shadows a correct
package install; a relocated copy comes next; a capability-less bundled copy is
still preferred over `$PATH`, so the failure is a visible permission error
rather than a silent jump to an unrelated binary.

**Root cannot read the AppImage it is installing from.** A squashfuse mount
carries no `allow_other` (an unprivileged mount cannot set it without
`user_allow_other` in `/etc/fuse.conf`), and the kernel's
`fuse_allow_current_process()` grants root no exemption — only the mounting uid
may access the mount, and `pkexec` stays in the same mount namespace. A `cp`
under the elevation would therefore fail with `EACCES` on exactly the file it
exists to install. So the helper is **streamed in on stdin**: the unprivileged
process, which *is* the mounting uid, opens the file and `run_elevation()`
passes the descriptor as the elevated shell's stdin. Root only ever writes the
destination.

Constraints on the relocating elevation:

- The payload source comes from `/proc/self/exe` only, never `$APPDIR`. Taking
  it from the environment would let anything able to set that variable — a
  rewritten desktop entry, say — choose what root installs.
- `privilege::validated_source()` stats the path as given (before
  `canonicalize` resolves the final component away), canonicalizes it, and
  refuses anything that is not a regular file beside our own executable. Opening
  it here rather than naming it to root also closes the window between checking
  the path and reading it.
- The destination directory is created inside the elevation with an explicit
  `chown root:root` and `chmod 0755`, never relying on the inherited umask.
- `chown` runs before `chmod`/`setcap`, since it clears both file capabilities
  and the setuid bit. The destination mode is set absolutely, and `rm -f` before
  the redirect forces a fresh inode, so no attribute of any previous file at
  that path survives.
- The result is `0750 root:v2ray-rs`, using the same dedicated system group the
  package hook creates. The invoker's *primary* group is deliberately not used:
  where a distribution shares one across accounts (gid 100 `users`), that would
  hand every local user a `cap_net_admin` binary. The elevation adds the invoker
  to the group, so the first grant needs a re-login. That stale-session case is
  `HelperAccess::ReloginRequired`, and both startup and the preferences page
  read it from the same classifier rather than treating every execution denial
  as one.
- The grant rewrites the destination unconditionally, so re-granting is the
  universal repair. `tun::helpers_stale(installed)` compares the size and mtime
  of the given installed helper against the
  bundled copy: the destination's mtime is the time of the last grant, so a
  newer source is exactly the "needs re-granting" condition. It is pure metadata
  because the preferences page polls it on every settings change, on the GTK
  thread — an earlier version spawned `netctl --version` there with no timeout.
- After the elevation, `grant()` confirms the newly installed helper by
  identity, ownership, mode, capability, and enrollment; a relocated copy is a
  fresh inode, so it is matched against the `helper-identity` record the
  elevated script prints rather than the replaced inode. A `getcap` that cannot
  be run is not treated as
  failure: it may simply be absent from the user's `PATH` while the elevation's
  `setcap` worked fine.
- `preflight_targets()` checks the *destination* directory's mount, not the
  bundled copy's — the latter is known `nosuid`, which is the whole reason for
  relocating.
- A relocating plan also carries the backend's identity and the invoking UID,
  plus the destination helper's prior `device:inode` or the literal `absent`,
  all re-checked under elevation before the destination is created. Missing
  destination directories are created only below a validated protected parent,
  never adopting an attacker-owned one. Membership is enrolled with
  `usermod -aG` unconditionally. The wrapper is not relocated.

`v2ray-rs-run` is deliberately **not** relocated and not shipped in the
AppImage. It drops to the `v2ray-rs-bypass` system user
(`crates/run/src/main.rs`), which only a distribution package creates, so a
setuid-root copy outside one could not function and would be pure attack
surface. The TUN page hides *Run with bypass* when that user is absent.

Two residual risks are accepted rather than fixed: a same-user process that
overwrites the `.AppImage` before a grant (indistinguishable from the existing
risk of running any downloaded AppImage and then authenticating a root prompt —
only bundle signature verification would address it), and the TOCTOU window
between the Rust-side validation of the streamed payload source and the
elevation reading it — the source is the caller's to read, not a root-owned
file, and it carries no recorded identity. The mutation targets it does guard —
helper, wrapper, backend, destination, and their parent directories — are
checked twice, before elevation and again inside it.

## On-disk layout

```
$XDG_CONFIG_HOME/v2ray-rs/
  settings.toml              — AppSettings (backend, ports, TUN, DNS, logging, ...)

$XDG_DATA_HOME/v2ray-rs/
  subscriptions.json
  subscription_blobs/        — persisted config-bundle JSON subscription files (0o600)
  routing_rules.json
  custom_nodes.json
  presets/                   — user-created presets

$XDG_CACHE_HOME/v2ray-rs/
  geodata/                   — .dat files (v2ray/xray); rule-sets/*.srs (sing-box)
  geodata-index/             — searchable index over geodata

$XDG_RUNTIME_DIR/v2ray-rs/     (else ~/.local/share/v2ray-rs/runtime/)
  v2ray-rs.lock              — exclusive flock for single-instance
  backend.pid                — PID + ownership record
  generated/
    {v2ray,xray,sing-box}.json  — generated backend config (0o600)

$XDG_STATE_HOME/v2ray-rs/      (else ~/.local/share/v2ray-rs/state/)
  instance.json              — InstanceStamp (version, first/last started)
  tun_session.v1.json        — versioned TUN ownership journal
  tun_session.json           — legacy marker, if present
  latency_snapshot.json      — per-node TCP latency cache
  logs/
    v2ray-rs.log             — app log (rotated: .1–.3 kept at 5 MiB each)
    backend.log              — backend output + session/exit records (same)
```

`state_dir` and `runtime_dir` follow `$XDG_STATE_HOME` and `$XDG_RUNTIME_DIR`
with the same qualifier, falling back to `<data_dir>/state` and
`<data_dir>/runtime` when the variable is unset
(`crates/core/src/persistence/mod.rs`).

Dev mode (`AppProfile::Development`) uses the qualifier `v2ray-rs-dev`,
keeping its paths fully separate from production.

### File-only settings

Four settings live only in `settings.toml` — the Preferences UI has no widget
for any of them:

- `tun.address_v6` — optional IPv6 tunnel address for TUN mode.
- `auto_update_geodata` — enables the background geodata refresh.
- `geodata_update_interval_secs` — geodata refresh interval in seconds
  (floored at 60 by the refresh service).
- `backend.config_output_dir` — writes the generated backend config to this
  directory instead of `<runtime_dir>/generated/`
  (`crates/core/src/config/writer.rs`).

## Logging

The app keeps two size-rotated logs under `<state_dir>/logs`
(`$XDG_STATE_HOME/v2ray-rs/logs/`, else `~/.local/share/v2ray-rs/state/logs/`).
Each rotates when the next record would push the active
file past 5 MiB: the oldest generation is deleted, `.2`→`.3`, `.1`→`.2`, the
active file becomes `.1` (newest first), and a fresh file is opened — four
files, ~20 MiB per log name. Both files are created 0o600 in a 0o700
directory and appended to across runs.

- **`v2ray-rs.log`** — the app's own log records, mirrored to stderr.
  Installed right after path resolution, before the first persistence call,
  so early startup failures land in the file; an unwritable file degrades to
  stderr-only. `V2RAY_RS_LOG` selects the level case-insensitively
  (`trace`, `debug`, `info`, `warn`, `error`; default `info`; an invalid
  value falls back to `info` with one stderr note).
  Caveat: at `debug`/`trace` levels, library logs (hyper/reqwest) may include
  request URLs that carry provider tokens; the file is user-private (0600).

- **`backend.log`** — everything the backend process prints, with each line
  tagged by its stream (`stdout`/`stderr`) or the route-helper tag. The
  connection task opens one shared writer, so failover between candidates
  appends to the same file. Interspersed diagnostics records: one `session`
  line per launch (`backend=… version=… node=… tun=… utc_offset=±HH:MM`; the
  version probe runs at most once per manager) and one `exit` line per exit
  (exit code or signal, whether the stop was requested, crashes in the restart
  window, and the backend's last output line). `core::ansi::strip_ansi`
  removes CSI and OSC escape sequences from backend, helper, and version
  output before it reaches the file, the `LogBuffer`, or the log channel.

- **Backend verbosity** — `AppSettings.logging` (`LoggingSettings`), edited in
  Preferences → System → Diagnostics, drives the generated config's `log`
  object. `backend_level` (`error`, `warning`, `info`, `debug`; default
  `warning`) sets xray/v2ray `loglevel` and sing-box `level` (`warn` for
  warning). `connection_log` (default off) applies to xray/v2ray only: off
  emits `access: "none"`. The switch is insensitive for sing-box, which writes
  connection lines at `info` and `debug`.
  Probe configs keep a fixed `warning` level.

## Cross-cutting patterns

- **Tagged-enum serde.** `ProxyNode`, `TransportSettings`, `RuleMatch`,
  `SubscriptionSource`, `DnsRuleMatch` all use `#[serde(tag = "...")]` for
  self-describing JSON/TOML. This makes it possible to extend variants without
  breaking existing stored data.

- **Atomic file writes.** Every persistence write goes through `atomic_write()`
  in `crates/core/src/fs.rs`: `NamedTempFile` in the target directory →
  `sync_all` → `persist` (rename). JSON files serialised via `serde_json`,
  settings via `toml`. Exception: TUN journals go through the
  `v2ray-rs-tun-session` owner-only atomic writer.

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

Explicit start renews a verified completed TUN journal and clears the previous
device index. Crash respawn retains the unfinished session identity. Journaled
route and rule additions reject EEXIST because a concurrent creator's resource
cannot establish ownership.
