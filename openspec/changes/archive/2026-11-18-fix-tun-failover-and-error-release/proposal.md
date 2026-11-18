# Proposal

## Why

Failover from a failed TUN candidate tears down or checks the shared tunnel after the replacement candidate is already live, destroying active routes and strict kill-switch protection while reporting a connected session. Aborted reconnects from a terminal TUN error also leave strict routes blackholing host traffic, and latency or availability probes run unmarked during non-`Running` TUN states.

## What Changes

- Prevent parked failed TUN candidates from executing tunnel teardown or sing-box stop checks against a shared interface and session journal once a replacement TUN candidate adopts that session.
- Reconcile optional xray policy rules (UID bypass at priority 8998 and port-53 DNS capture at priority 8999) on every `xray-up` run so failover from a pinned candidate to an unpinned candidate removes stale DNS capture rules.
- Release leftover fail-closed TUN routing state whenever a pending or deferred reconnect from `Error` aborts in preflight or transitions into a non-TUN connection.
- Suppress manual TCP latency tests, scheduled background latency ticks, Real Delay probes, and service availability checks whenever TUN owns or is releasing host routing (`Starting`, `Running`, `Stopping`, fail-closed `Error`, and route teardown in flight).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tun-mode`: Preserve live replacement TUN sessions across candidate failover, reconcile optional xray policy rules on `xray-up`, and release fail-closed TUN routes when follow-up reconnects abort or switch off TUN.
- `background-latency-testing`: Gate manual and scheduled node probes across all TUN-owned lifecycle phases, not only `Running`.

## Impact

`crates/process` (`session`, `manager`, `tun`), `crates/netctl` (`net`, `ownership`), `crates/ui` (`app`, `config_regen`, `subscriptions`), and `docs/ARCHITECTURE.md`. No new external dependencies or persisted schema versions.
