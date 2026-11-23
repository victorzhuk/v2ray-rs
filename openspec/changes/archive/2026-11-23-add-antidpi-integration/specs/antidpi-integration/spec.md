# Spec Delta: antidpi-integration (NEW capability)

## ADDED Requirements

### Requirement: Anti-DPI engine lifecycle is session-scoped
The system SHALL start and stop the anti-DPI engine as part of a connection when anti-DPI is enabled in the effective settings, and SHALL NOT invoke any anti-DPI helper when it is disabled. When enabled, before any backend is spawned the system SHALL run a one-shot helper invocation that first parses every configured strategy chain and loads the configured hostlist, and only then installs the NFQUEUE rules in an nftables table owned by the helper (`v2ray_rs_<euid>`, one table per address family) through the helper's own netlink client; no system firewall command SHALL be executed. If that invocation fails, the system SHALL NOT spawn the backend, SHALL NOT try further candidates, and SHALL end the connection attempt with an error naming the failure. After the backend spawn and after TUN route setup completes (after the route helper's `xray-up` under xray with TUN, otherwise directly after the backend spawn), the system SHALL spawn the engine child process with `run`, the configured queue, `--no-fw` (the app owns rule installation), one `--strategy` argument per configured strategy — omitting the flag entirely when no strategies are configured so the helper applies its built-in defaults — and `--hosts` with the configured hostlist path when one is set. The system SHALL NOT report the connection start as successful until the engine child has reported readiness on stdout after binding the queue; an engine exit before readiness SHALL be a startup failure that also stops the backend, while an exit after readiness SHALL be the unexpected-engine-exit signal. A live engine child from a previous launch attempt of the same connection SHALL be reused rather than double-spawned; a delivered exit record SHALL release the stale handle so the next launch re-installs and re-spawns. When a connection is parked after a failed candidate or the candidate loop gives up, the system SHALL release that manager's engine and remove its rules so a superseded or abandoned manager can never delete the replacement's table or leave an orphan engine bound to the queue; a replacement manager that already adopted the rules SHALL disarm the superseded manager's obligation before its teardown. On stop, the system SHALL terminate the engine SIGTERM-first with a SIGKILL fallback after a bounded window, then run a one-shot `remove-rules` helper invocation as a safeguard (with `--sweep` when the engine died unexpectedly), and SHALL attempt removal even when the engine itself failed to exit cleanly. Every anti-DPI helper invocation SHALL be bounded by a timeout, terminated if its caller is cancelled, and its output and failures SHALL reach the process log stream. The engine SHALL run for every backend — v2ray, xray, and sing-box — and with TUN on or off.

#### Scenario: Rules are installed before any backend traffic
- **WHEN** the user connects with anti-DPI enabled
- **THEN** the system SHALL install the NFQUEUE rules via a one-shot helper invocation before the backend process is spawned

#### Scenario: Rule installation failure blocks the start
- **WHEN** the one-shot `install-rules` invocation fails (malformed strategy chain, unreadable hostlist, missing binary, missing capability, or netlink error)
- **THEN** the connection SHALL fail with an error naming the failure, no backend process SHALL be spawned, no rules SHALL remain from the failed attempt, and no further candidate SHALL be tried

#### Scenario: Ready state follows the queue bind
- **WHEN** the engine child is spawned but has not yet bound the queue
- **THEN** the connection SHALL remain in the starting state; an exit or timeout before the readiness line SHALL fail the start and stop the backend, and no "connected" state SHALL be reported

#### Scenario: Post-readiness engine death is distinct from startup failure
- **WHEN** the engine exits after reporting readiness
- **THEN** the system SHALL log the unexpected exit and mark the stop path for a sweep, rather than treating it as a failed start

#### Scenario: Engine starts after route setup under xray TUN
- **WHEN** the user connects with xray, TUN enabled, and anti-DPI enabled
- **THEN** the system SHALL wait for the TUN device, run `xray-up`, and only then spawn the engine child

