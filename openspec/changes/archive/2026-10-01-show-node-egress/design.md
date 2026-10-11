## Context

- Real Delay sessions already run one ephemeral backend with one generated config holding all tested nodes as outbounds (`crates/core/src/config/probe.rs`, `ProbeConfigGenerator` per backend); delays are read through the backend's own API (Clash API for sing-box, Observatory for xray/v2ray), which cannot return response bodies, so egress cannot be derived from the existing API surface.
- The app already ships an HTTP client (reqwest with rustls) used by subscription fetching.
- The user's own diagnosis session measured exits per node with `https://www.cloudflare.com/cdn-cgi/trace`, which answers plain text (`ip=…`, `loc=…`) over HTTPS from anycast edges, needs no API key, and is not geo-fenced.
- Under TUN, the ephemeral probe backend's own outbound traffic rides the active user session exactly like Real Delay traffic does today (see the suppress-tun-latency-pings change for precedent); the per-node inbounds are loopback-only and are not captured by TUN.

## Decisions

- **Per-node loopback inbound instead of a backend API extension.** Each tested node gets one HTTP inbound bound to `127.0.0.1` on a port obtained by binding `127.0.0.1:0` up front (same allocation pattern as the probe API port), plus one routing rule mapping that inbound's tag to the node's outbound tag. This works identically for xray, v2ray, and sing-box and adds no backend-version constraint beyond what probes already require. The inbound carries no user traffic and lives only for the session.
- **One egress request per node, after its delay probe completes**, so a slow egress fetch never delays the delay results. Timeout equals the configured probe timeout. Response parsing extracts `ip` and `loc`; anything else (non-200, timeout, body over 64 KiB, missing keys) records the sample as unknown. The egress result never overwrites or masks a successful delay result.
- **Egress URL is a constant, not a setting.** One more preferences row for an IP-echo endpoint is not justified while the default is anycast and key-less; making it a constant keeps the settings surface unchanged. Revisit only if the endpoint becomes unreliable.
- **Persistence reuses the latency snapshot file** with a new optional per-node egress sample (`address`, `location`, `checked_at`), written through the existing atomic persist path. Missing sample means unknown; there is no separate state file.
- **UI placement is the node row subtitle**, the same slot pattern the manual-node editor uses for transport notes: `exit: AT · 2001:db8::1` when known, nothing when unknown, so identical display names across subscriptions become distinguishable at a glance.
- **Out of scope:** deriving location beyond the endpoint's `loc` code, geolocating the user's own direct IP, and showing egress for manual nodes is included only because they flow through the same probe session, not through extra UI.

## Risks

- CDNs blocking the echo endpoint from datacenter IPs would show unknown egress; the failure mode is a missing subtitle, never a wrong one.
- Inbound-per-node scales the probe config by N ports; the bulk-per-session requirement already caps N to the user's selection.
