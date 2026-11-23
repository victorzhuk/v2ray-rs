# Tasks: fix-antidpi-engine-and-nft-correctness

## 1. Netlink transport and nftables rules (`nft.rs`)

- [x] 1.1 Update `Netlink::transact` (`crates/antidpi/src/nft.rs:1436-1438`) to return `Ok(acc.buf)` when `Replies` records a valid kernel `NLMSG_ERROR`, preserving `Err` only for transport/framing/non-kernel errors so `Error::Conflict` (`EEXIST`) fires on real sockets.
- [x] 1.2 Recompute remaining deadline before each `recvmsg` in `Netlink::transact` (`nft.rs:1356-1448`), clamp `tv_usec >= 1` when `tv_sec == 0`, and retry `sendmsg`/`recvmsg` on `EINTR` while budget remains.
- [x] 1.3 Fix `tcp_rule_exprs` (`nft.rs:1165-1179`): drop dead `immediate_u64` calls and duplicate `ct_pkts`, emit `byteorder` (`NFT_BYTEORDER_HTON`, `len=8, size=8`), and compare against big-endian `1..=12`.
- [x] 1.4 Update `InstallPlan::build` (`nft.rs:818-893`) so `Family::V6` creates only the owned table with the session userdata marker and no NFQUEUE chains/rules; update `nft.rs` unit tests accordingly.

## 2. Firewall sweep, journal safety, and PID validation (`fw.rs`, `state.rs`)

- [x] 2.1 Update `fw::sweep()` (`crates/antidpi/src/fw.rs:693-709`) to pass `nft::name_present` directly instead of swallowing probe errors into `Ok(true)`.
- [x] 2.2 Probe recorded families in `decide_sweep` (`fw.rs:237-259`) before returning `SweepDecision::Drain` so `fw::tests::sweep_probe_failure_refuses_and_preserves_the_record` passes without mutating the journal or invoking removal.
- [x] 2.3 Run the `name_present` preflight check for both families after `InstallAction::Clean` (`fw.rs:423-436`) before calling `install_families`.
- [x] 2.4 Validate `1 <= pid <= i32::MAX as u32` in `state::parse_owner`, journal validation, and `state::owner_liveness` (`crates/antidpi/src/state.rs:464-480`) before any `libc::kill` call.
- [x] 2.5 Replace `create_dir` + `set_permissions` with `DirBuilderExt::mode(0o700)` (`state.rs:243-248, 306-308`), switch `write_journal` (`state.rs:168-185`) to fd-pinned `openat`/`renameat`, remove the `/tmp` and empty-`HOME` fallbacks in `state_path` (`state.rs:115-130`), and unify UID checks on `geteuid()`.

## 3. Packet, TLS, QUIC, and raw socket bounds (`packet.rs`, `tls.rs`, `quic.rs`, `rawsend.rs`, `fooling.rs`, `positions.rs`)

- [x] 3.1 Change `OutBuilder::build()` (`crates/antidpi/src/packet.rs:253-367`) to return `Result<OutPacket>`, enforcing `options.len() <= 40` and `total <= u16::MAX`; trim/replace decoy TCP options when `tcp_md5` would exceed 40 bytes (`fooling.rs:108-124`).
- [x] 3.2 Implement `self.badsum` in `OutBuilder::build_udp` (`packet.rs:321-367`) ensuring the corrupted checksum is never `0x0000`.
- [x] 3.3 Enforce IPv4 `Total Length`, reject fragmented IPv4 packets (`(flags_frag & 0x3FFF) != 0`), and enforce UDP `Length` in `Packet::parse` (`packet.rs:51-107`).
- [x] 3.4 Bound `tls::find_padding_ext` to `exts_end` (`p + 4 + elen <= exts_end`), enforce TLS length bounds in `replace_sni`/`pad_to_size`/`craft_client_hello`, fix 32-bit length encoding (`tls.rs:252`), and replace per-call `/dev/urandom` open+expect with `libc::getrandom` (`tls.rs:390-408, 501-508`).
- [x] 3.5 Add `SOCK_CLOEXEC` and wrap the raw socket fd in `OwnedFd` immediately in `RawSender::new` (`rawsend.rs:15-50`).
- [x] 3.6 Use wrapping arithmetic for TCP `seq`/`ack` offsets and saturating arithmetic in `autottl`, reject trailing tokens in `AutoTtl::parse` (`fooling.rs:18-101`), filter split offset `0` (`positions.rs:88-195`), and check QUIC fixed bit `0xC0` + QUIC v2 (`quic.rs:12-19`).

## 4. Engine loop, strategy DSL, and privileged tests (`engine.rs`, `strategy.rs`, `main.rs`, `tests/privileged.rs`, `docs/antidpi-registry.md`)

- [x] 4.1 Replace `thread::sleep(50ms)` in `engine.rs:166` with `libc::poll` on `queue.as_raw_fd()`, call `queue.set_fail_open(queue_num, true)` before `publish_ready`, and handle `EINTR`/`ENOSPC` without exiting the loop.
- [x] 4.2 Split TCP and UDP/QUIC strategy chains with independent rotation indices (`main.rs:13-17`, `engine.rs:243-383`) and set `entry.fired = true` only when an attack matched the packet's protocol.
- [x] 4.3 Add strict allowlist validation for strategy keys/duplicates/booleans in `strategy.rs:90-219`, fix `decode_hex` non-ASCII panic and odd-length truncation, and cap `repeats <= 64` and `seqovl <= 1500`.
- [x] 4.4 Count at most one RST per flow (`failure_counted: bool`), reject `fails == 0` in `parse_rotation` (`main.rs:94-119`), avoid inserting non-QUIC UDP flows, use `last_seen` LRU/TTL eviction (`engine.rs:537-545`), pass `reply_ttl` in `execute_chain_udp`, re-randomize per repeat (`engine.rs:290-381`), and use a `HashSet<Vec<u8>>` for O(labels) hostlist matching with HTTP `:port` stripping (`engine.rs:59-78`, `positions.rs:140-146`).
- [x] 4.5 Fix `tests/privileged.rs:432` to use valid `tls_mod=sni=...` syntax, add a privileged netns test sending real TCP/443 (>260 packets), UDP/443, and IPv6/443 traffic through `install-rules`, and remove the stale lifecycle test caveat from `docs/antidpi-registry.md:323-325`.

## 5. Verification

- [x] 5.1 Run `timeout 3m cargo test -p v2ray-rs-antidpi -- --test-threads=4` and verify all unit tests pass (including `sweep_probe_failure_refuses_and_preserves_the_record` and new regression tests for `EEXIST`, `ct packets`, IPv6 pass-through, `badsum` UDP, `tcp_md5` overflow, `decode_hex`, and strategy key allowlist).
- [x] 5.2 Run `cargo fmt --check` and `cargo clippy -p v2ray-rs-antidpi --all-targets --all-features -- -D warnings`.
- [x] 5.3 Where privileged netns prerequisites exist, run `timeout 5m cargo test -p v2ray-rs-antidpi --features privileged-tests -- --test-threads=1` and verify no skips on the `install-rules` traffic test.