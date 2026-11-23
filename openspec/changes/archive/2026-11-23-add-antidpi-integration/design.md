## Context

- `crates/antidpi` ships the CLI (`crates/antidpi/src/main.rs`): `check --strategy …`
  (parses strategy chains, then probes capabilities and reports unmet sysctls
  without changing them), `run --queue N --strategy … [--hosts file] [--rotation
  …] [--no-fw]` (installs the firewall rules when not `--no-fw`, runs the
  NFQUEUE loop, removes rules on SIGTERM/SIGINT — the signal handler flips a flag
  the loop polls, `main.rs:180-206`), one-shot `install-rules --queue N
  [--strategy …] [--hosts file] [--owner PID:START]`, and `remove-rules [--sweep]
  [--owner PID:START]`. `install-rules` validates every strategy and the hostlist
  before any kernel mutation. `run` prints `v2ray-rs-antidpi: ready queue=<N>` on
  stdout only after the queue bind succeeds. `--no-fw` exists precisely for
  "rules managed externally" (`main.rs:51-53`). The default queue (`fw::DEFAULT_QUEUE = 200`,
  `fw.rs:15`), fwmark, and the keenetic-default chain strings (`DEFAULT_CHAINS`,
  `main.rs:12-16`) live in the crate. The strategy DSL is documented in
  `docs/antidpi-registry.md` §3/§5.
- The process layer owns the privileged-helper pattern: `resolve_bin`/`pick_bin`
  (`crates/process/src/tun.rs:80-111`) picks bundled → relocated
  (`RELOCATE_DIR /usr/local/lib/v2ray-rs`) → `$PATH`; `run_helper`
  (`tun.rs:769-785`) bounds and captures helper runs; `launch()`
  (`manager.rs:767-834`) sequences reserve → spawn → `wait_for_device` +
  `xray_up`; `teardown_tun()` (`manager.rs:1176-1195`) owns TUN teardown;
  `log_helper` (`manager.rs:1197-1218`) streams helper output into the "helper"
  log stream, the `LogBuffer`, and `ProcessEvent::LogLine`.
- `privilege.rs::grant(backend, helper)` (`privilege.rs:291`) plans InPlace or
  Relocate, preflights `nosuid` mounts per target (`preflight_targets`,
  `privilege.rs:1175-1189`), re-validates every mutation target inside the
  elevated script (`grant_script`, `privilege.rs:999-1152`), and verifies the
  installed result by identity afterwards. `HELPER_CAPS = "cap_net_admin+ep"`
  (`privilege.rs:16`); `getcap_has_cap` (`privilege.rs:1155-1167`) does token
  matching over `getcap` output.
- Session wiring: `build_tun_runtime` (`session/mod.rs:1201-1237`) builds the TUN
  runtime from effective settings; `ProcessManager::with_tun` attaches it at the
  `configure()` call (`session/mod.rs:566-579`).
- Settings/restart: `TunConfig` + `TunConfigWire` (`models/tun.rs:34-172`) is the
  wire pattern; `RuntimeConfigSnapshot::diverges_from`/`restore_settings`
  (`runtime_snapshot.rs:30-69`) drives the restart banner; `preferences/tun.rs`
  is the page pattern, with capability probing (`tun.rs:640-694`) and the grant
  button (`tun.rs:701-727`); pages register in `preferences/mod.rs:100-137` and
  export a page name for restart-banner navigation (`TUN_PAGE_NAME`, used at
  `app/mod.rs:1751`).
- Packaging: `pkg/archlinux/PKGBUILD` builds `-p v2ray-rs-ui -p v2ray-rs-netctl
  -p v2ray-rs-run`; `v2ray-rs.install` provisions the helpers (group, `chown
  root:v2ray-rs`, `chmod 0750`, `setcap`); `pkg/archlinux/bin/PKGBUILD` installs
  prebuilt binaries; `Makefile` `dist` builds musl helpers and calls
  `scripts/stage-dist.sh`; `scripts/install.sh` carries the `BINS` list
  (`install.sh:48`) and the TUN instructions; Debian metadata in
  `crates/ui/Cargo.toml:38-54`.

## Goals / Non-Goals

**Goals:**
- One toggle in the app that starts and stops the engine with the session, with a
  correct rules lifecycle: installed before any traffic flows, removed on stop,
  never leaking.
- One `pkexec` grant that covers the backend, `netctl`, and `antidpi` in a single
  prompt.
