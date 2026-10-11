## Why

Google services gate availability per service, not per country, and the gate weighs exit-IP reputation: measured live, NotebookLM answers a Hetzner DE datacenter exit with a redirect to `notebook.google/?location=unsupported` (a blocked verdict for a supported country) while an AT Vienna exit reaches the sign-in flow, and Gemini passes both. Real Delay cannot catch this — a node can return a fast `gstatic` 204 and still be rejected by a service the user cares about. Today the only way to know is to connect a node and try each service by hand, which is how the user ended up attributing exit-IP differences to the proxy protocol.

## What Changes

- New user-triggered "Check availability" probe: for selected nodes, the app requests each built-in service's check URL through that node (reusing the ephemeral probe session and the per-node loopback inbounds introduced by show-node-egress) and derives a per-service verdict of `available`, `blocked`, or `unknown` from the reply's final URL and body markers.
- Built-in service checks are a data-defined table, initially NotebookLM (`https://notebooklm.google.com/`, blocked on a `location=unsupported` final URL or an availability-refusal body marker) and Gemini (`https://gemini.google.com/about`, blocked on an availability-refusal body marker), so adding a service needs no protocol work.
- Verdicts persist per node and per service in the latency snapshot file with a checked-at timestamp, hydrated at startup like the other probe data.
- A results view lists the selected nodes' per-service verdicts with badges and checked-at times; a `blocked` verdict shows the node's last known egress as context, making "this exit is rejected by this service" visible instead of inferable.

## Capabilities

### New Capabilities

- `geo-availability-probe`: per-node per-service availability verdicts derived through the node's own tunnel.

### Modified Capabilities

_None._

## Impact

- `crates/core/src/geo_probe.rs` — new pure module: service check table and verdict derivation from final URL plus body.
- `crates/core/src/config/probe.rs` — reuses the per-node inbounds from show-node-egress, no new generator surface.
- `crates/subscription/src/real_delay.rs` — probe runner gains the availability pass (request, redirect following, verdict derivation).
- `crates/core/src/resolve.rs`, `crates/core/src/models/` — verdict persistence on the latency snapshot.
- `crates/ui/src/subscriptions.rs` — availability results view and action entry point.
