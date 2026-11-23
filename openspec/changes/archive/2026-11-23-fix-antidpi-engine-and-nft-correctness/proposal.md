# Proposal: fix-antidpi-engine-and-nft-correctness

## Why

Adversarial review and test execution of `crates/antidpi` exposed multiple correctness, performance, and reliability defects in the netlink/nftables firewall client, ownership journal, packet builder/parser, and NFQUEUE engine loop:

- `Netlink::transact` returns `Err(io::Error)` on kernel `NLMSG_ERROR` replies, making `Error::Conflict` (`EEXIST`) unreachable on real netlink sockets and stranding the journal in `Phase::Cleaning` after a failed rollback.
- `tcp_rule_exprs` compares native-endian `u64` `ct packets` counters with big-endian `memcmp`, leaking 11 of every 256 packets (~4.3%) on every long-lived TCP/443 connection into NFQUEUE forever.
- `InstallPlan::build` installs active NFQUEUE rules in the `ip6` table, contradicting `docs/antidpi-registry.md` ("pass-through ownership artifact; queues nothing") and stalling IPv6 port-443 traffic in the engine loop only to reject it as non-IPv4.
- `Netlink::transact` can pass `SO_RCVTIMEO = {0, 0}` (infinite wait) when under 1 microsecond remains, never shrinks the timeout across datagrams, and does not retry `EINTR`.
- `fw::sweep()` masks probe errors as "table present" instead of using `nft::name_present`, and `decide_sweep` currently fails unit test `fw::tests::sweep_probe_failure_refuses_and_preserves_the_record`.
- The NFQUEUE loop uses a 50 ms `thread::sleep` on `WouldBlock` (adding 150-400 ms per TLS handshake), never enables `NFQA_CFG_F_FAIL_OPEN`, crashes on `EINTR`/`ENOSPC`, and shares one `chain_idx` across mixed TCP and QUIC chains in `DEFAULT_CHAINS`.
- Packet/TLS/strategy handling has multiple safety bugs: TCP `data_offset` overflow with `tcp_md5`, `OutBuilder::build_udp` ignoring `badsum`, `Packet::parse` ignoring IPv4 total length / fragments / UDP length, OOB panic in `tls::find_padding_ext`, non-ASCII panic and odd-length truncation in `decode_hex`, silent acceptance of unknown strategy keys, and unbounded `repeats`.

## What Changes

- Fix `Netlink::transact` to return kernel `NLMSG_ERROR` payloads in `Ok(acc.buf)`, shrink `SO_RCVTIMEO` per loop iteration with a 1-microsecond floor, and retry `EINTR`.
- Fix `tcp_rule_exprs` to load `ct_pkts` once, convert to network byte order (`NFT_BYTEORDER_HTON`, 8 bytes), and compare big-endian bounds `1..=12`; make `Family::V6` install an owned pass-through table with no queueing rules.
- Fix `fw::sweep()` to propagate `nft::name_present` errors, align `decide_sweep` with `sweep_probe_failure_refuses_and_preserves_the_record`, and add the pre-install `name_present` check after `InstallAction::Clean`.
- Harden `state.rs`: reject `pid == 0` and `pid > i32::MAX` before `libc::kill`, use `DirBuilderExt::mode(0o700)` and fd-pinned `openat`/`renameat` for journal writes, remove the `/tmp` and empty-`HOME` fallbacks, and unify UID checks on `geteuid()`.
- Harden packet, TLS, QUIC, and raw socket layers: return `Result` from `OutBuilder::build()` with TCP option (<=40 B) and IPv4 total length (<=65535 B) checks, apply non-zero `badsum` in `build_udp`, enforce IPv4 total length / non-fragment / UDP length in `Packet::parse`, fix `tls::find_padding_ext` bounds and TLS length caps, switch `OsRng` to `libc::getrandom`, fix `RawSender::new` fd leak + `SOCK_CLOEXEC`, use wrapping/saturating arithmetic in `fooling.rs`, filter split position `0`, and check QUIC fixed bit + QUIC v2.
- Fix the NFQUEUE engine and strategy DSL: replace 50 ms `thread::sleep` with `libc::poll` on `queue.as_raw_fd()`, enable `queue.set_fail_open`, handle `EINTR`/`ENOSPC` non-fatally, separate TCP and QUIC strategy rotation, reject unknown/duplicate strategy keys and cap `repeats`/`seqovl`, count at most one RST per flow, use TTL/LRU flow eviction, pass `reply_ttl` to UDP strategies, re-randomize decoy payloads per repeat, and switch hostlist matching to an O(labels) `HashSet`.
- Add privileged netns tests covering real `install-rules` traffic (TCP/443 > 260 packets, UDP/443, IPv6/443), fix `tests/privileged.rs:432`, and update the stale note in `docs/antidpi-registry.md`.

## Capabilities

### New Capabilities
- `antidpi-engine-correctness`: Correct nftables netlink rule encoding, conflict/timeout handling, journal/PID safety, packet/TLS/QUIC bounds safety, protocol-scoped NFQUEUE processing, and strict strategy DSL validation in `v2ray-rs-antidpi`.

### Modified Capabilities
- None (refines internal engine guarantees beneath the `antidpi-integration` contract).

## Impact

- **Modified crate**: `crates/antidpi` (`nft.rs`, `fw.rs`, `state.rs`, `packet.rs`, `rawsend.rs`, `fooling.rs`, `positions.rs`, `tls.rs`, `quic.rs`, `strategy.rs`, `engine.rs`, `main.rs`, `tests/privileged.rs`).
- **Docs**: `docs/antidpi-registry.md` (remove stale lifecycle test note; clarify protocol-scoped default chains).
- **Dependencies**: None new (`libc` and `nfq` are already direct dependencies).
