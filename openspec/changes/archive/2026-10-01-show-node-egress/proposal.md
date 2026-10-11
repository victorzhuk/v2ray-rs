## Why

Subscription providers reuse identical display names across differently-transported subscriptions while pointing them at different physical servers. Measured on a live vpndio setup: the node labelled "Germany - Nuremberg" exits via Hetzner DE in the `ws` subscription and via AT Vienna in the `raw_tls` subscription, and the geo gates of Google services treat those two exits differently (NotebookLM blocks the Hetzner exit with `location=unsupported` while passing the AT one). Nothing in the UI shows which exit a node actually provides, so the user attributes the difference to the protocol instead of the exit and cannot pick nodes by measured egress.

## What Changes

- A Real Delay probe session additionally determines each tested node's egress address and location: the probe config gains one loopback HTTP inbound per tested node routed exclusively to that node's outbound, and the probe runner issues one HTTP request per node through its inbound to an IP-echo endpoint, parsing the exit address and country code from the response.
- The egress sample is persisted per node in the latency snapshot file alongside the Real Delay sample, and hydrated at startup like the other latency data.
- Subscription node rows display the last known egress (location code and address) when a sample exists; nodes with identical display names but different exits become distinguishable.
- No change to how delays are measured, no additional backend instance, and no change to the user-facing connection flow.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `real-delay-latency-test`: egress detection during probe sessions and per-node egress persistence.
- `ui-lists`: node rows show the last known egress location and address.

## Impact

- `crates/core/src/config/probe.rs` — probe config generators emit per-node loopback inbounds and inbound-to-outbound routing rules.
- `crates/subscription/src/real_delay.rs` — probe runner issues the egress request per node and parses the reply.
- `crates/core/src/resolve.rs`, `crates/core/src/models/` — egress sample on the latency snapshot and node models.
- `crates/ui/src/subscriptions.rs` — node row subtitle/tooltip for the egress sample.
