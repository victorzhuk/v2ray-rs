# Design

## Context
See proposal.md. Existing atomic persistence and routing tables remain in use.

## Goals / Non-Goals
Keep failures observable and retryable. Preserve existing settings and subscription formats; version the TUN recovery journal.

## Decisions
Use existing-service-strict verification depth. Cleanup attempts independent steps and aggregates errors. Absent resources remain idempotent. Keep failed markers without blocking desktop startup. Match exact connection settings without remarks before a unique endpoint fallback. Reset measurements after connection settings change. Read regular local files with a byte cap on a blocking worker. Exclude arbitrary response text from displayed fetch errors.

## Risks / Trade-offs
Ambiguous changed duplicates receive fresh identities. Privileged ownership redesign requires isolated runtime evidence before acceptance. Release and privileged acceptance remain incomplete until their required scenarios execute.

## Migration Plan
Write new TUN journals under a versioned filename. Retain legacy records for explicit recovery. Clean owned networking before rollback. Use disposable namespaces or VMs for routing verification. Never mutate host routing during development.

## Approved recovery ownership phase

The current marker contains only backend and interface name. Runtime ifindex pins
are lost at application exit, and reserved rule priorities do not establish
ownership. The approved ownership phase introduces the versioned recovery format described below.

1. Introduce a versioned recovery journal with a session identifier, boot identity,
   network namespace identity, device identity, and exact owned route/rule records.
   Record intent before mutation and completion after each acknowledged operation.
2. Refuse setup when required tables, rules, or interface names conflict with
   resources outside the recorded session. Serialize mutations within each network
   namespace across runtime profiles. A per-profile UI lock is insufficient.
3. Bind xray's reservation to an independently surviving kernel ownership marker,
   created by the helper before route mutation. Refuse automatic cleanup if that
   marker is missing or has a different identity. Delete only exact journaled
   resources. Keep the existing capability and group authorization boundary.
4. Preserve sing-box's helper-free startup contract. When its previous ownership
   cannot be established, report pending recovery and require explicit operator
   intervention instead of flushing shared tables or deleting a live device.
5. Read legacy markers for diagnostics, but refuse destructive automatic recovery
   without identity evidence. Require explicit recovery for ambiguous legacy state.
   A boot or namespace mismatch must never authorize changes to the current network.
6. Retain the journal after any failure. Mark recovery complete only after verifying
   the owned resources are absent. Do not silently downgrade a new journal to the
   old name-only marker on rollback.

Verification depth remains existing-service-strict. Required scenarios: foreign
rules at matching priorities, reused device names and indices, overlapping runtime
profiles, reboot and namespace changes, interrupted setup, partial cleanup, and
legacy markers. Run them in disposable VMs or private namespaces. Add focused
journal interruption tests for failure points that cannot be scheduled reliably
through a GUI. Freeze automatic destructive recovery if ownership is ambiguous.

The ownership schema and legacy behavior were approved before implementation.
Core and netctl share the journal crate; no protocol or geodata code enters the
helper dependency graph. Native netlink messages are retained only within their
recorded boot and namespace. Independent controller PID/start-time identity
prevents a restarted desktop from treating a prior run as its own active session.
A separate UUID argument prevents delayed requests from following a replaced file.

The reservation uses a dummy link. The tested kernel ignores an alias supplied
on creation, so the helper records its observed index, explicitly sets the alias,
and verifies it before route mutation. An interrupted untagged reservation remains
ambiguous and requires operator recovery. An abstract Unix socket serializes helper
commands within the namespace; the link preserves ownership between commands.

Focused tests cover malformed and unsafe journal files, pending intent, selector
comparison, and duplicate objects because corruption and ACK/write interruption
cannot be scheduled reliably through the desktop. The real-kernel namespace test
covers reservation, IPv4/IPv6 route normalization, repeated additions, cross-session
refusal, foreign route refusal, and exact cleanup without requiring a TUN device.
The full TUN scenario remains a required separate gate.

Sing-box records Applying before spawning. Its stop path verifies device, route,
and rule absence through unprivileged netlink queries before recording Complete.
The generator explicitly selects its existing table 2022. Verification never
mutates that state and owns its netlink task through cancellation. An unstarted
Prepared record and a verified Complete record need no destructive recovery;
other interrupted sing-box records remain pending for an operator.
