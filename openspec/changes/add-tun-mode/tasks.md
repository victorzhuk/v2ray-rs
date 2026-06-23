# Tasks: add-tun-mode

## 1. Settings model

- [x] 1.1 Create `crates/core/src/models/tun.rs` with `TunConfig` (fields: `enabled`, `interface_name`, `mtu`, `address_v4`, `address_v6: Option<String>`, `stack`, `strict_route`, `dns_hijack`, `exclude_routes: Vec<String>`) deriving `Debug, Clone, PartialEq, Serialize, Deserialize`.
- [x] 1.2 Add `TunStack` (System | Gvisor | Mixed) and `DnsHijackMode` (Hijack | Native | Disabled) enums with serde rename to backend literals; implement `Default` for each.
- [x] 1.3 Implement `Default for TunConfig` (TUN disabled; `interface_name = "tun0"`, `mtu = 1500`, `address_v4 = "172.19.0.1/30"`, `address_v6 = None`, `stack = System`, `strict_route = true`, `dns_hijack = Hijack`, `exclude_routes = []`).
- [x] 1.4 Add the `#[serde(from = "TunConfigWire")]` forward-compat wire struct mirroring `DnsConfig`/`DnsConfigWire`.
- [x] 1.5 Add `#[serde(default)] pub tun: TunConfig` to `AppSettings` in `crates/core/src/models/settings.rs` and initialize it in `AppSettings::default()`.
- [x] 1.6 Export `TunConfig`/`TunStack`/`DnsHijackMode` from `crates/core/src/models/mod.rs`.
- [x] 1.7 Add validators in `crates/core/src/models/validation.rs`: `validate_tun_interface_name` (`[a-z0-9_-]`, ≤15 chars, non-empty), `validate_cidr` (IPv4/IPv6 CIDR), MTU range 576–9000, and each `exclude_routes` entry as CIDR; add `ValidationError::InvalidTunConfig` variant(s).
- [x] 1.8 Tests: legacy `settings.toml` without `[tun]` loads with `enabled = false` + defaults; round-trip of a modified `TunConfig`; validator coverage (valid + invalid interface names, CIDRs, MTU bounds).

## 2. sing-box generator

- [x] 2.1 Refactor `build_inbounds` in `crates/core/src/config/singbox.rs` to build a `Vec<Value>` and push a `tun` inbound when `settings.tun.enabled` (interface, `address`, `mtu`, `auto_route: true`, `stack`, `strict_route`, `route_exclude_address`, `dns_mode` from `dns_hijack`, `sniff: true`).
- [x] 2.2 Set `route.auto_detect_interface: true` in `build_route` when TUN is enabled (loop prevention).
- [x] 2.3 Tests: TUN inbound shape (asserts `auto_route`, address, mtu, stack, strict_route, `auto_detect_interface`); excluded-routes mapping; no `tun` inbound when disabled.

## 3. xray / v2ray generator

- [x] 3.1 Refactor `build_inbounds` in `crates/core/src/config/v2ray.rs` to a `Vec<Value>` and push the xray `tun` inbound (`protocol: "tun"`, `settings.name/mtu/gateway/dns`, `autoOutboundsInterface: "auto"`, sniffing enabled) only when backend is xray and `settings.tun.enabled`.
- [x] 3.2 Ensure the v2ray backend never emits a tun inbound regardless of `tun.enabled`.
- [x] 3.3 Tests under the xray module: TUN inbound shape + `autoOutboundsInterface:"auto"`; under the v2ray path: no tun inbound even with `tun.enabled = true`; no tun inbound when disabled.

## 4. Route helper crate (`v2ray-rs-netctl`)

- [x] 4.1 Create `crates/netctl` (binary crate, minimal deps: `rtnetlink`, `tokio`, an arg parser) and add it as a workspace member in the root `Cargo.toml`.
- [x] 4.2 Implement `xray-up --iface <n> --addr <cidr> [--addr6 <cidr>]`: ensure link up, assign address (ignore EEXIST), add `0.0.0.0/1` + `128.0.0.0/1` (and `::/1` + `8000::/1` when `--addr6`) routes bound to the device — all idempotent.
- [x] 4.3 Implement `xray-down --iface <n>`: delete the device (no-op if absent).
- [x] 4.4 Implement `recover --singbox` / `recover --xray`: remove a leftover device and, for sing-box, flush its default rule/table indices.
- [x] 4.5 Validate all CLI arguments strictly (interface name + CIDR) before any netlink call.
- [x] 4.6 Tests gated behind a privileged/integration feature (skipped in `-short`) exercising up→down idempotency in a network namespace.

