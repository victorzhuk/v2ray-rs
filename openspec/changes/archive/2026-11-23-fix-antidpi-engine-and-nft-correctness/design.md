# Design: fix-antidpi-engine-and-nft-correctness

## Context

`crates/antidpi` is a privileged helper (`cap_net_admin,cap_net_raw+ep`) with a hand-written nf_tables netlink client, a custom IPv4/TCP/UDP/TLS/QUIC packet processor, and a single-threaded NFQUEUE loop. Bugs in netlink error propagation, endianness, socket timeouts, packet bounds, or polling directly impact host networking, latency, and rule cleanup reliability.

## Decisions

### 1. Kernel `NLMSG_ERROR` delivery and monotonic socket timeouts (`nft.rs`)
- Separate framing/transport errors (`MSG_TRUNC`, non-kernel `nl_pid`, truncated headers, timeout) from valid kernel `NLMSG_ERROR` replies in `Netlink::transact` (`nft.rs:1436-1438`). When `Replies` finishes with a kernel error message, return `Ok(acc.buf)` so `install_verdict`, `lookup_with`, and `remove_with` observe the exact same byte stream on real sockets as in `ScriptedTx` unit tests.
- At the top of every `recvmsg` iteration, compute `remaining = deadline.saturating_duration_since(Instant::now())`. Return `io::ErrorKind::TimedOut` if `remaining.is_zero()`. Convert `remaining` to `timeval` clamping `tv_usec >= 1` when `tv_sec == 0` so `{0, 0}` (infinite wait) is never installed. Reapply `SO_RCVTIMEO` before each `recvmsg` and retry `sendmsg`/`recvmsg` on `EINTR` while time remains.
- The kernel-error discriminator is a non-zero errno in `Replies::error`: that alone separates a valid `NLMSG_ERROR` reply (returned as data) from a framing failure (returned as an error).

### 2. Accurate `ct packets` comparison and pass-through IPv6 table (`nft.rs`)
- In `tcp_rule_exprs` (`nft.rs:1165-1179`), load `ct_pkts(direction, NFT_REG32_00)` once, drop both dead `immediate_u64` expressions and the duplicate `ct_pkts`, emit `byteorder` (`NFT_BYTEORDER_HTON`, `sreg=dreg=NFT_REG32_00`, `len=8`, `size=8`), and compare against `CT_PACKET_LOW.to_be_bytes()` and `CT_PACKET_HIGH.to_be_bytes()`.
- In `InstallPlan::build` (`nft.rs:818-893`), when `family == Family::V6`, emit only the owned table carrying the 16-byte session `NFTA_TABLE_USERDATA` marker (no queueing chains/rules), matching `docs/antidpi-registry.md` and preventing IPv6 port-443 traffic from entering NFQUEUE.
- The `ip6` batch has 3 messages: batch begin, marked `NEWTABLE`, batch end. `install_with` derives its expected acks with `collector_for(&batch)` instead of a hard-coded index table, so the two batch shapes share one collector.

### 3. Fail-closed sweep probing and clean-then-install preflight (`fw.rs`)
- In `fw::sweep()` (`fw.rs:693-709`), call `nft::name_present(family, &budget, Duration::ZERO)` directly so kernel/netlink errors propagate instead of turning into `Ok(true)`.
- In `decide_sweep` (`fw.rs:237-259`), probe recorded families via `present_family` before returning `SweepDecision::Drain` (failing closed and leaving the journal untouched if the probe errors), restoring `sweep_probe_failure_refuses_and_preserves_the_record`.
- After `InstallAction::Clean` finishes `clean_session` (`fw.rs:423-436`), run the same `name_present` check for both families before calling `install_families`.

### 4. PID validation and fd-pinned journal I/O (`state.rs`)
- Validate `1 <= pid <= i32::MAX as u32` in `parse_owner`, journal validation, and `owner_liveness` (`state.rs:464-480`) before any `libc::kill` call.
- Create missing journal/runtime directories atomically with `DirBuilder::new().mode(0o700)`.
- Write journals using `openat` (`O_CREAT | O_EXCL | O_RDWR | O_NOFOLLOW | O_CLOEXEC`, mode `0600`) + `fsync` + `renameat` against the pinned parent directory fd from `open_journal_parent`.
- Return an explicit error when `V2RAYRS_ANTIDPI_STATE` is unset and `HOME` is unset or empty (no `/tmp` or relative-path fallback). Use `libc::geteuid()` consistently across journal, lock, and table checks.
- An out-of-range pid yields `Liveness::Unknown` before any `libc::kill`, `read_journal` rejects a journal whose owner pid is out of range, and `state_path` errors on a missing or relative `HOME` and on a relative `XDG_STATE_HOME`. The dead-owner test fixture pid is `2_000_000_000`: above the kernel pid limit, inside the signed `pid_t` range.

