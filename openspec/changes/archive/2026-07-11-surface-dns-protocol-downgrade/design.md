## Context

`DnsProtocol::fallback_protocol_for_backend` (core) implements the matrix; `dns_server_address_for_backend` (generator) applies it with a `log::warn!`. The server dialog's protocol ComboRow offers all six protocols regardless of backend; the only backend-conditional UI today is the Detour row and FakeIP group (sing-box only). The dialog already has an inline validation-warning pattern to reuse.

## Goals / Non-Goals

- Goal: no silent downgrade — user sees it at selection time and on saved rows.
- Non-goal: changing generator behavior (accept-and-downgrade stays; it is spec'd and tested).
- Non-goal: per-backend option hiding (rejected in brainstorm — silently reinterprets saved servers on backend switch).
- Non-goal: `listen_address` writer fallback (investigated: unreachable from UI, already spec'd as defense-in-depth; no change).

## Decisions

- Warning driven by `fallback_protocol_for_backend` at ComboRow change time and dialog open time; text names the effective protocol. Alternative — validation error blocking save — rejected: the config is valid, only backend-relative.
- Saved-row indicator rendered in `render_dns_servers` from the same function; re-rendered when backend type changes (preferences already receive backend changes via SyncSettings-style flow).
- No new core API unless the UI needs a convenience predicate; if added, it delegates to the existing function.

## Risks / Trade-offs

- [Two UI surfaces + generator all consulting the matrix] → single-sourcing requirement in dns-configuration makes drift a spec violation.
- [Warning fatigue for v2ray users with many DoT servers] → passive caption-style indicator on rows, prominent warning only inside the edit dialog.

## Migration Plan

UI-only PR; no data changes. Rollback = revert.

## Open Questions

None.
