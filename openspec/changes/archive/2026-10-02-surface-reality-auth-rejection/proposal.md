## Why

When a provider rotates REALITY keys faster than the subscription refreshes, the backend log fills with `REALITY: received real certificate (potential MITM or redirection)` — the server rejecting the client's handshake and serving the camouflage certificate instead. Measured live: this signature appeared 25 times in one day on a machine whose reality nodes then failed to connect at all, on every server of the pool, while a later subscription refresh did not fix it. The existing toast (added for deprecation warnings) quotes the raw log line once, but the connection still fails with a generic error, the toast does not say what it means for the user, and nothing points at the one action that can help: refreshing the subscription that owns the node.

## What Changes

- When a connect candidate fails and its captured backend output contains the REALITY auth-rejection signature, the connection failure is reported with a classified cause — "server rejected REALITY authentication; the subscription's REALITY keys may be stale" — instead of the generic failure text, at most once per candidate.
- The failure report carries an offer to refresh the subscription owning the failed node; the refresh itself remains user-initiated and is offered at most once per connection per subscription.
- Failures without a matching signature keep the existing generic reporting; nothing about the candidate order, retry, or failover flow changes.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `diagnostic-logs`: connect-candidate failures are classified from known backend output signatures.
- `subscription-update`: a subscription refresh is offered when REALITY auth rejection is classified on one of its nodes.

## Impact

- `crates/ui/src/backend_warning.rs` — the pattern table gains a failure classification (cause text + advice) alongside the existing toast patterns.
- `crates/ui/src/connection.rs` — on candidate failure, classify against the candidate's captured backend output and render the cause; attach the refresh offer.
- Toast plumbing — an action button on the classified toast opening the subscription's update flow; no core, process, or generator change.
