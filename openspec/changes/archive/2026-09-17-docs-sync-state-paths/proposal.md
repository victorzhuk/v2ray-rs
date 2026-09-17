## Why

`docs/ARCHITECTURE.md` no longer matches the code in places a user or contributor relies on when hunting for logs. It places state and runtime files under `$XDG_STATE_HOME/v2ray-rs` and `$XDG_RUNTIME_DIR/v2ray-rs` with no fallback, but when those variables are unset the code uses `data_dir/state` and `data_dir/runtime` (on the reference host the logs are in `~/.local/share/v2ray-rs/state/logs`). It says `ProbeRunner` verifies the binary before a connect, but only Real Delay uses it. It says every helper call is bounded at 10s but does not say which call each bound covers; all helper calls (`DEVICE_TIMEOUT`, `HELPER_TIMEOUT`, including startup `recover`) are 10s today. Several settings have no UI and are undocumented as file-only. Three preference rows also give no hint that a backend ignores them: the idle timeout does nothing on sing-box, a `direct` DNS detour does nothing on xray outside TUN, and DNS hijack `native` generates exactly what `disabled` does on both TUN backends.

## What Changes

- `docs/ARCHITECTURE.md`: on-disk layout and Logging section show the `data_dir/state` and `data_dir/runtime` fallbacks; `ProbeRunner` described as the Real Delay probe runner; helper timeouts listed per call (device wait, `xray-up`/`xray-down`, and startup `recover` all at 10s today via `HELPER_TIMEOUT`, re-read from the code at edit time); new "File-only settings" list: `tun.address_v6`, `auto_update_geodata`, `geodata_update_interval_secs`, `backend.config_output_dir`; per-backend applicability notes for idle timeout, DNS detour, and DNS hijack modes.
- `CLAUDE.md`: the persistence summary names `state_dir` for `latency_snapshot.json`, `tun_session.json`, and logs.
- Preferences: the idle timeout row notes it applies to v2ray and xray only; the DNS server dialog's detour row notes that on xray only `direct` has an effect and only while TUN is on; the DNS hijack row notes that `native` and `disabled` both leave DNS uncaptured.

## Capabilities

### New Capabilities

- `network-preferences-ui`: backend applicability notes on the Network preferences page.

### Modified Capabilities

- `tun-preferences-ui`: DNS hijack mode note.
- `dns-preferences-ui`: xray detour applicability note.

## Impact

- `docs/ARCHITECTURE.md`, `CLAUDE.md` — prose only.
- `crates/ui/src/preferences/network.rs`, `tun.rs`, `dns.rs` — row subtitles or note rows; no settings or generator change.
- `CHANGELOG.md` — no entry needed for docs; one `[Unreleased]` line for the preference notes.
