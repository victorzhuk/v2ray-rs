## Why

Both observatory tests in crates/subscription/tests/e2e.rs are ignored. They invoke the version command and print failures. Neither asserts that the production client can read a delay from a live backend.

## What Changes

Require a bounded interoperability test for each supported observatory implementation before publication.

## Capabilities

### Modified Capabilities

- `observatory-real-delay` gains release acceptance for live status mapping and cleanup.

## Impact

Subscription acceptance tests and the release quality workflow. Production measurement behavior stays the same.