## 5. Privilege manager

- [x] 5.1 Add a privilege module (e.g. `crates/process/src/privilege.rs`) that reads file capabilities on a binary path via the `caps` crate and reports whether `CAP_NET_ADMIN` is present.
- [x] 5.2 Implement a one-time grant that runs a single `pkexec setcap …` applying `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` to the backend binary and `cap_net_admin+ep` to the `v2ray-rs-netctl` helper.
- [x] 5.3 Re-detect capabilities after grant and before each TUN start; detect a capability lost after a binary replacement.
- [x] 5.4 Detect a `nosuid`/unsupported filesystem and return a clear error with the manual `setcap` command.
- [x] 5.5 Tests: cap detection against a fixture with/without the xattr; grant command construction (no real elevation in tests).

## 6. Process lifecycle integration

- [x] 6.1 In `crates/process/src/manager.rs`, gate TUN start on `CAP_NET_ADMIN`; refuse to start in TUN mode and surface the grant requirement when missing.
- [x] 6.2 xray + TUN: after spawn, poll `/sys/class/net/<iface>` (bounded timeout) for the device, then run `netctl xray-up …`; on miss or helper failure, stop and transition to `Error`.
- [x] 6.3 sing-box + TUN: spawn unchanged (no helper); rely on `auto_route`.
- [x] 6.4 Stop: keep SIGTERM-first → SIGKILL fallback; for xray run `netctl xray-down` as a safeguard after the process exits.
- [x] 6.5 In `crates/process/src/pid.rs::check_and_kill_orphaned`, add the missing SIGKILL fallback after the SIGTERM poll window.
- [x] 6.6 Run `netctl recover --<backend>` on app start when persisted connection state shows TUN was active during an unclean shutdown.
- [x] 6.7 Tests: xray start sequencing (device-appears vs timeout→Error); sing-box start skips the helper; orphan cleanup escalates to SIGKILL.

## 7. UI

- [x] 7.1 Create `crates/ui/src/preferences/tun.rs` with `build_tun_page()` returning an `adw::PreferencesPage`, registered in `preferences/mod.rs` alongside the DNS page.
- [x] 7.2 Primary group: enable `SwitchRow`, interface-name + address `EntryRow`s (`connect_apply` + validation + CSS error like `listen_address`), MTU `SpinRow`.
- [x] 7.3 Advanced `ExpanderRow`: `stack` ComboRow, `strict_route` SwitchRow, `dns_hijack` ComboRow, excluded-routes list (AlertDialog add/edit). Grey out rows that don't apply to the active backend with a note.
- [x] 7.4 Backend/cap gating: insensitive toggle for v2ray with a note; inline "Grant TUN privileges" button when caps are missing, refreshing state on completion.
- [x] 7.5 One-shot warning toast on enabling TUN (all system traffic routed through the proxy).
- [x] 7.6 Reuse the `apply_dns_settings_mutation` clone→mutate→validate→emit pattern; update any `AppSettings` UI fixtures/snapshots for the new `tun` field.

## 8. Docs, packaging, verification

- [x] 8.1 Update `CLAUDE.md`: `AppSettings.tun` summary, new `tun.rs` model, the `crates/netctl` crate, and the privilege model.
- [x] 8.2 Update `CHANGELOG.md` with an `Added` entry for TUN mode (sing-box + xray) and the one-time setcap requirement.
- [x] 8.3 Update `pkg/archlinux/PKGBUILD` to build/install `v2ray-rs-netctl` and document the capability requirement.
- [x] 8.4 `cargo test --workspace` green; manual smoke per the change's verification notes (sing-box and xray connect → traffic via proxy → clean teardown; no-privilege path greyed with grant button).
