# Design: state hygiene

## Context

All three defects are evidence-backed from a live install: `instance.json` carrying `build_version: 0.7.4` under a 0.13.1 binary; `data_dir/generated/` still holding configs months after the runtime-dir migration shipped (relocation ENOENTs every start because the destination subdir is only created later by `ConfigWriter`); and the tun-mode spec describing the pre-policy-routing split-route helper.

## Goals / Non-Goals

- Goal: stamp and on-disk layout match what the running build actually does.
- Non-goal: any change to netctl behavior — the spec moves to the code, not vice versa.

## Decisions

- Relocation creates destinations with the existing 0o700 directory helper; cleanup deletes legacy files only after a successful move or when the destination already has current data. Alternative — leaving legacy files as backup — rejected: they contain node credentials and stale ports, and the migration has already shipped.
- `update_started` rewrites the whole stamp (it already saves atomically); no separate migration path needed.

## Risks / Trade-offs

- [Deleting legacy configs a user manually pointed a backend at] → `config_output_dir` override exists for external consumers; the legacy default dir was never a supported external path.

## Migration Plan

Single PR. First start after upgrade performs the cleanup. Rollback = revert.

## Open Questions

None.
