# antidpi-engine-correctness Specification

## Purpose

Defines anti-DPI engine, firewall ownership, and packet processing correctness.

## Requirements

### Requirement: Netlink transport preserves kernel errnos and enforces monotonic deadlines
`Netlink::transact` SHALL return kernel `NLMSG_ERROR` replies to callers for typed classification (`Error::Conflict` on `EEXIST`), recompute `SO_RCVTIMEO` before every receive without ever passing `{0, 0}`, and retry `EINTR` while budget remains.

#### Scenario: Concurrent table creation classifies as Conflict on real netlink
- **WHEN** `NEWTABLE` receives a kernel `-EEXIST` reply over `Netlink::transact`
- **THEN** `install` SHALL return `Error::Conflict`, drop only that family's unacquired journal flag, and avoid rolling back a foreign table

#### Scenario: Sub-microsecond remaining budget never disables socket timeout
- **WHEN** less than 1 microsecond remains in the monotonic budget before `recvmsg`
- **THEN** `Netlink::transact` SHALL clamp `tv_usec` to at least `1` (or fail `TimedOut` if zero) and SHALL shrink `SO_RCVTIMEO` on each subsequent datagram

### Requirement: nftables rules bound TCP packet counts accurately and leave IPv6 unqueued
`tcp_rule_exprs` SHALL convert `ct packets` to network byte order before comparing against `1..=12`, and `Family::V6` SHALL install only an owned pass-through table without queueing IPv6 traffic into NFQUEUE.

#### Scenario: Long-lived TCP flow stops entering NFQUEUE after packet 12
- **WHEN** a TCP/443 connection sends more than 260 packets on a little-endian host
- **THEN** packets 257-267 SHALL NOT re-enter NFQUEUE

#### Scenario: IPv6 port-443 traffic bypasses NFQUEUE
- **WHEN** `install-rules` installs both family tables
- **THEN** the `ip6` table SHALL carry the session marker without any `queue` statements

### Requirement: Sweep probing and journal I/O fail closed without PID wraparound or TOCTOU
`fw::sweep` SHALL propagate netlink probe errors without mutating the journal, `owner_liveness` SHALL reject PIDs outside `1..=i32::MAX`, and journal writes SHALL pin the parent directory fd without `/tmp` fallbacks.

#### Scenario: Probe failure during sweep preserves the journal
- **WHEN** `sweep_at` encounters a probe failure on either a prior-boot or same-boot dead-owner journal
- **THEN** `sweep_at` SHALL return an error without calling removal or rewriting the journal

#### Scenario: Out-of-range PID never signals a process group
- **WHEN** `owner_liveness` evaluates a journal with `pid == 0` or `pid > i32::MAX`
- **THEN** the helper SHALL NOT pass a non-positive value to `libc::kill`

### Requirement: Packet, TLS, QUIC, and raw socket layers enforce wire bounds
`Packet::parse`, `OutBuilder`, `tls`, `quic`, and `RawSender` SHALL enforce IPv4/UDP/TCP/TLS length bounds, reject IPv4 fragments, apply non-zero UDP `badsum`, avoid panics on malformed inputs, and close raw sockets on setup failure.

#### Scenario: TCP MD5 option does not overflow data_offset
- **WHEN** `tcp_md5` is applied to a packet whose original options exceed 20 bytes
- **THEN** `OutBuilder` SHALL keep TCP options at or below 40 bytes and never emit a zero `data_offset`

#### Scenario: UDP badsum corrupts checksum to a non-zero value
- **WHEN** `fake:blob=quic:badsum` builds a UDP decoy
- **THEN** the emitted UDP checksum SHALL be non-zero and invalid

#### Scenario: Malformed ClientHello padding never panics
- **WHEN** `pad_to_size` processes a ClientHello with trailing bytes or an out-of-range extension length
- **THEN** it SHALL return an error instead of panicking

### Requirement: NFQUEUE loop is poll-driven, fail-open, protocol-scoped, and strictly parsed
The engine SHALL wait on `queue.as_raw_fd()` via `poll` and enable `NFQA_CFG_F_FAIL_OPEN`.
It SHALL partition configured chains into separate TCP and UDP lists in their original order.
Each list SHALL start at its first chain.
The engine SHALL reject unknown strategy keys or unsafe parameters, bound flow memory with LRU/TTL eviction, and match hostlists in O(labels).

#### Scenario: Default chains select each protocol independently
- **WHEN** the engine starts with `DEFAULT_CHAINS`
- **THEN** TCP/443 ClientHellos SHALL use the TCP list and UDP/443 QUIC Initials SHALL use the UDP list

#### Scenario: TCP failures reach the rotation threshold
- **WHEN** the first reply RST on each fired TCP flow reaches the configured failure threshold within the rotation window
- **THEN** the engine SHALL advance the TCP list
- **AND** the engine SHALL NOT advance the UDP list

#### Scenario: UDP traffic has no automatic rotation signal
- **WHEN** the engine processes UDP traffic
- **THEN** the UDP list SHALL remain on its first chain because no UDP failure signal invokes rotation

#### Scenario: Unknown strategy key is rejected before kernel mutation
- **WHEN** a user supplies `fake:blob=tls:sni=example.com` or `fake:blob=0xaé` or `repeats=99999`
- **THEN** strategy parsing SHALL return an error without panicking or installing rules