#### Scenario: Engine starts after spawn when TUN is off
- **WHEN** the user connects with anti-DPI enabled and TUN disabled
- **THEN** the system SHALL spawn the engine child directly after the backend spawn, with no route-helper invocation in between

#### Scenario: Disconnect stops the engine and removes rules
- **WHEN** the user disconnects from a session with a live engine
- **THEN** the system SHALL send SIGTERM to the engine first, escalate to SIGKILL only after the timeout, and run the one-shot `remove-rules` safeguard with the same controller identity, leaving both `v2ray_rs_<euid>` nftables tables absent

#### Scenario: Failed removal keeps the obligation
- **WHEN** the one-shot `remove-rules` fails or removes only one family
- **THEN** the helper SHALL keep the journal in its cleaning phase, the system SHALL report the failure, and the manager SHALL retain its rule-cleanup obligation so a later stop retries it

#### Scenario: Engine dies unexpectedly mid-session
- **WHEN** the engine child exits without a stop request
- **THEN** the system SHALL log the exit to the process log stream and record it; the stop path removes the recorded rules with the controller's own identity (no sweep while the controller lives), and a later launch of the same manager SHALL treat the dead engine as gone and re-install and re-spawn it. (Traffic stays unaffected in the interim because every NFQUEUE rule carries `--queue-bypass`.)

#### Scenario: Crash respawn reuses the live engine
- **WHEN** the backend crashes and is respawned while the engine child from the previous attempt is still running
- **THEN** the system SHALL NOT run a second `install-rules` for the same queue and SHALL NOT spawn a second engine child

#### Scenario: Disabled anti-DPI invokes nothing
- **WHEN** the user connects with anti-DPI disabled in the effective settings
- **THEN** no anti-DPI helper invocation SHALL be made and no engine process SHALL exist

#### Scenario: v2ray backend still gets the engine
- **WHEN** the user connects with the v2ray backend (which has no TUN support) and anti-DPI enabled
- **THEN** the engine SHALL start and cover the proxied flows, since NFQUEUE operates on host egress regardless of backend

### Requirement: Rule ownership is journaled and session-scoped
The system SHALL record every installed nftables table in a write-ahead ownership journal keyed by session id, boot id, network-namespace identity, PID-namespace identity, controller process identity (pid and start time), queue, fwmark, and the families installed. Installation SHALL write the journal before the first mutation, SHALL require both owned family table names to be absent before recording fresh intent, SHALL create tables exclusively (never adopting or replacing a same-named table, whatever marker it carries), and SHALL update the journal after each family installs. A verified same-owner reinstall SHALL perform no kernel mutation and SHALL repair only a family flag the journal failed to record, persisted atomically; a confirmed creation conflict SHALL drop only the attempted family's unacquired obligation, and ambiguous replies SHALL keep every obligation. Removal SHALL verify the journal's PID namespace, epoch, and controller identity before touching any table, SHALL refuse a journal without matching PID-namespace evidence, SHALL keep the journal on any removal failure or exhausted exchange budget, and SHALL clear it only after every recorded family is confirmed gone. Transactions SHALL serialize on one per-effective-UID, per-network-namespace lock independent of any journal path override, failing closed when that runtime directory is unusable, and readiness verification SHALL hold the lock through queue bind and teardown. A same-epoch journal whose controller is provably dead SHALL be recovered before a new install; a journal from another boot SHALL be handled only by `remove-rules --sweep`, which retires the stale local record once both owned table names are confirmed absent in the current namespace, refuses while either name exists, refuses a same-boot foreign network namespace while naming it, and preserves records without PID-namespace evidence for operator inspection; cross-epoch table deletion SHALL stay forbidden. `remove-rules --sweep` SHALL refuse while the recorded controller is alive (a live controller never sweeps after engine death) and SHALL refuse a `v2ray_rs_<euid>` table that has no journal instead of guessing ownership. No automatic path SHALL delete a table it cannot attribute to a dead same-boot, same-namespace session. Because the table name carries the effective UID, one account's table is never visible to, or removable by, another account's helper.

