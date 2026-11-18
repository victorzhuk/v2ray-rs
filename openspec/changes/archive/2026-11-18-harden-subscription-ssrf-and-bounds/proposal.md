# Proposal

## Why

Subscription HTTP SSRF checks run only on 3xx redirect hops, allowing initial subscription URLs (including base64-encoded `v2raytun://import/` links) and DNS-rebound hostnames to connect to loopback, LAN, CGNAT, and cloud metadata endpoints. In addition, 10 MiB subscription payloads enforce no cap on parsed node count, allowing compact URI lists to materialize over 100,000 GTK row widgets synchronously on the main thread.

## What Changes

- Validate the initial subscription URL against the public-target host/IP policy before issuing any HTTP request, treating violations as non-transient errors without retry.
- Enforce the same private/local IP rejection on DNS-resolved socket addresses in the production subscription HTTP client so public hostnames resolving to loopback, link-local, RFC 1918, CGNAT, benchmarking, or unique-local addresses cannot connect.
- Cap parsed proxy nodes per subscription at 4,096 across URI-list and JSON bundle imports, failing oversized imports cleanly while preserving existing subscription data on refresh.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `subscription-update`: Reject private/local initial subscription URLs and DNS answers, and bound parsed node count per subscription.

## Impact

`crates/subscription` (`fetch`, `parser`, `json_import`, `service`) and `crates/ui` (`subscriptions/dialogs`, `wizard`). Loopback HTTP test fixtures in `crates/subscription` opt into an explicit test-only client builder so production callers always enforce public-target resolution.
