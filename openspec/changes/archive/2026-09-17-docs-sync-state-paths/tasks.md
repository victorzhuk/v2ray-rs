## 1. Architecture doc

- [x] 1.1 `docs/ARCHITECTURE.md` on-disk layout (`:404-436`) and Logging section (heading `:455`): state/runtime headings show the `data_dir/state` and `data_dir/runtime` fallbacks with the rule from `persistence/mod.rs:84-92`; verified by reading the rendered section against `AppPaths` path helpers
- [x] 1.2 Replace the `ProbeRunner` sentence (`:149-150`) with its Real Delay role; `rg -n ProbeRunner crates --type rust` shows no other user
- [x] 1.3 Helper timeout sentence (`:133-135`) lists each call with the constant bounding it — device wait `DEVICE_TIMEOUT`, `xray-up`/`xray-down` and `recover` `HELPER_TIMEOUT` (`crates/process/src/tun.rs:13-15`, used by `recover_tun_session` at `crates/ui/src/app.rs:3682`) — values re-read from the code at edit time (all 10s at planning)
- [x] 1.4 New "File-only settings" subsection listing `tun.address_v6`, `auto_update_geodata`, `geodata_update_interval_secs`, `backend.config_output_dir` with their TOML keys; `rg` over `crates/ui/src/preferences` confirms none has a widget
- [x] 1.5 Per-backend applicability notes (idle timeout, DNS detour, DNS hijack modes) added next to the existing TUN/DNS prose; each claim cross-checked against the generator lines cited in design.md

## 2. CLAUDE.md

- [x] 2.1 Persistence summary states `latency_snapshot.json`, `tun_session.json` and `logs/` live under `state_dir` (`$XDG_STATE_HOME/<qualifier>` or `data_dir/state`); no other CLAUDE.md edits

## 3. Preference notes

- [x] 3.1 Idle timeout row subtitle in `crates/ui/src/preferences/network.rs` names v2ray and xray; verified live on the Network page
- [x] 3.2 DNS hijack row subtitle in `crates/ui/src/preferences/tun.rs` states `native` and `disabled` behave the same; existing `test_singbox_no_hijack_dns_rule_when_hijack_mode_native` and xray hijack tests still pass (`timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4`)
- [x] 3.3 Detour row subtitle in `crates/ui/src/preferences/dns.rs` per backend via a pure `detour_note(backend) -> Option<&'static str>`; unit test for sing-box, xray, v2ray (`None`, row hidden)
- [x] 3.4 `CHANGELOG.md` `[Unreleased]` gains one line for the preference notes

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [x] 4.2 With `XDG_STATE_HOME` unset, the path the doc gives for `backend.log` exists on a running install