- Strategy strings stay a power-user feature with helper-side validation; the app
  never reimplements the DSL.
- Ship the binary through every existing distribution channel.

**Non-Goals:**
- No strategy DSL parsing in the app; no strategy preset picker in v1 (the
  helper's built-in `DEFAULT_CHAINS` is the default; editing is free-form strings
  parsed by the helper at connect time, before backend spawn).
- No rotation parameter exposure — the helper default `fails=2:time=300` applies;
  a settings field can come later.
- No hostlist editor — just a path to an externally managed file.
- No L2 (local stream-level) fallback layer; NFQUEUE L1 only (`docs/antidpi-registry.md` §2).
- Windows/macOS and per-app scoping. The firewall backend is nf_tables only: the
  helper writes its own table over netlink and never executes `iptables`,
  `ip6tables`, or `nft`.

## Decisions

**1. The engine is a child of `ProcessManager`, sequenced inside `launch()`.**
Per launch (`manager.rs:767-834`): (a) TUN reserve (existing); (b) one-shot
`install-rules --queue N --strategy … --hosts … --owner …` before `try_spawn()` —
the helper parses every strategy chain and loads the hostlist first, then writes
its nftables table through its own netlink client. The table is
`v2ray_rs_<euid>`, one per address family, so two local accounts can never
displace or delete each other's rules. Rules exist before any traffic
flows; a malformed chain, a missing hostlist, a capability problem, or a netlink
failure fails the start before any backend spawn. (c) backend spawn + pid + output
capture (existing); (d) `wait_for_device` + `xray_up` (existing, xray TUN only);
(e) engine child `run --queue N --no-fw --strategy … [--hosts …]`, spawned after
`xray_up` so the tunnel route state settles first, or directly after the backend
spawn when TUN is off. The engine gets `--no-fw` because the app owns rule
installation, so the engine never mutates the firewall and stop stays
deterministic. The manager reports the start as successful only after the child
prints its stdout readiness line following the queue bind; an exit before that
line is a startup failure carrying the child's status and last stderr line, while
an exit after it is the existing unexpected-engine-exit signal. If a live engine
child survives from the previous launch attempt (crash respawn), launch reuses it
and skips both `install-rules` and a second spawn, so the queue is never
double-bound.
*Alternative:* let `run` install its own rules and skip the one-shot — rejected:
with rules at step (b) traffic is protected even during the engine's startup
window, install-failure reporting stays synchronous, and strategy errors surface
before the backend starts.

**2. Stop kills the engine, then removes only the session's own rules.** SIGTERM
to the engine → its handler flips the shutdown flag → the loop exits (with
`--no-fw` the engine never mutates the firewall); SIGKILL fallback after a
bounded window; then a one-shot `remove-rules --owner …` deletes the recorded
nftables tables. The helper first verifies the journal's controller identity,
PID namespace, boot id, and network namespace, so a stop can never delete
another session's table; a numeric PID is attributed only inside the PID
namespace the journal records, and every ownership decision refuses a
journal whose PID namespace differs from — or predates evidence of — this
one. Removal is not best-effort bookkeeping: a failed or partial removal
keeps the journal (phase `cleaning`) and returns an error, and the manager
keeps its cleanup obligation so a later stop retries it; an ambiguous reply
or an exhausted exchange budget is a distinct timeout error that also keeps
the journal. `--sweep` is recovery only for a provably dead controller in
the same boot, network namespace, and PID namespace; a live controller never
sweeps after engine death — it removes its own recorded rules. A prior
boot's stale record is retired only by `--sweep`, and only once both owned
table names are confirmed absent in the current namespace; cross-epoch table
deletion stays forbidden. A same-boot journal from another network namespace
is refused and names that namespace; records without PID-namespace evidence
are preserved for operator inspection; a table with no journal is reported,
never deleted. An engine orphaned by a crashed app is healed on the next
start: a same-epoch journal whose owner is dead is recovered before the new
install, while any other journal is refused. Fresh intent requires both
owned table names absent and creation is exclusive — a same-named table is
never adopted, and a confirmed `EEXIST` drops only the attempted family's
unacquired obligation while ambiguous replies keep every obligation; a
verified same-owner reinstall performs no kernel mutation and repairs only a
lost family flag. All of this serializes on one per-effective-UID,
per-network-namespace transaction lock (`/run/user/<euid>`, or
`/run/v2ray-rs-antidpi` for euid 0) independent of any journal path
override, fail-closed when that directory is unusable. Readiness
verification, queue bind, and teardown share that lock; netlink exchanges
share one monotonic budget (five seconds across an install, three seconds
reserved for rollback removals, eight seconds for a removal), enforced
before every socket send and wait, and no exchange resets it.

