# Tasks

## 1. Initial-URL and DNS-Resolution SSRF Protection

- [x] 1.1 Enforce public-target validation on initial subscription URLs and resolved DNS `SocketAddr`s (`reqwest::dns::Resolve` + `ip_is_private_or_local`) on the production subscription HTTP path in `crates/subscription/src/fetch.rs` and `service.rs`, exposing a `#[cfg(any(test, feature = "test-utils"))]` loopback-capable client constructor for local HTTP fixtures; verify with unit and integration tests covering `127.0.0.1`, `169.254.169.254`, `[::1]`, `[::ffff:10.0.0.1]`, `localhost`, and a hostname resolving to a private IP (asserting zero TCP connections and zero retries).
- [x] 1.2 Validate `http`/`https` scheme and public host target on subscription URL inputs (after `strip_v2raytun_import_prefix`) in `crates/ui/src/subscriptions/dialogs.rs` and `crates/ui/src/wizard.rs`, and verify with unit tests covering raw private URLs and base64-encoded `v2raytun://import/` private URLs.

## 2. Per-Subscription Node Count Cap

- [x] 2.1 Enforce `MAX_SUBSCRIPTION_NODES` (`4_096`) during URI-list and JSON-bundle subscription parsing/import in `crates/subscription`, returning a descriptive error without mutating stored subscription nodes on refresh; verify with unit and service refresh tests covering 4,096-node acceptance and 4,097-node rejection with prior nodes preserved.
