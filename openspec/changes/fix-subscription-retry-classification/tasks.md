## 1. Error taxonomy

- [ ] 1.1 Add `FetchError::InvalidUrl` (terminal) and return it from the scheme/parse check in `fetch.rs` instead of `NetworkError`
- [ ] 1.2 Classify transport errors from reqwest into transient (timeout/connect) vs terminal (builder/request construction), verifying predicate names against the pinned reqwest version
- [ ] 1.3 Add `FetchError::is_transient()` covering: HttpError 408/429/5xx transient, other 4xx terminal, Timeout transient, InvalidUrl terminal

## 2. Retry loop

- [ ] 2.1 In `fetch_with_retry`, return immediately on terminal errors (no sleep, no retry); keep existing backoff for transient errors

## 3. Tests

- [ ] 3.1 Unit tests for `is_transient` across every variant/status bucket
- [ ] 3.2 Test that a terminal error short-circuits `fetch_with_retry` (single attempt, no sleep) and a transient error retries up to the cap

## 4. Verification

- [ ] 4.1 `cargo test -p v2ray-rs-subscription` green; `cargo clippy` clean
