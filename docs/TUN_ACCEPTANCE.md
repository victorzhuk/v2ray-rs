# TUN acceptance runbook

This runbook collects acceptance evidence for two already implemented features:

1. sing-box TUN automatic exclusions for attached networks.
2. xray TUN privilege grant and post-grant authorization.

It is a procedure, not a design document. Every claim in an evidence report must
come from a command whose output is stored.

## Contents

- [Scope and safety](#scope-and-safety)
- [Prerequisites](#prerequisites)
- [Guest fixture layout](#guest-fixture-layout)
- [Evidence collection commands](#evidence-collection-commands)
- [Snapshot and evidence ordering](#snapshot-and-evidence-ordering)
- [Automated routing harness](#automated-routing-harness)
- [Manual routing scenarios](#manual-routing-scenarios)
- [xray grant and authorization scenarios](#xray-grant-and-authorization-scenarios)
- [Required matrix](#required-matrix)
- [Evidence table and completion](#evidence-table-and-completion)
- [Bounds](#bounds)
- [Retry policy](#retry-policy)

## Scope and safety

Every account, mount, capability, package, and routing mutation in this runbook
happens inside an explicitly authorized disposable Arch Linux desktop VM. Never run
any step on a developer workstation or on production. Steps that create users, add
group members, change file capabilities, mount with `noexec`, swap package files,
or move routes destroy the guest. The VM is expected to be destroyed.

The snapshot must be restorable. Restore is part of the procedure, not a recovery
measure you reach for afterwards. Keep an independent administrative console
available (a second console, a serial console, or an out-of-band hypervisor
console) so a failed GUI cannot remove your recovery access.

Never map writable host directories into the VM during adversarial privilege
scenarios. A shared folder lets a host process replace a target file between the
identity check and the elevation, which converts a valid race test into a host
compromise. Guest-local copies only.

Record the tested Git revision and the digests of every artifact used:

```sh
timeout 10s git -C /srv/src/v2ray-rs rev-parse HEAD
timeout 10s git -C /srv/src/v2ray-rs status --porcelain
timeout 10s sha256sum -b /srv/artifacts/v2ray-rs-x86_64.AppImage
```

A released artifact does not prove unpushed source changes. If `git status`
reports a dirty tree, the build under test is not the revision named in the
evidence table. Rebuild from a clean tree or record the tree state as part of the
artifact identity.

## Prerequisites

Record each row as present, missing, or unverified. Do not reuse an earlier claim
about an unavailable host tool.

| Requirement | How to check | Status (present/missing/unverified) |
|---|---|---|
| Disposable Arch Linux desktop VM | hypervisor inventory | |
| Restorable snapshot | snapshot tool, one restore rehearsal | |
| Independent admin console | console screenshot from outside the guest desktop | |
| Working polkit authentication agent | agent process is running for the desktop session, and the first grant row shows a real prompt | |
| Package install of both Arch recipes | `getent group v2ray-rs`, `id v2ray-rs-bypass` | |
| Current-revision packaged build | `timeout 10s v2ray-rs --version` plus recorded digest | |
| Current-revision FUSE-mounted AppImage | real FUSE mount, not `APPIMAGE_EXTRACT_AND_RUN` | |
| xray binary | `timeout 10s /usr/bin/xray version` | |
| `getfacl` / `setfacl` | `command -v getfacl setfacl` | |
| `getcap` / `setcap` | `command -v getcap setcap` | |
| `ip` | `command -v ip` | |
| `unshare` | `command -v unshare` | |
| `nsenter` | `command -v nsenter` | |
| `runuser` | `command -v runuser` | |
| `python3` | `command -v python3` | |
| `tcpdump` | `command -v tcpdump` | |
| Docker plus an already-available image with client and server tools | `timeout 10s docker image ls` before the scenario | |
| Controlled DNS resolver answering `tun-accept.test` | `timeout 10s dig @<resolver> tun-accept.test A AAAA` | |
| Controlled TCP responders with nonce echo | `timeout 10s nc -z <responder> <port>` | |
| Outside client able to reach a published guest port | documented route to the guest | |
| sing-box 1.13.0 at `/usr/bin/sing-box` | `timeout 10s /usr/bin/sing-box version` | |
| sing-box 1.13.14 at `/usr/bin/sing-box` | `timeout 10s /usr/bin/sing-box version` | |
| Guest with usable kernel IPv6 | `timeout 10s ip -6 route get 2001:db8::80` | |
| Guest without usable kernel IPv6 | kernel IPv6 state recorded before launch | |

A missing prerequisite blocks the affected scenario. It never permits a passing
skip. Record the row as BLOCKED with the reason.

The package recipes are `pkg/archlinux/PKGBUILD` and the `PKGBUILD-bin` produced
by the release job. Their shared install hook is
`pkg/archlinux/v2ray-rs.install`, which creates the system group `v2ray-rs` and
the system user `v2ray-rs-bypass`, then applies `0750 root:v2ray-rs` and
`cap_net_admin+ep` to `v2ray-rs-netctl` and `4750 root:v2ray-rs` to
`v2ray-rs-run`.

## Guest fixture layout

Three guest-only roles. These names identify disposable test accounts. They are
not developer accounts.

- `administrator`: the guest administrator. Owns snapshot, console, and fixture
  preparation. Enrolled in `v2ray-rs`.
- `tun-tester`: the primary test account. Starts outside `v2ray-rs` and with no
  named ACL entry on any helper. Keep it that way until the scenario under test
  performs the enrollment, otherwise the stale-session rows prove nothing.
- `tun-outsider`: never enrolled in `v2ray-rs` and never given a named ACL entry
  on a helper. Used only to prove kernel denial.

Protected layout:

```
/opt/v2ray-rs-accept/
  inplace/bin/v2ray-rs
  inplace/bin/v2ray-rs-netctl
  inplace/bin/v2ray-rs-run      # optional, only where the scenario needs it
  hold/                         # retained old inodes, mode 0700, root-owned
  evidence/                     # exported logs, captures, command output
```

Rules for the layout:

- `/opt/v2ray-rs-accept` and every ancestor directory are root-owned and not
  writable by non-root accounts. The grant refuses an unsafe canonical parent.
- Packaged acceptance uses the real installation under `/usr/bin`.
- `/opt/v2ray-rs-accept/inplace/bin` is only for adversarial copies with an
  adjacent helper and an optional wrapper.
- The relocation destination is `/usr/local/lib/v2ray-rs`. That directory is the
  product's own relocation target and must exist, be root-owned, and be empty
  before the relocation row unless the row says otherwise.
- Keep xray and the netctl helper out of AppImage mounts. A squashfs is `nosuid`,
  so a copy inside it can never hold a file capability.
- Keep old inodes under `/opt/v2ray-rs-accept/hold` with mode `0700`.
- `/tmp` is never a grant target. The grant refuses targets writable by non-root
  accounts, and `/tmp` fails that check.
- Do not share mutable fixtures between simultaneous scenarios. Each scenario
  owns its copies.

## Evidence collection commands

Every command below takes its target from the shell variable `$file`. Set `$file`
explicitly before each block, once per target: the backend binary,
`v2ray-rs-netctl`, the optional `v2ray-rs-run` wrapper, the relocated helper, or
any retained old inode. The operator sets it per target, and each block is run
once per target with the value in place.

```sh
timeout 10s stat -c '%d:%i %u:%g %a' "$file"
timeout 10s getfacl -a -c -n -E -- "$file"
timeout 10s getcap "$file"
timeout 10s sha256sum -b "$file"
timeout 10s getent group v2ray-rs
```

What each output proves:

- `stat` gives the decimal device and inode pair that identifies the file
  instance, the numeric owner and group, and the exact octal mode.
- `getfacl` gives the access ACL with numeric qualifiers (`-c` clears the comment
  header, `-n` prints numeric ids, `-E` suppresses the effective-rights comments
  that would otherwise be interleaved).
- `getcap` gives the file capability set.
- `sha256sum` gives the digest that ties an installed copy to its source.
- `getent group v2ray-rs` gives persistent enrollment.

A screenshot alone proves nothing. Screenshots cannot show numeric ids, inode
identity, or a capability set, and they cannot be diffed.

Record persistent group enrollment separately from the running GUI process's
actual supplementary groups. `getent group` reports the database. The process
reports what login gave it:

```sh
timeout 10s id -u tun-tester
timeout 10s sed -n '/^Groups:/p' /proc/$(pgrep -n -x v2ray-rs)/status
```

`pgrep -n -x` matches the exact process name `v2ray-rs`, so a backend or helper
process cannot supply the PID. If `pgrep` prints nothing, the GUI is not running
and this command is not applicable. Record the row as NOT RUN instead of running
it against an empty PID.

These next commands rewrite the access ACL of a fixture file. They are
guest-only commands. Never run them on a developer or production host.

Use numeric ACL qualifiers and explicit masks. Use `setfacl -n` for every manual
ACL fixture so an implicit mask recalculation cannot hide a dormant permission:

```sh
timeout 10s setfacl -n -m u:tun-outsider:--- "$file"
timeout 10s setfacl -n -m u:tun-outsider:r-x "$file"
timeout 10s setfacl -n -m m::r-- "$file"
```

Compare capability sets, not their printed order. Strip the leading path that
`getcap` prints, split on commas, then sort, so the comparison does not depend on
print order:

```sh
timeout 10s getcap "$file" | sed 's/^[^ ]* //' | tr ',' '\n' | sort
```

The backend capability set the product applies is
`cap_net_admin,cap_net_bind_service,cap_net_raw+ep`. The helper set is exactly
`cap_net_admin+ep`. Those are the `setcap` literals from the source constants
(`BACKEND_CAPS` and `HELPER_CAPS`). `getcap` renders the same sets in its own
form, printing a single `=ep` when every capability in the set shares the same
flags, while the source constants spell every capability with `+ep`. After
splitting on commas, each token is one capability name and only the last token
carries a flag suffix. Compare the parsed names, not the two spellings. Read the
raw `getcap` output when the flags themselves matter.

## Snapshot and evidence ordering

Follow this order for every independent scenario.

1. Prepare the guest: install packages, create accounts, build the fixture
   networks, place artifacts. Do this once and capture it in the baseline.
2. Verify the baseline: record `ip route show table all`, `ip rule list`,
   `ip -6 route show table all`, `ip -6 rule list`, `getent group v2ray-rs`, and
   the digests of all artifacts.
3. Ensure no xray or helper operation is active: no `v2ray-rs-netctl` process, no
   `xtun0` or `tun0` device, no leftover helper-owned state.
4. Close the GUI and end all tester sessions. A live session keeps a stale group
   list alive and invalidates every enrollment row.
5. Shut the guest down.
6. Create the clean baseline snapshot from the powered-off guest.
7. Restore that powered-off baseline before each independent scenario.
8. Apply only the fixture that scenario needs.
9. Start the tester session after the fixture is in place, except where the
   scenario requires an existing process (identity race, account race, stale
   relocated helper, genuine logout).
10. Run the scenario, capture evidence.
11. Export the evidence outside the guest before reset. Copy it to the
    administrative console or a read-only export target. Evidence that lives only
    inside the snapshot is lost on the next restore.
12. End scenario processes and sessions. Reap every child before namespace
    deletion.
13. Shut the guest down.
14. Restore the baseline.

## Automated routing harness

The harness is `crates/process/tests/singbox_routing.rs`. Read it before quoting
anything it does.

Accurate description:

- The file starts with `#![cfg(feature = "privileged-tests")]`. Without that
  feature the whole file compiles to nothing and the tests do not exist.
- Each test re-executes the test binary itself through `unshare -rn`, an
  unprivileged user namespace plus a new network namespace. Host root is not
  required and must not be assumed.
- The child half is selected by the environment variable
  `V2RAY_RS_NETNS_CHILD=1`.
- The sing-box binary is hardcoded: `const SING_BOX: &str = "/usr/bin/sing-box"`.
- Prerequisites checked before a run: `/dev/net/tun`, `/usr/bin/ip`,
  `/usr/bin/unshare`, `/usr/bin/nsenter`, `/usr/bin/python3`, and
  `/usr/bin/sing-box`.
- Each test prints `EVIDENCE <marker>=<value>` lines. The parent half asserts
  that every expected marker is present in the child's stdout.
- Run this with a single test worker. The tests create and tear down their own
  network namespaces and bind fixed fixture addresses, so serial execution keeps
  failures attributable. The `TEST_ARGS` override below sets one worker.
- Internal bounds in the harness: 15 s backend startup, 2 s request, 180 s child
  timeout.

Run it inside the disposable VM, from the guest source tree `/srv/src/v2ray-rs`.
The command builds the crate and runs the harness against the guest's own
`/usr/bin/sing-box`. Never run it on a developer host:

```sh
timeout 6m make test-process CARGO_FLAGS='--locked -j 2 --features privileged-tests --test singbox_routing' TEST_TIMEOUT=5m TEST_THREADS=1 TEST_ARGS='-- --test-threads=1 --nocapture'
```

`make test-process` expands to
`timeout $(TEST_TIMEOUT) cargo test -p v2ray-rs-process $(CARGO_FLAGS) $(TEST_ARGS)`,
and `TEST_ARGS` defaults to `-- --test-threads=$(TEST_THREADS)`. The explicit
`TEST_ARGS` override above adds `--nocapture`, which keeps the `EVIDENCE`
records visible on passing runs. Without it, cargo captures the test output and
the markers never reach the operator.

Five scenarios and their required evidence:

| Scenario | Required evidence |
|---|---|
| `attached_bridge_is_not_captured` | `route_172.17.0.2=docker0` (bridge selection) and `tcp_bridge=ok` (real TCP request and reply) |
| `positive_control_without_automatic_exclusions` | `route_172.17.0.2=tun0` (the same destination is captured once the automatic entries are removed) |
| `non_connected_private_destination_is_captured` | `route_10.0.0.5=tun0` (TUN selection) and `tcp_10_0_0_5=ok` (packet delivered through the tunnel to a controlled endpoint) |
| `tun_subnet_inside_attached_bridge_stays_on_tun` | `exclude_contains_172.19.0.0/16=yes`, `route_172.19.0.2=tun0`, `route_172.19.0.100=brwide0` |
| `directly_attached_dns_bypasses_tun` | `route_172.17.0.2=docker0` and `udp53_bridge=ok` |

Backend matrix: use separate VM snapshots for sing-box 1.13.0 and sing-box
1.13.14, both installed at `/usr/bin/sing-box`. Before each run record the binary
checksum and the observed version:

```sh
timeout 10s sha256sum -b /usr/bin/sing-box
timeout 10s /usr/bin/sing-box version
```

Never replace the developer host's backend and never add manual policy rules to
make a test pass. A manual exclusion rule that makes a row pass invalidates the
row and the campaign.

Proof limits, stated plainly:

- The `udp53_bridge` exchange is not a full DNS resolver transaction. It is a UDP
  exchange with a fixture on port 53.
- The private destination row uses a Direct outbound after the packet enters the
  TUN. It proves capture and delivery inside the tunnel. It does not prove a
  remote proxy.
- The harness proves neither IPv6 behavior nor a distinct published-service
  return path. It uses no IPv6 and no inbound published service.
- The diagnostic text inside `positive_control_without_automatic_exclusions`
  describes a previous host (its kernel and sing-box 1.14.3). It is source
  commentary, not evidence about the current environment.

If the control cannot reproduce the defect, preserve routes, rules, the observed
backend version, and logs, and report the reproduction as incomplete.

Observed reference behavior. On a host whose `/usr/bin/sing-box` is newer than
the pinned 1.13.x line, the four positive rows pass and print their evidence
markers, and the positive control fails with the incomplete-reproduction
diagnostic quoted above. That is the expected shape on such a host, not a
regression. Run the control on the pinned snapshots for the reproduction claim.

## Manual routing scenarios

Fixed fixtures for the manual rows:

- The prepared resolver answers `tun-accept.test` with A `192.0.2.80` and AAAA
  `2001:db8::80`. `192.0.2.0/24` and `2001:db8::/32` are documentation-only
  addresses. They are not public services.
- A fresh nonce is recorded before each TCP request. The responder must return
  that exact nonce verbatim.
- Capture only the relevant connection tuple. A busy bridge produces other
  traffic that cannot satisfy a row.
- Identify endpoints by their observed addresses and bridges. Never guess Docker
  allocations.

Create every fixture network before connecting. Prefix discovery refreshes at
connection time. Do not claim automatic update for a bridge created mid-session.

### 1. Baseline

- Actor: administrator.
- Fixture: clean baseline snapshot, GUI not running.
- Action: record `ip route show table all`, `ip rule list`, the IPv6 equivalents,
  and every existing routing workaround.
- Assertion: the record identifies any pre-existing Docker routing workaround
  before the GUI starts.
- Reset: none, read-only.

A workaround-contaminated journey cannot prove the fix. Do not remove a
workaround without separate authorization. Use the clean VM baseline instead.

### 2. IPv4 attached bridge with TUN connected

- Actor: `tun-tester`.
- Fixture: prepared Docker bridge with an attached container, sing-box selected.
- Action: start the real GUI as the tester, enable TUN, connect through the
  controlled node. Inspect the generated configuration file for the automatic
  exclusions and confirm the saved explicit exclusions are unchanged.
- Assertion: the generated file contains an automatic exclusion for each
  attached prefix, the saved explicit exclusions appear verbatim, `ip route get`
  for the actual attached container selects its bridge device, and a 2 s TCP
  request to the controlled responder returns the recorded nonce.
- Reset: disconnect, restore the baseline.

### 3. Published service from an outside client

- Actor: administrator plus the outside client.
- Fixture: guest container with an explicitly published guest port.
- Action: from the outside client request the guest's published port.
- Assertion: the reply succeeds, and a bridge-side capture shows the container's
  response on the relevant connection tuple.
- Reset: disconnect, restore the baseline.

Record the connection tuple. Unrelated traffic must not be able to satisfy the
row.

### 4. Attached resolver on port 53

- Actor: `tun-tester`.
- Fixture: the prepared resolver attached on a bridge.
- Action: query the resolver on port 53 for `tun-accept.test`.
- Assertion: a valid DNS response arrives with the expected transaction id and
  the expected answer, the resolver records receipt, and routing evidence shows
  the bridge device.
- Reset: disconnect, restore the baseline.

### 5. Non-connected private destination

Use the existing harness row `non_connected_private_destination_is_captured` for
`10.0.0.5`. Label it Direct-outbound delivery after TUN capture. Do not label it
remote-proxy acceptance.

### 6. Overlapping prefix

Use the existing harness row `tun_subnet_inside_attached_bridge_stays_on_tun` for
`172.19.0.0/16` with the TUN `/30`. Require both observed route devices. Never
request container connectivity at a TUN endpoint address; that address belongs to
the tunnel, not the container.

### 7. TUN IPv6 enabled

- Actor: `tun-tester`.
- Fixture: a guest with usable kernel IPv6 and container and bridge IPv6
  addresses observed before launch.
- Action: reconnect after fixture setup. Do not restart the GUI against an
  already-established session.
- Assertion: automatic attached IPv6 exclusions appear in the generated file,
  `ip -6 route get` selects the bridge, actual IPv6 TCP returns the nonce,
  the published service answers over IPv6, the resolver answers AAAA
  `2001:db8::80`, and the more-specific TUN prefix stays on the TUN inside an
  overlapping broader attached prefix.
- Reset: disconnect, restore the baseline.

### 8. TUN IPv6 disabled

- Actor: `tun-tester`.
- Fixture: restored baseline snapshot.
- Action: disable TUN IPv6 through the GUI and reconnect.
- Assertion: no automatic IPv6 exclusions appear, and the IPv4 TCP nonce and
  IPv4 DNS answer still succeed.
- Reset: restore the baseline.

This is not a kernel-without-IPv6 test. It is a product-level switch. Say so in
the evidence row.

### 9. No usable kernel IPv6

- Actor: `tun-tester`.
- Fixture: a separately prepared guest without usable kernel IPv6.
- Action: record the kernel IPv6 state before launch, then exercise the normal
  GUI connection without editing the generated `strict_route`.
- Assertion: IPv4 startup succeeds and bridge traffic flows.
- Reset: destroy the guest.

Mark the row BLOCKED when such a guest is unavailable. Do not substitute row 8.

The IPv4 negative control proves only the original attached-prefix capture
mechanism. Do not claim that it independently proves IPv6, DNS semantics, or
published-service causality.

## xray grant and authorization scenarios

Use the normal GUI and the existing Grant action. Do not extract scripts from the
binary and do not call private functions directly.

GUI entry points:

- Preferences, Network, `Binary path` selects the backend under test.
- Preferences, TUN, Privileges, `Grant TUN privileges` runs
  `v2ray_rs_process::grant(&backend, &helper)` on a blocking worker thread while
  the polkit dialog is open.

Adversarial copies run as:

```sh
V2RAY_RS_PROFILE=test /opt/v2ray-rs-accept/inplace/bin/v2ray-rs
```

Close any application already using that profile before launching. A second
process with the same profile can activate the first remotely.

Keep packaged, adversarial InPlace, and actual FUSE AppImage snapshots separate.
Restore before each independent row. A successful `grant` followed by an old
session and then a full logout and login is one row, not three.

| Scenario | Fixture and action | Required observation |
|---|---|---|
| Packaged precedence | Launch the packaged GUI while a stale relocated helper exists. | Preferences and startup select the packaged helper and agree on status. |
| InPlace enrollment | Safe root-owned files, tester unenrolled, capabilities missing. Invoke the real Grant. Repeat the group-absent and group-present variants. | Helper identity retained, mode `0750 root:v2ray-rs`, capability exactly `cap_net_admin+ep`. Optional wrapper identity retained, mode `4750 root:v2ray-rs`. Backend identity retained, capability set `cap_net_admin,cap_net_bind_service,cap_net_raw+ep`. |
| Genuine logout | Observe the old tester desktop after grant, then perform a complete logout and login. | The old GUI reports logout without a Grant loop. The fresh session has live membership and performs actual xray startup and helper route programming. |
| FUSE relocation | Real mounted AppImage, protected external backend, absent destination. Repeat with a safe existing destination. | Source bytes unchanged. The installed helper carries the source digest, has a verified new identity, correct metadata and capability, and correct enrollment. Selection moves to `/usr/local/lib/v2ray-rs/v2ray-rs-netctl`. |
| Replacement evidence | Retain the old relocation inode through a protected hard link. | The new target differs from the retained old inode. Do not rely on an inode number that may be reused. |
| Outsider denial | A fresh authorized tester and an independent unenrolled outsider execute identical absolute helper and wrapper paths. | The tester programs actual routes. The outsider receives a kernel execution denial. The packaged wrapper runs `id -u` as the bypass user for the tester only. |
| Safe ACL | Named outsider entry `---`, mask `r-x`, then grant. | The named entry stays unchanged and the outsider stays denied. Record intended group-mask changes separately. |
| Dormant ACL | Named `r-x` entry masked by `r--`. Repeat the mask reduction during the polkit dialog. | Refusal with `mode change would broaden named ACL access`, and no enrollment or privilege change. |
| Unsafe targets | Separate backend fixtures: user-owned, non-root-writable, symlinked, unsafe canonical parent. | Refusal before elevation, without adopting ownership. |
| Identity race | Replace the backend, helper, wrapper, or existing relocated destination with a different inode during the polkit dialog. Test absent and present destination transitions separately. | Refusal with `target identity changed`, compared against a post-intervention baseline. Pre-create destination directories before testing zero mutations. |
| Account race | Change the captured name-to-uid mapping during the polkit dialog, preserving an independent administrator session. | Refusal with `invoking account changed` before enrollment or target privilege mutation. |
| Root refusal | Root GUI, valid helper metadata, missing backend capability, then Grant. | Exact refusal `cannot grant TUN group membership to uid 0. Run the application as a non-root account`, no polkit prompt, no mutation. Mark BLOCKED if desktop policy prohibits a root GUI. |
| Noexec | File-level `noexec` mount over the helper, GUI still executable, no `nosuid` relocation. | Preferences and startup report NoExec, with no Grant and no false logout. |
| ACL tool unavailable | Hide the real `getfacl` before an InPlace grant. Repeat the hiding during the polkit dialog. | Refusal with `cannot verify access ACL` and no privilege change. This check does not apply to the relocation replacement-inode path. |
| Capability probe unavailable | Hide the real `getcap`, then separately make it non-executable. | The actual probe error appears, with no Grant loop and no false logout. |
| Invoker unresolved | Keep the tester GUI alive, remove only its account record, preserve home and the admin session, then Grant. | Refusal with `cannot resolve the invoking account` before polkit. Recover through snapshot restoration. |

### Authorized route proof

Use a VM-local network namespace. Do not create a new user namespace for this
row. Enter the namespace as the guest administrator and launch the GUI inside it
with this verified command:

```sh
timeout 120s nsenter --net=/run/netns/tun-accept -- runuser -u tun-tester -- env DISPLAY="$TESTER_DISPLAY" XDG_RUNTIME_DIR="$TESTER_RUNTIME" DBUS_SESSION_BUS_ADDRESS="$TESTER_BUS" V2RAY_RS_PROFILE=test /usr/bin/v2ray-rs
```

Constraints for this command:

- `TESTER_DISPLAY`, `TESTER_RUNTIME`, and `TESTER_BUS` come from the fresh guest
  tester desktop. Never from a developer host.
- Any existing application on that profile must be closed first, or the launch
  activates the outside process instead of starting a new one.
- Verify the actual GUI and backend UIDs, groups, and network namespace before
  treating a route change as authorized evidence:

```sh
timeout 10s cat /proc/$(pgrep -n -x v2ray-rs)/status
timeout 10s readlink /proc/$(pgrep -n -x v2ray-rs)/ns/net
timeout 10s readlink /proc/self/ns/net
```

- Verify effective mount options, `NoNewPrivs`, tracing state, and the capability
  bounding set. Do not assume a file capability works because `getcap` prints it:

```sh
timeout 10s findmnt -no TARGET,OPTIONS /usr/local/lib/v2ray-rs
timeout 10s sed -n '/^NoNewPrivs:/p' /proc/$(pgrep -n -x v2ray-rs)/status
timeout 10s sed -n '/^TracerPid:/p' /proc/$(pgrep -n -x v2ray-rs)/status
timeout 10s sed -n '/^CapBnd:/p' /proc/$(pgrep -n -x v2ray-rs)/status
```

`pgrep -n -x` matches the exact process name `v2ray-rs`. If `pgrep` prints nothing,
the GUI is not running, these commands are not applicable, and the row is NOT
RUN.

- `runuser` initializes groups when no explicit group override is given. That is
  why this row is a real authorization test.
- Its retained environment and PAM and resource-limit behavior must not be
  mistaken for a clean login. Record it as a namespace launch, not as a login.
- This launch does not replace the separate fresh-desktop logout and login
  scenario.

Required result: a controlled proxy endpoint, actual xray TUN device creation, the
device address and routes, a request and reply, and a clean teardown. Route
programming uses the real helper CLI, whose subcommands are `xray-up`, `xray-down`,
and `recover`; `--version` comes from clap.

Do not use the management network or external proxy credentials as decisive
fixtures. They make a failure ambiguous.

Do not use `nsenter --setgid` alone. It drops supplementary groups and removes
the very membership the row tests.

References:

- https://man7.org/linux/man-pages/man1/nsenter.1.html
- https://www.mankier.com/1/runuser
- https://man7.org/linux/man-pages/man2/execve.2.html
- https://man7.org/linux/man-pages/man7/capabilities.7.html

### Public-boundary limits

- Help and version output, root execution, and capability text are supporting
  checks. They are not route-programming proof.
- The AppImage omits `v2ray-rs-run` and does not create the `v2ray-rs-bypass`
  user. Do not require wrapper relocation from an AppImage.
- A symlink or nonregular helper normally blocks before Grant becomes reachable.
  That is the designed outcome, not a missing row.
- Noexec mounts and failed capability probes intentionally suppress Grant. Do
  not bypass those decisions to manufacture coverage.
- Missing groups or accounts do not prove NSS transport errors. A true NSS failure
  needs a prepared failing provider and observed error statuses. Otherwise the
  row is BLOCKED.
- A timeout case requires observing the actual stopped probe and its kill and
  reap. A missed race is NOT RUN.
- Do not fabricate deterministic nonzero-getcap, worker-cancellation, or
  mixed-UID GUI cases. If the OS cannot produce them deterministically, the row
  is BLOCKED.

## Required matrix

These dimensions are independent. Do not multiply every grant fault by every
backend version.

- For each of sing-box 1.13.0 and 1.13.14, all five harness scenarios.
- For each sing-box version, manual IPv4 bridge, published TCP, real DNS, and
  overlapping-prefix evidence through the packaged GUI.
- For each sing-box version, the applicable manual routing checks with TUN IPv6
  enabled.
- For each sing-box version, the TUN-IPv6-disabled row and the separate
  no-kernel-IPv6 row.
- xray packaged precedence, InPlace enrollment and repair, logout and login,
  outsider denial, and wrapper execution, with one recorded supported xray
  version.
- Actual FUSE AppImage relocation, replacement, logout and login, and helper
  denial, with that same xray version.
- Each adversarial privilege row once, in its applicable InPlace or relocation
  fixture.
- Backend, helper, and wrapper identity races as separate InPlace cases.
  Relocation destination races are separate.
- NSS-error and timeout rows remain BLOCKED or NOT RUN until their real OS-level
  triggers are observed.

## Evidence table and completion

One table per campaign. Statuses: PASS, FAIL, BLOCKED, NOT RUN.

| Scenario | Revision | Artifact digest | Environment | Command or action | Expected | Actual | Evidence path |
|---|---|---|---|---|---|---|---|

Rules:

- A required non-PASS row prevents product acceptance.
- Documentation completion means the procedure is written, source-checked, and
  reviewed. It does not require available VM infrastructure.
- Never label an unexecuted scenario PASS because its instructions are complete.
- BLOCKED means a prerequisite or OS-level trigger was missing, with the reason
  recorded.
- NOT RUN means the scenario was never attempted. Do not merge it into BLOCKED.

## Bounds

- Ordinary inspection commands: 10 s deadline.
- The 6 m outer harness wrapper and the harness's internal deadlines (15 s
  startup, 2 s request, 180 s child) are explicit exceptions.
- Each individual GUI action: 120 s, including one authentication interaction.
- A full logout and login is a separate 120 s transition.
- Snapshot restore and boot: ten minutes, performed outside the measured product
  action.
- Requests: 2 s. Backend startup: 15 s.
- End and reap scenario children before namespace deletion.
- Restore the snapshot after any account, mount, or package mutation.
- If restoration fails, stop that VM campaign. Do not improvise host recovery.

## Retry policy

- After a failure, allow one diagnostic reproduction with unchanged assertions.
- A diagnostic retry that passes stays flaky. It cannot clear the campaign.
- After a documented cause is corrected, a new clean-snapshot campaign may
  supersede the failed campaign.
- Preserve both campaign records and identify the changed revision or environment.