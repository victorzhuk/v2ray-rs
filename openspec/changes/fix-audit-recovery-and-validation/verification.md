# Verification

## Executed checks

- Workspace suite with required xray/sing-box and a virtual display: 1,266 passed, zero failed, two explicitly ignored. The ignored observatory tests are not acceptance evidence.
- The final reserved-interface guard was added afterward. Both its preparation regression and existing interface-validation scenario passed on the final source.
- Final formatting, all-target/all-feature Clippy, workflow actionlint, strict change validation, and whitespace checks passed.
- Pinned xray 26.11.9 and sing-box 1.13.14 archives were checksum-verified. Config checks used the pinned binaries.
- All five pinned release asset API URLs returned matching SHA-256 digests. The commit-pinned GTK plugin was verified separately. Inspection found appimagetool's implicit runtime download; the runtime is now separately pinned and passed through LDAI_RUNTIME_FILE.
- Owner-only backups of current workflow copies were verified outside version control before workflow edits. Restore only after verifying there were no concurrent changes.

## Ownership scenarios

| Scenario | Result | Observations |
| --- | --- | --- |
| Real-kernel journal lifecycle | PASS | Fresh user/network namespace, verified distinct from parent. IPv4/IPv6 default routes, repeated additions, exact acknowledged objects, competing session refusal, foreign route refusal, disappeared route handling, repeated cleanup, and empty completed ledger. |
| Real helper CLI reservation | PASS | Fresh user/network namespace. Initial priority conflict preserves journal bytes; repeat reserve succeeds; another session is refused; stale UUID, wrong namespace, and missing marker refuse without retargeting the journal. |
| Read-only sing-box stop verification | PASS | Fresh user/network namespace. Remaining selecting rules and routes cause refusal and remain byte-identical. Absence succeeds after the fixture removes them. |
| Reserved interface name | PASS | Settings validation rejects the reservation name; preparation creates no recovery record. |
| Legacy/malformed evidence and controller changes | PASS | Focused persistence and desktop tests retain records, block unsafe reuse, and avoid destructive helper calls. |
| Full TUN and replaced-device scenario | BLOCKED | /dev/net/tun is absent. Journal-only namespace scenarios do not discharge this row. |
| Packaged desktop, grant, and downgrade matrix | NOT RUN | No prepared artifact or authorized prepared VM is available; Docker access is denied. |

Each native scenario has an outer process deadline and one worker. Workspace
build concurrency and test workers are both capped at two, with a five-minute
deadline. There is no wrapper for the gated netctl target, so its focused checks
use bounded raw Cargo commands.

The first kernel reservation attempt failed because the kernel ignored the alias
on link creation. An explicit update was verified independently and the helper
now records the index, sets the alias, and verifies it. Fresh-namespace scenarios
then passed. A later disappearance regression failed on stale completed-ledger
entries; cleanup now clears verified-absent entries and resume prunes absent
acknowledged objects. The corrected scenario passed in a new namespace. These
were corrected deterministic defects, not passing retries of unchanged failures.

## Completion limits

The ownership implementation and schema migration were approved and implemented.
CI contains required native ownership and full TUN gates, but the workflow has
not been executed on GitHub during this change. The complete runbook matrix in
docs/TUN_ACCEPTANCE.md remains required. Do not archive the change or treat the
product as accepted until every required row has evidence.

## Review corrections

Journaled route and rule additions now reject EEXIST without recording the
observed object as owned. The private namespace regression inserts a foreign
route after intent and verifies both insertion failure and cleanup refusal.
It passed outside the restricted sandbox (`/tmp/review-collision-native.log`).
The sandbox attempt failed when opening its netlink socket.

Explicit start renews a completed journal, updates the runtime UUID, and clears
the prior device index. Unfinished journals retain their identity. Crash respawn
continues through launch without renewal. Core renewal tests cover both backends.
The process regression verifies the explicit start boundary before config rejection.
Full device and packaged acceptance remain outstanding.

Final process library run: 178 passed, no failures or skips
(`/tmp/review-process-final.log`). The core renewal regression passed.
Formatting, strict change validation, and diff whitespace checks passed.

## Second review pass

The review found that the containerized release steps ran under `sh`, the
subscription refresh left persisted and in-flight latency samples attached to
changed connections, and the recovery lifecycle could discard or refuse a
journal it should have kept or retired. Corrections on the final tree:

- Release steps run under Bash, and the local AppImage recipe now requires an
  observed window and a clean SIGTERM exit instead of accepting a timeout.
- Automatic helper commands take an owner-only lock file beside the journal.
  The abstract-name lock could be squatted by any local user and block connect,
  teardown, and recovery for the whole namespace.
- Absent rule and route deletes count as success; the observed TUN index reaches
  route setup, and a vanished device refuses setup instead of proceeding
  unpinned. A session identity without a journal is refused.
- A changed connection invalidates its persisted and in-flight latency samples
  before the planner reads them.
- A finished journal is retired before the boot and namespace gate, while an
  unfinished journal survives a clean stop of an unrelated session. The legacy
  marker error names the file and the verified-cleanup command.
- The `app-persistence` delta now states the config-preservation requirement.

Evidence on the final tree: `cargo fmt -- --check`, `cargo check --workspace
--all-targets --all-features`, and `cargo clippy --workspace --all-targets
--all-features -- -D warnings` passed. The bounded workspace suite with required
display and backends ran 1,284 tests: 1,280 passed, four failed, two ignored.
The four are capability-gate tests in `v2ray-rs-process`: this sandbox mounts `/`
with `nosuid`, so the unchanged probe returns `TunMountUnsupported` before their
assertion. The CI runner uses a writable root. Netctl ran 37 unit tests; its
privileged scenarios compile with `--features privileged-tests` but were not
executed here. OpenSpec strict validation, actionlint, and `git diff --check`
passed.
