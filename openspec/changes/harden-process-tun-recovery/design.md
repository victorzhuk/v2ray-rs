## Context

`stop()` early-returns when `child` is `None` regardless of state; every preflight failure and crash-budget exhaustion leaves state `Error` with no child. The state machine already permits `Error → Stopped`; nothing reaches it except a fresh `start()`. Masked today because the UI computes terminal states itself — a footgun for any direct caller. `grant()` calls `file_caps_supported` only for the backend; the helper path and wrapper are resolved but never preflighted, and `setcap`/`chmod u+s` exit 0 on `nosuid` mounts.

## Goals / Non-Goals

- Goal: `stop()`/`shutdown()` are total — callable from any state with a sane result.
- Goal: grant failure on `nosuid` is loud and names the path, before the pkexec prompt.
- Non-goal: any change to netctl's runtime dependencies (it stays dependency-light; the fwmark test is dev-only).
- Non-goal: backend minimum-version gating (deferred; no target versions exist).

## Decisions

- `stop()` handles the no-child case by transitioning `Error → Stopped` (permitted transition) and keeping the existing silent `Ok(())` for `Stopped`; `shutdown()` unconditionally sets `auto_restart = false` and delegates. Alternative — teaching every caller to check state first — rejected: fixes one caller, not the API.
- Fwmark: both constants become `pub`; netctl gets `[dev-dependencies] v2ray-rs-core` and one test asserting equality. Alternative — a new shared leaf crate — rejected as over-structure for one `u32`; revisit if a second shared value appears.
- Grant preflight order: check backend, helper, wrapper mounts first; report the first unsupported path via the existing `PrivilegeError::Unsupported { path, caps }` shape so UI copy keeps working.

## Risks / Trade-offs

- [Behavior change: grant now fails where it "succeeded" before] → it previously succeeded falsely; failing fast with the manual command is strictly more honest. UI already renders this error class.
- [CI job time grows by one package install] → negligible; unblocks a schema-regression net that currently never runs.

## Migration Plan

Single PR; no data migration. Rollback = revert.

## Open Questions

None.
