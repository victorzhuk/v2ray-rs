## Why

`fetch_with_retry` retries every fetch error blindly (3 retries, exponential backoff, ~7s wasted): a 404, 401, or malformed URL is retried exactly like a transient 503, and a malformed URL is even reported as a generic `NetworkError`. Terminal failures should fail fast with an honest error. Source: session gap-scan finding "subscription fetch retries permanent errors" (crates/subscription/src/update.rs).

## What Changes

- Classify fetch errors as transient vs terminal before retrying: HTTP 408/429/5xx and transport-level connect/timeout failures are transient; other 4xx, malformed/unsupported URLs, and request-construction failures are terminal.
- Terminal errors return immediately — no retry, no backoff sleep.
- Split the `FetchError::NetworkError(String)` catch-all so a malformed URL is a distinct terminal variant instead of masquerading as a network failure.
- Retry behavior for transient errors is unchanged (up to 3 retries, exponential backoff).

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `subscription-update`: the "Automatic subscription update" requirement's failure scenario gains a transient/terminal split — retry-with-backoff applies only to transient errors; terminal errors fail fast.

## Impact

- `crates/subscription/src/fetch.rs` — `FetchError` taxonomy (new terminal variant(s); classification helper). No external variant matches exist outside fetch.rs/update.rs (verified); downstream uses `Display` only.
- `crates/subscription/src/update.rs` — `fetch_with_retry` consults classification before looping.
- Local-file subscription sources bypass retry entirely today; that stays unchanged.
- New unit tests per classification bucket (no existing coverage of `fetch_with_retry`).