**3. Helper resolution reuses the existing resolver, and execution uses its
absolute result.** `antidpi_path()` = `resolve_bin(ANTIDPI_BIN)` with
`ANTIDPI_BIN = "v2ray-rs-antidpi"` — bundled → relocated → `$PATH` ordering as
for the route helper (`tun.rs:99-111`), so AppImage installs land on the
relocated copy and distribution installs on `/usr/bin`. The `$PATH` fallback is
resolved to an absolute executable before it is inspected or executed; a bare
name is never classified or spawned. Every helper invocation — install, remove,
engine — receives the same absolute path, so the copy that passed the access and
capability checks is the copy that runs, regardless of later `$PATH` order. The
process environment is never mutated. Access classification reuses
`helper_access(path)` (`tun.rs:213`) — one classifier for preflight and the
preferences page, per the tun-mode spec's "Startup and preferences agree"
requirement.

**4. The grant covers all privileged binaries in one elevation.** `grant()`
gains an antidpi target as `Option<&Path>`, skipped when the binary is absent so
environments that build without the antidpi crate still grant TUN.
`ANTIDPI_CAPS = "cap_net_admin,cap_net_raw+ep"` (raw sockets for re-injection +
`SO_MARK` for the loop guard — the same pair the engine's `probe_caps` demands).
InPlace plan: `check_file`/`check_parents`/`check_acl` → `chown root:v2ray-rs` →
`chmod 0750` → `setcap` with the antidpi caps; ownership precedes mode precedes
capabilities, as the existing script comments require. Relocate plan: both
payloads (route helper and anti-DPI helper) are piped on stdin in that order,
split by their declared byte counts. Each destination's prior `device:inode`
identity and protected parents are verified before any mutation; each payload is
staged into a fresh protected temp file in the destination's directory, its
length is checked, and only then is it published with an atomic rename. Both
publishes must succeed before the script emits the helper identity line and then
the anti-DPI identity line; any failure unlinks the staged files and leaves the
existing privileged binaries byte-identical. `preflight_targets` gains the
antidpi path/caps pair so a `nosuid` mount refuses before the polkit prompt.
*Alternative:* a separate `grant_antidpi()` with its own prompt — rejected: two
polkit prompts for one feature, and the plan/script machinery is already
parameterized per target.

**5. Settings mirror the `TunConfig` wire pattern.** `AntiDpiConfig { enabled,
queue, strategies: Vec<String>, hostlist_path: Option<String> }` with
`AntiDpiConfigWire` serde defaults; a legacy `settings.toml` without `[antidpi]`
loads with defaults (`enabled = false`). `validate()` is shape-only (queue ≠ 0,
strategy count and length bounds, non-empty strings, non-empty hostlist path)
— the UI does not run the helper. At connect, the helper parses the chains in
`install-rules` before backend spawn. The standalone `check` command also parses
chains on demand. Core does not depend on the helper crate. An empty
`strategies` list means the helper's built-in default chains (the process layer
omits the `--strategy` flags).

**6. The engine works with every backend, TUN on or off.** NFQUEUE sees OUTPUT
egress regardless of backend or TUN, so v2ray — which has no TUN support —
benefits too, and the tunnel handshake itself (the proxy server's ClientHello on
port 443) is covered once the engine is ready (`docs/antidpi-registry.md` §2 L1).
`build_antidpi_runtime` gates only on `settings.antidpi.enabled`. The table
matches only TCP and UDP port 443; plain HTTP on port 80 is not covered, and the
readiness handshake means the very first packets of a connection attempt that
races the engine start can pass unmodified. DNS is unaffected: the rules exclude
the fwmark and split-DNS handling stays with the existing DNS configuration.
With a hostlist configured, only TLS SNI and HTTP `Host` can be matched; QUIC
Initials carry no hostname, so they are not treated in hostlist mode.