#### Scenario: Another account's rules are untouched
- **WHEN** two local accounts each run a session with anti-DPI enabled
- **THEN** each helper installs and removes only its own `v2ray_rs_<euid>` table, and neither sweep can observe or delete the other's rules

#### Scenario: Foreign owner is refused
- **WHEN** `remove-rules` or a second install runs in the same boot and namespace while the recorded controller is alive and different
- **THEN** the helper SHALL refuse without mutating any table and report the ownership conflict

#### Scenario: Dead owner is recovered
- **WHEN** a journal records a controller whose pid no longer exists or whose start time no longer matches, in the same boot and namespace
- **THEN** the next install SHALL remove only the recorded families, clear the journal, and then install fresh rules

#### Scenario: Unowned table is not deleted
- **WHEN** a `v2ray_rs_<euid>` table exists but no journal is present
- **THEN** `remove-rules --sweep` SHALL report the unowned table and return an error without deleting it

#### Scenario: Partial removal is retried
- **WHEN** one family is removed and the other fails
- **THEN** the journal SHALL retain both the phase and the recorded families, and a later stop with the same controller identity SHALL retry the remaining family

#### Scenario: Prior-boot record is retired only when its tables are absent
- **WHEN** `remove-rules --sweep` runs on a journal whose boot id differs from the current boot
- **THEN** the helper SHALL delete only the stale local journal once both owned table names are confirmed absent in the current namespace, SHALL refuse while either name still exists, and SHALL never delete a table recorded under another boot

#### Scenario: Same-boot foreign namespace is refused with guidance
- **WHEN** a journal matches the current boot but records a different network namespace
- **THEN** install, removal, and sweep SHALL refuse without mutating any table and SHALL name the recorded namespace so the operator can act there

#### Scenario: Records without PID-namespace evidence stay preserved
- **WHEN** a same-boot journal lacks PID-namespace evidence or records a different PID namespace
- **THEN** no automatic ownership or liveness decision SHALL run on it, the record SHALL stay on disk for operator inspection, and `remove-rules --sweep` SHALL refuse to retire it

### Requirement: Anti-DPI settings are restart-relevant
The system SHALL treat the anti-DPI settings as restart-relevant: the runtime snapshot SHALL capture the anti-DPI configuration actually used for the current connection attempt, SHALL report divergence when the edited settings differ from the snapshot, and SHALL restore the snapshot's anti-DPI configuration on apply-and-restart. While connected, a session launched without the engine SHALL keep running without it — indicators follow the running session — until the session is restarted with the new settings.

#### Scenario: Toggling the engine while connected raises the restart banner
- **WHEN** the user enables or disables anti-DPI, changes the queue, or edits the strategy list while connected
- **THEN** the restart-required banner SHALL appear, and "Apply & Restart" SHALL reconnect with the new engine arguments captured in the new snapshot

#### Scenario: Indicators follow the running session
- **WHEN** anti-DPI is enabled in settings while a session launched without the engine is running
- **THEN** the app SHALL keep treating the session as engine-less until it is restarted with the new settings

### Requirement: Anti-DPI helper resolution and access classification
The system SHALL resolve the anti-DPI helper binary with the same ordering as the route helper — a capability-capable bundled copy beside the running executable, else the relocated copy under `/usr/local/lib/v2ray-rs`, else the bundled copy, else the first executable of that name on `$PATH` — and SHALL classify its access with the same shared classifier used for the route helper (ready, missing, enrollment required, re-login required, wrong ownership or mode, noexec mount, failed probe). A `$PATH` result SHALL be converted to an absolute path before classification, execution, or grant planning; a bare name SHALL never be classified or spawned. The connection preflight and the preferences page SHALL use the same classifier and the same resolved absolute path. The system SHALL verify, before spawning anything, that the resolved helper holds both `CAP_NET_ADMIN` and `CAP_NET_RAW` as effective and permitted file capabilities; a permitted-only file SHALL NOT pass. When the application runs with effective user ID 0, capability checks SHALL be skipped. Any non-ready classification or missing capability SHALL end the connection attempt with an error naming the classification or the missing capability and pointing at the shared grant action, without trying further candidates.

