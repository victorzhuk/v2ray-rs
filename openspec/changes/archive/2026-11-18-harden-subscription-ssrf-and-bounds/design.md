# Design

## Context

See `proposal.md` for motivation. `crates/subscription/src/fetch.rs` already defines `redirect_target_is_public` and `ip_is_private_or_local` inside `redirect_policy`, leaving two gaps:
1. `fetch_with_client` parses the initial URL and checks only `http` / `https` scheme before calling `client.get(parsed).send().await`. Neither `subscriptions/dialogs.rs` nor `wizard.rs` rejects private hosts either.
2. `reqwest` resolves hostnames via the OS resolver without checking resolved `SocketAddr`s against `ip_is_private_or_local`, permitting DNS rebinding on both initial requests and redirect targets.
3. `MAX_SUBSCRIPTION_SIZE` caps raw payload bytes at 10 MiB, which still fits ~150,000 compact `ss://` lines; `SubscriptionsPage::render_list` builds an `adw::ActionRow` + controls per node synchronously on the GTK thread.

## Goals / Non-Goals

**Goals:**
- Enforce one uniform public-target policy across initial URLs, redirects, and resolved DNS addresses for all production subscription HTTP fetches.
- Surface immediate validation feedback in UI subscription forms when an entered URL (or decoded `v2raytun://import/` payload) targets a non-public literal/localhost, while keeping `fetch` authoritative.
- Bound per-subscription node count to `MAX_SUBSCRIPTION_NODES = 4_096` before persistence or UI rendering.

**Non-Goals:**
- Rewriting `SubscriptionsPage` from `adw::ExpanderRow` to virtualized `gtk::ListView` in this change.

## Decisions

1. **Check initial URL in `fetch_with_client` and filter DNS answers via `reqwest::dns::Resolve`.**
   - Call `target_is_public(&parsed)` in `fetch_with_client` before `client.get(parsed)` and return `FetchError::InvalidUrl` (non-transient, zero retries) on failure.
   - Implement `reqwest::dns::Resolve` using `tokio::net::lookup_host`, filtering out/rejecting any lookup where an address satisfies `ip_is_private_or_local(addr.ip())`. Wire this resolver into `SubscriptionService::new()`'s default `reqwest::Client`.
   - Keep `SubscriptionService::with_client(...)` (or `#[cfg(test)]` loopback client helper) for existing local `127.0.0.1` HTTP integration tests in `crates/subscription`, and separate initial-URL validation into production vs loopback-test client policy (or enforce `target_is_public` in the production client path) so existing local HTTP server tests (`e2e.rs`, retry/redirect tests) remain deterministic while testing the public guard directly in dedicated unit/integration tests.
   - Gate loopback/private target allowance behind a `#[cfg(any(test, feature = "test-utils"))]` client constructor (`allow_private_targets = false` on `SubscriptionService::new()` and `fetch_url()`) so every production caller strictly rejects private initial URLs and private DNS answers while local `127.0.0.1` test fixtures opt in explicitly.

2. **Enforce `MAX_SUBSCRIPTION_NODES = 4_096` in `SubscriptionService` / import entrypoints.**
   Stop parsing or reject once parsed node count exceeds `4_096` in both line-based URI parsing and `json_import`, returning a dedicated `SubscriptionError` variant (e.g., `TooManyNodes { limit: usize }`) before reconciling or saving.

## Risks / Trade-offs

- **[Risk] Self-hosted LAN subscription servers blocked by SSRF guard** -> Matches the existing documented security posture already enforced on redirects (`fetch.rs:32-46`); local files and pasted JSON remain supported for offline/LAN-managed configs.
- **[Risk] Legitimate mega-aggregators exceeding 4,096 nodes** -> 4,096 nodes is an order of magnitude above normal commercial subscriptions (typically 20-300 nodes) while keeping GTK widget construction and JSON persistence well under interactive latency limits.
- **[Risk] Environment HTTP proxies (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`) move DNS resolution to the proxy**, so the connector-level resolver never sees the answer and the initial-URL name check is the only local guard. -> Accepted: honoring the environment proxy is required for users who can reach subscriptions only through a corporate or censorship proxy, and the user's own proxy configuration is a trusted local input. Direct (unproxied) fetches keep both checks. Disabling proxy support would break a supported deployment to defend against the same user's own environment.
