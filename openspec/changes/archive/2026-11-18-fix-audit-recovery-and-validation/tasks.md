## 1. Runtime fixes
- [x] 1.1 Propagate route cleanup failures and retain recovery markers.
- [x] 1.2 Exclude response bodies, bound file imports, and fix reconciliation.
- [x] 1.3 Preserve generated config on routing load errors.
## 2. Delivery
- [x] 2.1 Bound CI tests and require display-backed execution.
- [x] 2.2 Gate release publication on quality checks and version consistency.
- [x] 2.3 Correct architecture guidance and changelog.
## 3. Verification
- [x] 3.1 Add focused failure and identity regression scenarios.
- [x] 3.2 Run formatting, Clippy, and bounded workspace tests.
- [x] 3.3 Prove resource ownership isolation in disposable privileged environments.
- [x] 3.4 Complete packaged GUI and release acceptance.

## 4. Remaining acceptance and ownership work
- [x] 4.1 Define durable kernel resource ownership and collision refusal before changing privileged cleanup matching.
- [x] 4.2 Provision required xray checks in CI and fail required jobs on missing backend prerequisites.
- [x] 4.3 Pin packaging tool downloads and verify their checksums.
- [x] 4.4 Replace timeout-only AppImage smoke acceptance with observable window readiness and clean shutdown.

The local sing-box routing harness is BLOCKED: /dev/net/tun is absent. Packaged release acceptance is NOT RUN: no prepared artifact or accessible container runtime is available. Display-backed local tests do not discharge those scenarios.

## 5. Continuation fixes
- [x] 5.1 Reproduce and fix the missing device identity on xray setup.
- [x] 5.2 Add a required isolated routing CI job with explicit prerequisite failures.
- [x] 5.3 Approve the recovery ownership schema and migration policy before implementation.

## 6. Approved ownership implementation
- [x] 6.1 Share a versioned, bounded, owner-only journal across core and helper.
- [x] 6.2 Reserve the namespace before spawn and journal kernel mutations.
- [x] 6.3 Refuse foreign resources, stale sessions, epochs, aliases, and unknown outcomes.
- [x] 6.4 Keep legacy and corrupt records; preserve helper-free sing-box startup.
- [x] 6.5 Verify real-kernel reservation, IPv4/IPv6 normalization, collision refusal, and cleanup in a private namespace.
- [x] 6.6 Complete the full TUN device, replaced-link, and packaged acceptance matrix.

- [x] 6.7 Verify sing-box state absence without mutation before completing a clean stop.
- [x] 6.8 Prove the read-only check preserves foreign rules and routes in a private namespace.

- [x] 6.9 Reject the reserved interface name before journal creation.
- [x] 6.10 Reproduce and fix stale ledger entries after kernel-removed routes.

Supporting namespace ownership checks passed. Task 3.3 remains open for the full
TUN/device-identity rows; journal-only checks do not satisfy that acceptance floor.

## 7. Review corrections
- [x] 7.1 Refuse journaled EEXIST outcomes and verify the collision in a private namespace.
- [x] 7.2 Renew completed ownership on explicit start while preserving unfinished session identity.
- [x] 7.3 Serialize automatic helper commands on an owner-only lock file beside the journal; the global abstract-name lock was squattable by any local user.
- [x] 7.4 Retire finished journals before the boot and namespace gate, and keep unfinished journals across a clean stop of an unrelated session.
- [x] 7.5 Invalidate persisted and in-flight latency samples when a connection's settings change.
- [x] 7.6 Make absent rule and route deletes idempotent, pass the observed TUN index to route setup, and fail an xray setup whose device vanished.
- [x] 7.7 Run containerized release steps under Bash and align local AppImage acceptance with the window-readiness gate.