#### Scenario: Missing capabilities block the start
- **WHEN** anti-DPI is enabled but the helper binary lacks `cap_net_admin` or `cap_net_raw`
- **THEN** the connection SHALL fail without spawning the backend, with an error naming the missing capability and surfacing the "Grant TUN privileges" action

#### Scenario: Helper not executable yet
- **WHEN** the resolved helper requires group enrollment or a re-login
- **THEN** the connection SHALL fail without spawning, with the classifier's message (enrollment or log-out instruction) and SHALL NOT report a different cause

#### Scenario: Relocated copy is used on a nosuid mount
- **WHEN** the bundled helper sits on a mount that ignores file capabilities and a relocated copy exists
- **THEN** the system SHALL execute the relocated copy, matching the route-helper resolution order

#### Scenario: Root skips capability probes
- **WHEN** the application runs with effective user ID 0 and anti-DPI is enabled
- **THEN** the system SHALL NOT read file capabilities and SHALL proceed to start

#### Scenario: Startup classification uses the shared classifier and path
- **WHEN** the connection preflight classifies the anti-DPI helper
- **THEN** it SHALL use the same shared classifier and the same absolute resolved path the grant provisions, and a non-ready classification SHALL fail the start with that classification's error; the anti-DPI page shows the hint, and the TUN page's shared capability row reports the anti-DPI helper so the grant action is visible

### Requirement: Anti-DPI privileges are granted once via pkexec
The system SHALL extend the one-time `pkexec` privilege grant to cover the anti-DPI helper: a single elevation SHALL apply `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` to the backend binary, `cap_net_admin+ep` to the route helper, and `cap_net_admin,cap_net_raw+ep` to the anti-DPI helper, enroll the invoking account in the `v2ray-rs` group, and set `root:v2ray-rs` mode `0750` on the anti-DPI helper with ownership changes preceding mode changes preceding capability changes. The antidpi target SHALL be optional: when the binary is absent, the grant SHALL proceed for the remaining targets. The elevated script SHALL re-validate the antidpi target (non-symlink regular file, root-owned, not world/group-writable, recorded `device:inode` identity, protected parents, ACL check) exactly like the route helper. A relocating grant SHALL verify both destinations' prior identities before any mutation, stage each payload into a fresh protected temp file in the destination directory, verify each staged length against the declared size, publish each with an atomic rename, and emit the helper identity receipt first and the anti-DPI identity receipt second only after both publishes succeed; any failure SHALL unlink the staged files and leave the existing privileged binaries unchanged. Every target's filesystem SHALL be preflighted for file-capability support before the polkit prompt.

#### Scenario: Grant covers all privileged binaries in one prompt
- **WHEN** the user invokes the grant action with the backend, route helper, and anti-DPI helper all needing capabilities
- **THEN** the system SHALL run exactly one `pkexec` elevation applying each target's capability set and the group enrollment, and SHALL verify each installed result by identity rather than trusting the exit status

#### Scenario: Grant proceeds without the antidpi binary
- **WHEN** the anti-DPI binary is not present on the system and a TUN grant is invoked
- **THEN** the grant SHALL complete for the backend and route helper without touching the antidpi target

#### Scenario: Antidpi target on a nosuid mount refuses before elevation
- **WHEN** the anti-DPI helper resides on a filesystem that does not honor file capabilities
- **THEN** the grant SHALL fail before the polkit prompt, naming the path and the manual `setcap` command

#### Scenario: Capabilities lost after upgrade are re-detected
- **WHEN** the anti-DPI binary is replaced by a package upgrade and loses its capabilities
- **THEN** the next anti-DPI start attempt SHALL detect the missing capability and re-offer the grant
