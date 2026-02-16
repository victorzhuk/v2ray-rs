## Context

The app currently flattens enabled nodes from all subscriptions and generates a single config for a backend process. When multiple enabled nodes exist, connection selection is implicit and the UI only indicates connected/disconnected. Users need explicit auto-resolve strategies and clear visibility into which node is active, as well as a deterministic attempt order that stops on first success. This change touches core settings, config generation, process orchestration, and UI/tray surfaces.

## Goals / Non-Goals

**Goals:**
- Provide a global auto-resolve strategy setting that controls how enabled nodes are ordered for connection attempts.
- Build a deterministic, ordered candidate list across subscriptions/nodes that can incorporate latency, last-success, or geo hints.
- Attempt connections sequentially until one succeeds or all fail, with explicit status reporting.
- Surface connection context (state, subscription, node, latency, backend, strategy, since) in a persistent status panel and tray tooltip.
- Track and persist enough metadata (last-success node, latency snapshots, connect timestamps) to support strategies and UI.

**Non-Goals:**
- Manual per-connection node picking or multi-node load balancing.
- Protocol-level health checks beyond existing TCP latency measurement.
- Advanced geo-resolution beyond basic country/region tags already available in node metadata.

## Decisions

- **Introduce a core auto-resolve model and store in settings.**
  - Add a `AutoResolveStrategy` enum in core settings and persist it in `AppSettings`.
  - Rationale: strategy is global and must be accessible to UI, config writer, and process orchestration.
  - Alternative: store per-subscription strategy, rejected for v1 to avoid UX complexity.

- **Centralize candidate ordering in a core helper.**
  - Add a `ConnectionPlanner` (core or process crate) that accepts subscriptions + settings + latency cache and returns ordered candidates with metadata.
  - Rationale: avoid duplicated logic across UI and config generator; enables consistent status reporting.
  - Alternative: keep selection in UI and pass ordered nodes to config writer. Rejected because status/telemetry and tray also need shared context.

- **Sequential connection attempts with per-attempt config generation.**
  - Each candidate attempt generates a config with a single proxy outbound, starts the backend, and treats successful process start plus a short stabilization window as success.
  - Rationale: current backends don’t expose a unified health API; using startup success as signal is pragmatic.
  - Alternative: generate multi-node config and rely on backend routing to select. Rejected because selection becomes opaque and status cannot report the chosen node.

- **Status model propagated via process events.**
  - Add a `ConnectionStatus` struct (state, active candidate, latency, strategy, since) emitted alongside process state events.
  - Rationale: keeps UI and tray synchronized without direct polling.
  - Alternative: UI reads shared state directly; rejected to keep components decoupled.

- **Status panel extends existing UI status bar.**
  - Expand bottom status panel to show primary status + secondary details, still using `gtk::ActionBar` for layout.
  - Rationale: aligns with existing UI status bar spec and avoids large redesign.

## Risks / Trade-offs

- **[Risk]** Sequential attempts increase connect time when early candidates fail → **Mitigation:** cap attempts, show progress in status panel, and surface errors in toast/logs.
- **[Risk]** Latency data can be stale → **Mitigation:** reuse last measured values with timestamps and allow explicit refresh; prioritize list order when latency missing.
- **[Risk]** Using backend start as “success” may not reflect actual connectivity → **Mitigation:** keep behavior consistent with current app, add future hook for health checks.
- **[Risk]** Extra config regeneration per attempt → **Mitigation:** cache generated configs per candidate where possible and clean up temporary files.