**7. UI mirrors the TUN page.** `build_antidpi_page` with an enable `SwitchRow`,
queue `SpinRow`, a multiline strategy editor (one chain per line, committed on
focus-out), and a hostlist `EntryRow`. The editor is shape-only: the page calls
`AntiDpiConfig::validate()` and never invokes the helper's `check`; the one-shot
`install-rules` is the authoritative parser and now validates before the backend
spawns. An invalid edit is rejected in full with one localized error and the
editor is restored to the saved list; an empty editor means "use the helper's
built-in defaults". The page shows a capability hint row and points at the shared
"Grant TUN privileges" action on the TUN page — it has no inline grant button.
The TUN page's capability row probes the route helper and the anti-DPI helper and
offers the same grant when either needs repair, so an anti-DPI-only credential
problem still has a visible fix. `ANTIDPI_PAGE_NAME` is exported for
restart-banner navigation like `TUN_PAGE_NAME`. Changing any anti-DPI field while
connected raises the restart-required banner automatically through snapshot
divergence — no extra UI gating; the next session is built from the persisted
settings.

**8. Capability detection extends the existing getcap probe.** A caps-present
check for the antidpi binary requires both `cap_net_admin` and `cap_net_raw` to
be present in the capset with both the permitted and effective flags, parsed from
the flags after `=`/`+` rather than by name-token presence; a permitted-only file
does not pass, because the exec'd process would have no effective capability.
Effective UID 0 skips the probe, same rule as TUN.

**9. Kernel sysctls are reported, not silently changed.** `check` is
observation-only: it names each unmet key (`nf_conntrack_acct`,
`nf_conntrack_tcp_be_liberal`) and returns an error. `run` and `install-rules`
keep a best-effort write and log every key they could not set; a non-root helper
cannot pass the `/proc/sys` DAC check even with `CAP_NET_ADMIN`, so the warning
is the truthful outcome. The sysctl key validator accepts the real key alphabet
(letters, digits, `_`, `-`, `/`) and rejects empty components and traversal.
The helper never writes sysctls: automatic connection attempts and
`install-rules` only warn on unmet values, keeping the existing nonfatal
policy without changing namespace policy implicitly.

## Risks / Trade-offs

- **A crashed app leaves the engine running and rules installed.** Mitigation:
  the journal records the controller, PID namespace, boot, network namespace,
  and session; the next connect recovers a same-epoch journal only when the
  controller is provably dead in that PID namespace, a live foreign owner is
  refused, and a table with no journal is reported rather than deleted. A
  failed removal keeps the journal and the manager's obligation.
- **The nftables encoder is hand-written and cannot be exercised without
  `CAP_NET_ADMIN`.** Mitigation: constants and layouts come from the installed
  UAPI headers, encoding is compared byte-for-byte against a captured `nft` batch,
  and a `privileged-tests` case exercises install/remove inside a disposable
  namespace. Kernel-level proof remains a release gate that must run in a
  disposable VM.
- **The engine queues the app's own proxy-connection traffic.** Intended (it
  protects the tunnel handshake), but a bad strategy could stall the tunnel.
  Mitigation: strategies are user-supplied and parsed before the backend starts;
  rotation auto-advances on failure; disconnect removes only the session's table.
- **The grant script's positional arguments grow.** Mitigation: indexed
  positionals, a `sh -n` check of the generated script, and argv-order assertions
  in the grant tests.
- **Queue-number collision** with another local NFQUEUE consumer (e.g. a
  separately installed zapret). Mitigation: surfaced as the helper's queue-bind
  readiness failure; the queue is user-configurable.
- **A second setcap'd binary widens the local attack surface.** Mitigation: same
  as `netctl` — `root:v2ray-rs` `0750` group-only execute, dedicated system
  group, identity-revalidating elevation script, and the capability pair is the
  minimum the engine needs.

## Migration Plan

- `[antidpi]` defaults to disabled; a legacy `settings.toml` loads unchanged (no
  migration step).
- The antidpi binary is additive to packaging (stage 6 of the task list).
- **Rollback:** disabling anti-DPI restores today's behavior exactly; no
  persisted data shape is removed or repurposed.

## Security-scan constraints

The change previously spawned the engine by its literal name and prepended the
resolved directory to the process `PATH`, because one implementation
environment's pre-tool-use hook rejected `Command::new` with a non-literal
argument. That workaround is removed: the manager already spawns the backend and
runs every helper one-shot by absolute path in this repository, and the engine
now does the same (`Command::new(&rt.helper_path)`), with no process-global
environment mutation. The helper's own firewall writes use a hand-written
nf_tables netlink client — no `Command`, no shell, no system firewall binary — so
the privileged process never transfers its capabilities across an exec.
