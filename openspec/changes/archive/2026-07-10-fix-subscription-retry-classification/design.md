## Context

`fetch_with_retry` (crates/subscription/src/update.rs) loops on any `Err` from `fetch_with_client` up to `DEFAULT_MAX_RETRIES = 3` with 1s/2s/4s backoff. `FetchError` already carries `HttpError { status, .. }` but the status is never consulted; the scheme check in `fetch.rs` returns `NetworkError` for malformed URLs. Only `SubscriptionSource::Url` goes through retry; file sources bypass it.

## Goals / Non-Goals

- Goal: terminal errors fail fast; transient errors keep the existing retry envelope.
- Non-goal: changing UI copy for fetch failures or surfacing auto-update failures as toasts (candidate follow-up, out of scope).
- Non-goal: retry policy for local-file sources.

## Decisions

- Classification lives on `FetchError` (e.g. `fn is_transient(&self) -> bool`) so the policy sits next to the taxonomy, not in the retry loop. Alternative — classifying inside `fetch_with_retry` — rejected: the loop shouldn't know reqwest internals.
- HTTP split: 408/429/5xx transient; all other 4xx terminal. Standard retry taxonomy; agreed in brainstorm.
- Transport split via `reqwest::Error` predicates: timeout/connect → transient; builder/request-construction → terminal. Verify predicate availability against the pinned reqwest version before implementing (workspace pins reqwest in root Cargo.toml).
- New terminal variant `InvalidUrl` for the scheme/parse check instead of `NetworkError`. Display strings adjust; no code matches variants outside the crate (verified by grep).

## Risks / Trade-offs

- [No existing tests around retry] → add unit tests per bucket; classification is pure and testable without a network. The retry loop test can use an injected fetch closure or a counter fake if the current signature allows; otherwise test `is_transient` exhaustively and keep the loop change minimal.
- [Faster failure changes auto-update batch timing] → net improvement; `refresh_all_overdue` concurrency (4) unaffected.

## Migration Plan

Single PR; no data or config migration. Rollback = revert.

## Open Questions

None.