### 5. Packet, TLS, QUIC, and raw socket bounds safety
- Change `OutBuilder::build()` to return `Result<OutPacket>`. Enforce `options.len() <= 40` and `total <= u16::MAX as usize`. When `tcp_md5` would push options over 40 bytes, replace original TCP options on the decoy with the MD5 option only.
- Apply `self.badsum` in `OutBuilder::build_udp`, XOR-corrupting the checksum and guaranteeing the result is non-zero (`0x0000` disables UDP checksum verification in IPv4).
- In `Packet::parse`, read and validate IPv4 `Total Length` (`ihl <= total_len <= bytes.len()`), reject fragmented IPv4 packets (`(flags_frag & 0x3FFF) != 0`), and bound UDP payload using validated `udp_len` (`8 <= udp_len <= rest.len()`).
- In `tls.rs`, bound `find_padding_ext` to `exts_end` with `p + 4 + elen <= exts_end`, enforce TLS record/u16 length caps in `replace_sni`/`pad_to_size`/`craft_client_hello`, fix 32-bit length encoding (`&(hs_len as u32).to_be_bytes()[1..4]`), and switch `OsRng::bytes` to `libc::getrandom` with `EINTR` retry.
- In `rawsend.rs`, open the socket with `SOCK_RAW | SOCK_CLOEXEC` and immediately wrap in `OwnedFd` so `setsockopt` failures never leak descriptors.
- In `fooling.rs`, use wrapping arithmetic for TCP `seq`/`ack` offsets, saturating arithmetic in `autottl`, and reject trailing tokens in `AutoTtl::parse`.
- In `positions.rs`, filter split position `0` (must be strictly `0 < pos < data.len()`). In `quic.rs`, require fixed bit `0xC0` and match both QUIC v1 and QUIC v2 Initial packets.
- `Fooling::seq_off` and `Fooling::ack_off` are `i32` applied with wrapping arithmetic, `craft_client_hello` returns `Result<Vec<u8>>`, `pad_to_size` returns `Result<()>` and leaves the buffer untouched on refusal, and `resolve_multi` also drops position `0` (position `1` was already dropped).

### 6. Low-latency NFQUEUE loop, protocol-scoped chains, and strict DSL
- Replace `std::thread::sleep(50ms)` in `engine.rs:166` with `libc::poll` on `queue.as_raw_fd()` (`POLLIN`, 50 ms timeout) so packets process immediately while shutdown checks remain responsive.
- Call `queue.set_fail_open(queue_num, true)` before `publish_ready`; treat `EINTR` on `recv`/`verdict` as loop-continue (re-checking `shutdown()`), and log-and-continue on transient `ENOSPC` / `ENOENT`.
- Partition chains into TCP-applicable and UDP/QUIC-applicable lists with independent rotation indices so `DEFAULT_CHAINS` protects QUIC from packet 1 and TCP never stalls on a QUIC-only chain; set `entry.fired = true` only when an attack actually executes on the packet.
- In `strategy.rs`, validate every parameter key against the attack's allowlist, reject unknown/duplicate keys and invalid boolean values, verify `s.is_ascii() && !s.is_empty() && s.len() % 2 == 0` in `decode_hex`, and bound `repeats <= 64` and `seqovl <= 1500`.
- Track `failure_counted: bool` per `Flow` so multiple RSTs on one connection count once; reject `fails == 0` in `parse_rotation`.
- Insert UDP `Flow` entries only after `looks_like_quic_initial` succeeds; track `last_seen: Instant` per flow and evict oldest/expired entries rather than dropping active `fired` flows or clearing the entire map.
- Pass `entry.reply_ttl` in `execute_chain_udp`; re-randomize `Blob::Quic` and `tls_mod=rnd|rndsni` inside the repeat loop.
- Store normalized hosts (stripping leading/trailing dots) in `HashSet<Vec<u8>>` for O(labels) suffix lookup, and strip `:port` when matching HTTP `Host`.
- The partition predicates are `Chain::applies_to_tcp` (any split, or a fake whose blob is not QUIC) and `Chain::applies_to_udp` (a fake whose blob is not TLS). `repeats` is bounded to `1..=64`, flow entries expire after 60 seconds of silence, eviction removes the oldest quarter once the idle pass leaves the table full, `parse_rotation` rejects `fails == 0`, and the engine prints one `trace` line per queued packet carrying direction, protocol, endpoints, and length — never payload bytes.

## Risks / Trade-offs

- **Strict strategy key validation**: Previously ignored typos (including `tests/privileged.rs:432`) will now return a parse error at `check` and `install-rules` time. This is intended and surfaces silent misconfiguration before backend start; `tests/privileged.rs:432` is updated alongside.
- **Pass-through IPv6 table**: Existing unit test `v6_differs_from_v4_only_by_nfgen_family` must be replaced by tests asserting IPv6 installs an owned table without queueing rules.
