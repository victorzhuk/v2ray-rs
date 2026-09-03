# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- A DNS server set to a "direct" detour no longer produces a sing-box config
  that refuses to start. The backend rejects a DNS server detoured to an
  outbound that carries no settings, and validation accepts it, so the failure
  only appeared on connect. A direct detour is now expressed by omitting the
  field, which is what the backend asks for.
- sing-box carries the connect-time host pin on every path, not only when the
  DNS feature is on, and a proxy node whose hostname is pinned resolves from
  that pin at dial time. Dial-time resolution does not consult the DNS rules,
  so the pin previously sat in the config unused while the proxy's own hostname
  was resolved through the proxy being dialed.

## [0.17.3] - 2026-09-03

### Fixed
- Applying a DNS provider preset no longer aborts the app. The preset dialog
  drove the strategy row while the settings were still borrowed, and the row's
  own handler re-entered that borrow. Programmatic widget updates on the DNS
  page now run with the handlers suppressed, and the preset also syncs the
  master switch it turns on.
- xray with TUN can resolve its own proxy hostname again. With the DNS feature
  off, the derived DNS plane dropped the connect-time host pin, so the only
  resolver was reachable through the proxy it was resolving. The pin now reaches
  every xray TUN config, and a bootstrap resolver scoped to the proxy and DNS
  server hostnames leaves through the direct outbound — plain UDP first, DoH
  second, since each transport is blocked on some of the networks this runs on.
- xray honors a `direct` detour on a DNS server, and the server dialog keeps the
  choice for xray. Excluded domains bind to that server on both backends
  instead of the first proxied one.
- Kernel-side DNS capture follows the generated config: it stays off when a
  hostname node has no override xray can answer with, and the route helper is
  configured from the same effective settings the config came from.

## [0.17.2] - 2026-08-27

### Fixed
- The AppImage release job now installs `xauth`. `xvfb-run` needs it and the
  `xvfb` package does not pull it in, so the windowed smoke test aborted before
  it could start the app. v0.17.0 and v0.17.1 were tagged but never published
  for this and the `dpkg-dev` problem below; the 0.17.0 entry describes what
  first ships in 0.17.2.

## [0.17.1] - 2026-08-27

### Fixed
- The AppImage release job now installs `dpkg-dev`. `linuxdeploy-plugin-gtk`
  shells out to `dpkg-architecture` to pick the multiarch library directory and
  aborts without it, which failed the AppImage build and, with it, the release
  and AUR jobs. v0.17.0 was tagged but never published for this reason.

## [0.17.0] - 2026-08-27

### Added
- Prebuilt release artifacts and a `curl | sh` installer. The GUI ships as a
  tarball built against glibc 2.41, GTK 4.18, and libadwaita 1.7 — the floor the
  app already required — so it runs on Debian 13+, Ubuntu 25.04+, Fedora 42+,
  and Arch. The two privileged helpers are statically linked against musl and
  carry no runtime dependency at all. The installer verifies the release
  checksum, refuses to unpack onto a host whose GTK is too old, installs under
  `$HOME/.local` by default, and prints the privileged TUN setup commands rather
  than running them.
- An AppImage bundling GTK 4.18 and libadwaita, for distributions below that
  floor.
- TUN mode now works from the AppImage. An AppImage is mounted `nosuid`, so the
  route helper could never hold `cap_net_admin` where it sits; **Grant TUN
  privileges** now installs it to `/usr/local/lib/v2ray-rs/` in the same
  elevation and grants it there. The helper is streamed to the elevated process
  on stdin rather than copied by it, because the kernel gives root no exemption
  from FUSE's owner-only access rule and a copy under `pkexec` would fail on the
  AppImage's own mount. It is restricted to a dedicated `v2ray-rs` system group,
  matching the distribution package, so the grant asks you to log out and back
  in once. The trigger is the mount refusing file capabilities rather than any
  AppImage detection, so the same path covers a hardened `nosuid` `/home` or a
  USB stick. The setuid bypass wrapper is deliberately excluded from the
  AppImage: it requires a system user only a distribution package creates.
- `v2ray-rs-bin` on the AUR, installing the prebuilt tarball instead of
  compiling. It conflicts with `v2ray-rs` and shares the same install hook, so
  the privileged helper setup is identical either way.
- `make dist` builds the release tarball and its checksum locally.

### Fixed
- `v2ray-rs --help` and `--version` now print to stdout and exit 0. Both were
  routed through the startup-failure path, so help text went to stderr behind a
  "startup failed" prefix and the process exited 1.
- Translations are now installed. The `en_US` and `ru_RU` catalogs were built
  and committed but shipped by neither the Arch package nor the Debian metadata,
  so every packaged install silently fell back to untranslated strings.

### Changed
- Release builds are now optimised with thin LTO, a single codegen unit, and
  stripped symbols.
- `make release` no longer passes `-C target-cpu=native`, which produced
  binaries that could not run off the machine that built them.

## [0.16.1] - 2026-08-17

### Fixed
- Under xray TUN, the excluded routes and domains now take precedence over the port-53 hijack rule. Xray applies the first matching routing rule, and the hijack was emitted first, so it swallowed every query — including the ones aimed at an excluded resolver. That made the exclusion useless in the case it exists for: a split-horizon corporate resolver reachable only over a separate VPN, holding internal records no public resolver can answer. Names in those zones resolved to nothing while the resolver itself stayed reachable on every other port, and a corporate VPN whose endpoint hostname had to be looked up before its tunnel existed could fail to connect at all.

## [0.16.0] - 2026-08-16

### Added
- A routing rule can now name the node that carries it. Proxy rules gained a "Via node" picker, so destinations that hold connections open for a long time can use one provider while everything else keeps using the connected node — useful when a provider tears down idle streams sooner than the traffic can tolerate. A pinned node that has since been deleted or disabled falls back to the connected node instead of failing the connect.
- WebSocket ping interval setting for xray (`wsSettings.heartbeatPeriod`), off by default. Holds NATs and middleboxes open on idle ws tunnels; it operates on the transport, so it does not affect an idle timer at the far end of the proxy hop.

### Changed
- xray TUN mode now steers port-53 traffic into the tunnel's routing table, so a resolver on the local subnet stops being the one destination the tunnel never sees. Previously the policy rule that keeps LAN routes working also carried DNS, so a `nameserver` pointing at the router meant every name the host looked up was resolved outside the tunnel and the app's DNS settings applied to nothing but the proxy's own lookups. The rules exclude the proxy's marked sockets, and they resolve to the tunnel table, so if the app dies the entries stop matching anything and DNS falls back to normal instead of blackholing. Applies when TUN is on and DNS hijack is set to `hijack`.
- The TUN DNS hijack rule now covers TCP as well as UDP, so a resolver falling back to TCP on a truncated answer does not get a different view of the world.
- Under TUN, node hostnames are pinned to their resolved addresses in `dns.hosts` at connect time and outbounds dial with `sockopt.domainStrategy`, so the backend resolves its own server through its built-in resolver rather than the OS one. Left as-is, that lookup goes out on an unmarked socket — the kind the tunnel captures — so it would be sent into the tunnel it is trying to establish. Xray's documentation recommends this pinning for transparent-proxy setups. DNS capture stays off for any connect where a node could not be pinned, so an unresolvable host degrades to the previous behavior instead of deadlocking.
- Multiple `dns.hosts` entries for one domain now emit as an address array instead of the last one silently winning.
- xray DNS servers scoped to a domain list now set `skipFallback`, and an unrestricted resolver is appended when every configured server is scoped. Previously a region-scoped resolver also answered for every domain it was never meant to see.
- WebSocket outbounds emit the dedicated `wsSettings.host` field instead of `headers.Host`, which xray 26 warns about on every start and has slated for removal.

### Fixed
- Reconnecting quickly could leave a running backend with no tunnel while the UI reported Connected. Disconnect clears the connection handle before teardown has run, so a following Connect started setting up while the previous session was still tearing down — and since the route helper deletes devices by interface name and policy rules by fixed priority, with no session identity, that teardown removed the new session's rules rather than its own. Connection setup and teardown now run under one lock, and the startup route-recovery pass takes it too instead of racing an early Connect. Terminal state from a superseded connection is also ignored, so it can no longer clear the handle of the connection that replaced it.
- The TUN "DNS hijack" setting was greyed out on the xray backend even though both generators act on it.
- The WebSocket ping interval is now marked and gated as xray-only rather than offered on backends whose generators ignore it.

## [0.15.0] - 2026-08-04

### Added
- Subscriptions can now be a JSON config-bundle — the format v2RayTun, v2rayN and v2rayNG export, and the shape many providers subscribe to: an array of complete backend configs, one per node, sharing a common `routing`/`dns` block. Adding a subscription accepts a paste of this JSON directly (spilled to `data_dir/subscriptions/`, never inlined into `subscriptions.json`) in addition to a URL or file, and a `v2raytun://import/` deep link is unwrapped automatically. The provider's routing and DNS import as a profile scoped to that subscription — never merged into the app's own routing rules or DNS config — and apply automatically when a node from that subscription is active; a per-subscription switch turns it off in favor of the app's global rules. A refresh preserves that toggle instead of silently re-enabling the profile.
- `xhttp` transport support for VLESS/VMess (xray and v2ray only — sing-box has no xhttp upstream, so a node using it on that backend fails config generation with a clear error naming the node, instead of connecting silently wrong).
- Routing rules gained `Protocol` (sniffed-protocol match, e.g. block BitTorrent), `Port`, `Network`, `DomainKeyword`, and `DomainFull` matchers, closing the gap between what provider-exported routing configs commonly express and what the app could represent.

### Changed
- A `Domain` routing rule now matches a domain and its subdomains on xray/v2ray (`domain:` prefix) instead of any substring occurrence — it already matched that way on sing-box. The bare-substring case is still available, explicitly, as the new `DomainKeyword` matcher.

### Fixed
- sing-box no longer fails to start when a configured DNS server (the app's own or an imported profile's) is addressed by hostname rather than IP literal — the common case for public DoH, e.g. `https://dns.google/dns-query`. Such a server can't resolve itself; the generator now adds a local bootstrap resolver and points every hostname-addressed server at it.
- An imported or hand-written routing rule matching GeoIP `private` no longer makes sing-box try to download a rule-set file that doesn't exist (v2fly/xray bundle RFC 1918 ranges as a `private` GeoIP category; SagerNet's sing-geoip mirror ships no such file). It now generates as `ip_is_private`, sing-box's own equivalent, which needs no download at all.
- A subscription's imported profile — or a routing-rule edit referencing a GeoIP/GeoSite category the app hadn't cached yet — is now included in what gets pre-fetched before connecting. Previously only the global routing rules were scanned, so a subscription bringing new categories (a provider's own GeoSite-based rules, for instance) hit sing-box's synchronous, FATAL-on-failure startup fetch for all of them at once instead of using cached data.

## [0.14.0] - 2026-07-19

### Added
- sing-box geodata is now app-managed as per-tag binary rule-set (`.srs`) files instead of the dead `geoip.db`/`geosite.db`, which sing-box has been unable to read since 1.12. The app downloads only the GeoIP/GeoSite tags referenced by the current routing rules into `cache_dir/geodata/rule-sets/` on the existing refresh paths (startup, scheduled, and manual **Update Now**), and the generated config points each cached tag at its local file (`type: "local"`), falling back to `type: "remote"` for tags not yet fetched — so once rule-sets are primed, a cold start with GitHub blocked no longer needs the network. Stale `.db` files are deleted on the next refresh.

### Fixed
- sing-box no longer fails to start when `raw.githubusercontent.com` is blocked or throttled. Remote rule-sets dropped the forced `download_detour: "direct"` — sing-box now fetches them through the proxy outbound, its own default — and configs referencing rule-sets enable `experimental.cache_file`, so every start after the first successful fetch passes rule-set initialization offline. `store_fakeip` is set when FakeIP is on, keeping fakeip mappings valid across restarts.
- xray TUN mode no longer trusts the operating-system resolver, whose poisoned answers for blocked domains landed in `geoip:ru → direct` and got the connection reset by DPI (`proxy/tun: connection reset by peer` / `connection was refused`, dropped API streams). With TUN on, the config always carries a DNS section — derived DoH via the proxy when the DNS feature is off — xray's built-in resolver queries are tagged and routed through the proxy, and the `direct` outbound resolves via that resolver (`domainStrategy: UseIP`) instead of the OS at dial time.
- The TUN "DNS hijack" setting now works on xray: TUN-captured plaintext DNS (udp/53) is answered by xray's built-in resolver via a `dns` outbound. On sing-box it also applies with the DNS feature off, via the same derived DNS section.
- Starting TUN with an xray older than 26.1.13 (no `tun` inbound) now fails with a clear versioned message instead of an opaque config-test error. xray 26.1.13–26.6.22 additionally logs an advisory at TUN start: those releases can crash (`panic: Net: Unknown address type.`, upstream Xray-core #6364) when a connection through the tunnel closes quickly — each crash drops the tunnel until the automatic restart — and the fix ships in Xray-core 26.6.27.
- `instance.json` reports the running build's version instead of the version that first created the profile.
- Legacy `generated/` and `geodata/` directories under the data dir are actually migrated now (the relocation previously failed every start because the destination directory was never created) and credential-bearing leftovers are removed once the new location is in use.

## [0.13.1] - 2026-07-11

### Fixed
- CI `test` job no longer fails at dependency install. `sing-box` is not in the official Arch repositories, so `pacman -Syu … sing-box` aborted with `target not found`; the binary is now fetched from the SagerNet GitHub release into `PATH`, letting the `sing-box check` config test run instead of silently skipping.

## [0.13.0] - 2026-07-11

### Fixed
- Generated sing-box configs now pass `sing-box check` on 1.12/1.13 instead of being rejected on launch. The TUN inbound no longer emits the legacy `sniff`/`dns_mode` fields (removed in sing-box 1.13.0) — sniffing and DNS hijacking are now expressed as route rules. DoH servers send `path` as a string instead of an array, static host overrides use the current `predefined` field and are actually consulted via a DNS rule, and `route.default_domain_resolver` is set whenever DNS is enabled, which sing-box now requires with more than one server. A DNS server's `detour` (previously the placeholder `"proxy-0"`, which never matched a real outbound tag) now resolves to the actual first proxy outbound. FakeIP moved off the legacy top-level `dns.fakeip` block onto the fakeip server entry itself, with a DNS rule routing A/AAAA queries to it instead of setting it as the default resolver (which sing-box now rejects).
- DNS settings can no longer be saved enabled with zero servers configured, which produced an unusable config since FakeIP alone cannot answer real queries.
- "Apply & Restart" and switching the auto-resolve strategy while connected now actually reconnect. The disconnect handler cleared the pending-reconnect flag before it was read, so both flows silently stayed disconnected until a manual Connect.
- When a crashing node exhausts its restart budget mid-session, the connection now fails over to the next candidate in the resolved list instead of giving up; the summarized error only surfaces after every candidate has failed.
- A connection error now carries the backend's own last log line (e.g. the actual xray startup failure) instead of just "process exited with code N", and log readers drain the pipes on exit so that line isn't lost.
- The generated config is validated with the backend's own checker (`sing-box check`, `xray run -test`, `v2ray test`) before spawning, so a rejected config fails fast with the real reason instead of burning the crash-restart budget on instant exits.
- Clicking Disconnect while a connection is still starting no longer flips the UI back to "Connected" against a dead session; the stop is honored as soon as the start attempt settles.
- App startup no longer freezes the window for up to ~6.5s while reaping an orphaned backend and recovering leftover TUN routes; both now run in the background.
- A corrupt TUN session marker is logged and discarded instead of silently disabling the route-recovery pass on every subsequent launch.
- Subscription fetches no longer retry permanent failures. A 404, 401, malformed URL, or bad request now fails immediately instead of wasting up to 7s on exponential backoff; transient errors (HTTP 408/429/5xx, connection/timeout) keep the existing retry envelope.

### Added
- Idle connection timeout setting (Network → Proxy Ports, default 600s). Generated xray/v2ray configs now set `policy.levels.0.connIdle` accordingly; previously every config ran with the backend's stock 300s, which silently killed long-idle streams such as SSE/streaming API connections.
- Subscriptions and nodes can now be edited while connected — toggles, reordering, and adding no longer require disconnecting first. Changes are persisted immediately and the existing "Configuration changed" banner offers Apply & Restart; the running session is untouched until then.
- The currently connected node is marked with a "Connected" tag in the subscription and manual node lists.

### Changed
- Changing the auto-resolve strategy or the "Use Real Delay for Lowest Latency" toggle while connected now raises the existing "Configuration changed" banner instead of reconnecting immediately; the new strategy is applied only after clicking **Apply & Restart**. While disconnected, the change is still applied silently.

---

## [0.12.0] - 2026-07-04

### Fixed
- Unstable connections now recover on their own. A backend killed by a signal (out-of-memory, a crash, or an external kill — which reports no exit code) was previously mistaken for a clean stop and left disconnected; it is now treated as a crash and restarted. The in-place restart, which never actually worked because of an invalid state transition, now relaunches without a visible disconnect, with a backoff that grows with recent crashes.
- A crash or give-up is no longer lost behind a burst of backend log output. Log lines and connection-state changes now travel on separate channels, so a busy log can't drop the event that tells the UI the connection died (which left it showing "connected" against a dead backend).
- After the backend exhausts its restart budget, the app retries the whole connection a bounded number of times, re-selecting a candidate, instead of staying disconnected.
- TUN routing state is cleaned up on every path that ends a tunnel — a crash, a failed start, and a give-up — not just a clean disconnect, so host-wide policy rules no longer outlive the device. The recovery marker now survives a crash so leftover state is cleared on the next launch.
- The app no longer freezes while the polkit TUN-privilege dialog is open, nor for up to ~1.5s while reaping a leftover backend on Connect; both now run off the UI thread.

### Security
- `v2ray-rs-netctl` refuses to operate on any interface that is not a TUN device, so a local process can no longer point it at `lo` or a physical NIC to black-hole system traffic, or delete another tunnel's link (`wg0`, `docker0`, ...).
- The one-time privilege grant resolves the route helper and setuid wrapper to absolute paths beside the executable, so a bare name can no longer be resolved against the process working directory by the elevated `setcap`/`chown`/`chmod`.
- The privileged helpers are no longer world-executable. Packaging now creates a `v2ray-rs` group and restricts `v2ray-rs-netctl` (`0750`) and `v2ray-rs-run` (`4750`) to it, so only group members can invoke them.

### Changed
- **TUN mode now requires membership in the `v2ray-rs` group.** After installing or upgrading, run `sudo usermod -aG v2ray-rs "$USER"` and re-login; until then TUN connect fails with a permission error. (Non-TUN proxying is unaffected.)

---

## [0.11.0] - 2026-06-25

### Added
- Per-process TUN bypass for xray: a dedicated `v2ray-rs-bypass` system user, a setuid-root `v2ray-rs-run` wrapper, and a `uidrange` policy rule (priority 8998) programmed by `v2ray-rs-netctl xray-up --bypass-uid`. App-launched tools can exit the tunnel via the *Run with bypass* action in TUN preferences; sing-box already excludes processes natively. Packaging creates the user and installs the wrapper setuid on Arch.

---

## [0.10.2] - 2026-06-23

### Fixed
- Choosing a custom backend path (or finishing the first-run wizard) now verifies the binary's reported identity against the selected backend type instead of trusting the dropdown. This stops, for example, the xray binary from being saved under a "sing-box" selection — a mismatch that fed the wrong-schema config to the backend and crashed it with a nil-pointer SIGSEGV on connect.
- xray TUN mode no longer floods the logs with a `[tun-in -> direct]` connection storm. xray's own outbound sockets are marked (`sockopt.mark`) and the route helper installs policy rules that send marked traffic past the tunnel, so `direct`-routed connections egress the real interface instead of looping back into the TUN device. LAN and link routes are preserved, and the rules are torn down on disconnect and recovery.

---

## [0.10.1] - 2026-06-23

### Fixed
- TUN mode no longer reports "backend lacks CAP_NET_ADMIN" when the backend binary already holds the capability. `getcap` output is trimmed before parsing, so its trailing newline can no longer blank out the capability set and cause every candidate to be rejected. The same fix corrects the Preferences → TUN capability-status indicator.

---

## [0.10.0] - 2026-06-23

### Added
- TUN mode for sing-box and xray: a virtual interface becomes the default route, transparently proxying all system traffic. sing-box self-routes via `auto_route`; xray uses a minimal privileged route helper (`v2ray-rs-netctl`) for the address and split routes. v2ray is excluded (no native TUN).
- `[tun]` settings section (`enabled`, `interface_name`, `mtu`, `address_v4`, `address_v6`, `stack`, `strict_route`, `dns_hijack`, `exclude_routes`, `exclude_processes`, `exclude_domains`) with a TUN preferences page, backend/capability gating, and a system-wide-routing warning.
- One-time TUN privilege grant: `setcap` via `pkexec` grants `cap_net_admin` to the backend binary and the route helper; capabilities are re-checked before each TUN start and re-prompted if lost (e.g. after a package upgrade).
- Route recovery after an unclean shutdown removes a leftover TUN device and flushes stale routes; orphan cleanup now escalates SIGTERM to SIGKILL.

---

## [0.9.0] - 2026-05-31

### Added
- Real Delay latency probes for sing-box with Clash API and xray/v2ray with native ObservatoryService gRPC.
- Per-node Real Delay persistence, badges, sorting, and optional Lowest Latency strategy integration.
- Real Delay settings for test URL, timeout, enable/disable, and Lowest Latency usage.
- Isolated probe backend runners and loopback-only probe configs for sing-box, xray, and v2ray.

### Fixed
- Real Delay availability now refreshes when the backend type or binary path changes.
- Hung or stale Real Delay probes no longer keep the subscriptions UI stuck in testing state.
- Real Delay preferences are wired to `AppSettings.real_delay` and persist immediately.

---

## [0.8.1] - 2026-05-22

### Fixed
- CI: clippy `field_reassign_with_default` errors in `AppSettings` listen-address tests (use struct-init syntax instead of `Default::default()` + reassignment).
- CI: `rustfmt` violations in `crates/core/src/config/v2ray.rs` and `crates/ui/src/preferences/network.rs`.

---

## [0.8.0] - 2026-05-22

### Added
- `listen_address` setting (default `127.0.0.1`) controlling the inbound bind address for the SOCKS5/mixed and HTTP proxies; surfaced as a Settings → Network entry with validation and a one-shot warning toast when set to a non-loopback address.
- Per-backend regression tests asserting UDP on the SOCKS-capable inbound (v2ray/xray: `settings.udp = true`; sing-box: `mixed` type with no `udp_disabled: true`).

### Changed
- Inbound listen address is now configurable (default `127.0.0.1`) instead of hard-coded; applies to both SOCKS and HTTP inbounds for v2ray, xray, and sing-box.
- `ConfigWriter` defensively validates `listen_address` before generation and falls back to `127.0.0.1` (with a `log::warn`) for malformed values, so backends never start with a broken bind string.

---

## [0.7.4] - 2026-04-28

### Fixed
- Flaky test: add process initialization delay in orphan detection test
- AUR publish: update deploy action to v4.1.3 (fixes `bash: --command` error)

---

## [0.7.3] - 2026-04-28

### Fixed
- CI workflow YAML indentation error preventing all jobs from running

---

## [0.7.2] - 2026-04-28

### Fixed
- Clippy lint violations: collapsible `if let` chains, `&PathBuf` → `&Path`, missing `.truncate(true)` on file create
- Code formatting across workspace

### Changed
- CI workflow: corrected YAML indentation for `fmt` job, changed format check from `cargo check` to `cargo fmt --check --all`

---

## [0.7.1] - 2026-04-28

### Fixed
- CI build failure: add `protobuf` package to Arch Linux container dependencies for `prost_build` compilation

---

## [0.7.0] - 2026-04-28

### Added
- Runtime profiles (`--profile`, `V2RAY_RS_PROFILE` env) with isolated storage for production, development, test, and custom profiles
- Per-directory path overrides via CLI flags (`--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`) and matching env vars
- Instance compatibility stamp (`state_dir/instance.json`) — refuses to start when profile, App ID, or schema version mismatch the running build
- Single-instance lock per profile (`flock` on `runtime_dir/v2ray-rs.lock`) — second instance of same profile exits with code 75
- `--reset-instance` flag to wipe a profile's data (production requires `--i-understand`)
- `--install-icons` flag for non-production profiles

### Changed
- PID file moved from `data_dir/backend.pid` to `runtime_dir/backend.pid`
- Generated backend configs moved from `data_dir/generated/` to `runtime_dir/generated/`
- Geodata files moved from `data_dir/geodata/` to `cache_dir/geodata/`
- Latency snapshot moved from `data_dir/latency_snapshot.json` to `state_dir/latency_snapshot.json`
- Legacy files are automatically relocated on first launch (best-effort, logged)
- `V2RAY_RS_DEV` env var is deprecated; use `V2RAY_RS_PROFILE=development` instead
- `AppPaths::from_paths()` deprecated; use `AppPaths::for_profile_in(profile, root)` in tests
- `AppPaths` now exposes `cache_dir()`, `runtime_dir()`, `state_dir()` alongside existing `config_dir()` and `data_dir()`
- Tray icon installation skipped for non-production profiles unless `--install-icons` is set

---

## [0.6.2] - 2026-02-27

### Fixed
- LICENSE file restored to canonical Apache 2.0 text for correct GitHub license detection

---

## [0.6.1] - 2026-02-27

### Changed
- License changed from MIT to Apache 2.0

### Documentation
- CLAUDE.md: documented `models/dns.rs`, `models/resolve.rs`, `config/` module, `AppPaths::new_dev()`, corrected `app.rs` layout description (Paned + Preferences dialog), updated preset list and field descriptions
- OpenSpec specs actualized against implementation: routing-rules, process-lifecycle, dns-configuration, dns-provider-presets, subscription-update, main-window, system-tray
- README.md: updated license badge
- CONTRIBUTING.md: corrected branch name (`master`), project structure (all 5 crates), license reference

---

## [0.6.0] - 2026-02-26

### Changed
- `DnsProtocol` now exposes `default_port()` — eliminates duplicate port table across `singbox.rs`, `preferences.rs`, and `dns.rs`
- `builtin_dns_presets()` refactored via `standard_preset()` helper, reducing ~170 lines to ~30
- `apply_dns_preset()` now drops DNS rules whose `server_tag` no longer exists after the preset replaces servers

### Fixed
- Dead variables `_is_singbox` and `_servers` removed from `preferences.rs`
- DNS providers dialog now closes automatically after a preset is successfully applied
- `DnsValidationError` re-exported from `models` public API

### Tests
- `test_builtin_dns_presets_count` strengthened to `assert_eq!(8)` from `>= 8`
- Added `test_apply_dns_preset_clears_orphaned_rules` to verify preset application passes `validate()`

---

## [0.5.3] - 2026-02-22

### Fixed
- CI formatting: stray blank line in `resolve.rs` after `Default` impl removal

---

## [0.5.2] - 2026-02-22

### Fixed
- Clippy `collapsible_if` in `xray.rs`: collapse nested `if let` into a single `&&` chain
- Clippy `derivable_impls` in `resolve.rs`: replace manual `Default` impl with `#[derive(Default)]`
- Clippy `clone_on_ref_ptr` in `app.rs`: replace `&[node.clone()]` with `std::slice::from_ref`

---

## [0.5.1] - 2026-02-22

### Fixed
- CI formatting check failure: apply `cargo fmt` to all crates

---

## [0.5.0] - 2026-02-22

### Added
- DNS configuration model (`DnsConfig`, `DnsServer`, `DnsProtocol`) with DoH and plain DNS support
- DNS config generation for all backends (v2ray, xray, sing-box) — enabled via Settings when `dns.enabled = true`
- Routing rule groups: rules carry an optional `group` name (set automatically when applying a preset)
- Group-based routing rules UI: rules grouped by preset name in Preferences → Routing, with per-group "Remove" button
- `geodata_dir` forwarded to `ProcessManager` and exported as `V2RAY_LOCATION_ASSET` / `XRAY_LOCATION_ASSET` env vars on process spawn
- Log viewer buffer cap: trims to 10,000 lines to prevent memory growth on long-running sessions

### Changed
- "RU Direct" preset renamed to "RU Bypass" and expanded with private CIDRs and Russian GeoSite categories (media, retail, gov, mail, entertainment, e-commerce, etc.)
- "Popular AI", "Social Networks", and "Bypass LAN" presets merged into single "Proxy Popular" preset
- Country code validation now accepts extended GeoIP tags (GOOGLE, FACEBOOK, NETFLIX, TELEGRAM, TWITTER, etc.) in addition to ISO 3166-1 codes
- GeoSite category validation no longer requires membership in a hardcoded allowlist — accepts any valid lowercase hyphenated string
- Subscription operations (toggle, rename, move, delete, add) now use atomic whole-file saves via `save_subscriptions` instead of granular per-item persistence
- Xray: removed `security: "xtls"` override — xray-core v1.8+ uses `security: "tls"` with the `flow` field for XTLS vision

---

## [0.4.0] - 2026-02-16

### Added
- Connection auto-resolve: automatically select proxy nodes using configurable strategies (list order, lowest latency, random, last successful, geo-aware)
- Connection status bar in main window showing active node, latency, backend, and strategy
- Auto-resolve strategy selector in preferences (Network → Connection)
- Tray tooltip showing connection details (node, latency, backend, strategy, uptime)
- Tray menu label showing active node name when connected
- Latency snapshot persistence (`latency_snapshot.json`) — ping results survive restarts
- Connection state persistence (`connection_state.json`) — tracks active connection metadata
- `ConnectionPlanner` for ordered candidate resolution with fallback through all enabled nodes
- Last-successful-node tracking for reconnection preference

### Changed
- Connect flow iterates through ranked candidates instead of sending all nodes at once
- Process state events now carry `ConnectionMetadata` through the broadcast channel
- Reconnect on settings change only triggers when auto-resolve strategy changes (not on every save)

---

## [0.3.11] - 2026-02-15

### Fixed
- System tray icons missing on some status notifier hosts: install symbolic icons into the user hicolor theme and update icon cache
- App icon missing in some compositors: set window icon name and install icon cache on startup after registering resources

---

## [0.3.7] - 2026-02-12

### Fixed
- App icon not displayed: resized PNG to match 256x256 icon theme directory
- App icon not displayed: update icon cache after runtime install
- App icon not associated with window: added `StartupWMClass` to desktop entry
- Port settings changes had no effect until manual reconnect: auto-reconnect on settings change

---

## [0.3.6] - 2026-02-12

### Fixed
- AUR build failure: disable LTO to fix `ring` assembly linking (`ring_core_0_17_14__*` undefined symbols)

---

## [0.3.5] - 2026-02-12

### Fixed
- Runtime panic "No provider set": install ring crypto provider before reqwest client creation

---

## [0.3.4] - 2026-02-12

### Fixed
- AUR build failure: replaced `aws-lc-rs` crypto backend with `ring` (fixes linker errors in sandboxed builds)

---

## [0.3.3] - 2026-02-12

### Fixed
- CI test failure: ETXTBSY race on overlayfs (retry spawn on `ExecutableFileBusy`)
- AUR publish failure: `github-actions-deploy-aur@v4` tag doesn't exist (pinned to `v4.1.1`)

---

## [0.3.2] - 2026-02-12

### Fixed
- CI test failure: ETXTBSY race in `crash_detection` test (sync script file before exec)
- AUR publish not triggering after release (GITHUB_TOKEN events don't trigger other workflows)

### Changed
- AUR publish workflow now called directly from release via `workflow_call`

---

## [0.3.1] - 2026-02-12

### Fixed
- CI build failure: added `base-devel` to Arch container packages (fixes `glib-sys` compilation)
- Release build failure: committed `Cargo.lock` for reproducible `--locked` builds

---

## [0.3.0] - 2026-02-12

### Changed
- CI/Release builds inside Arch Linux container (rolling glib >= 2.84 for GNOME 48)
- Updated GitHub Actions: actions/checkout v4 -> v6
- Bumped toml 0.8 -> 1, nix 0.29 -> 0.31, resvg 0.45 -> 0.47, reqwest 0.12 -> 0.13
- Added --locked flag to release build
- Removed .deb packaging from release workflow

### Added
- AUR publishing workflow (auto-publishes PKGBUILD on GitHub release)

### Fixed
- CI failure from cargo fmt violations
- Release build failure (glib 2.84 not available on Ubuntu 24.04)
- All clippy warnings (derivable_impls, collapsible_if, needless_borrows, manual_ok)

---

## [0.2.0] - 2026-02-12

### Added
- GTK4/Relm4 GUI application with libadwaita (`v2ray-rs-ui`)
  - Main window with ViewSwitcher: Subscriptions, Routing, Logs, Settings
  - Connect/Disconnect button with process state tracking
  - Subscription management: add, rename, delete, toggle, reorder
  - Node latency testing (TCP ping) with sort-by-latency
  - Routing rule editor with drag-and-drop reordering and preset dialogs
  - Settings editor for backend, ports, update intervals, language
  - Process log viewer
  - First-run onboarding wizard
  - Toast notifications for status messages
  - i18n support via gettext
- System tray integration via ksni (`v2ray-rs-tray`)
  - FreeDesktop symbolic icons (connected/disconnected/error)
  - Connect/Disconnect toggle, status label, Open Main Window, Quit
  - Desktop notifications on state changes
- Workspace expanded to five crates: core, subscription, process, tray, ui
- Arch Linux PKGBUILD packaging
- Desktop entry file (`v2ray-rs.desktop`)
- Shared config test fixtures (`config/test_fixtures.rs`)
- Shared `outbound_tag()` helper (`config/common.rs`)
- Bounded concurrent TCP pings via `Semaphore` (max 50)
- PID reuse protection via `/proc/PID/cmdline` verification
- Signal-aware exit classification (SIGINT/SIGTERM/SIGKILL vs real crashes)
- Tracked log capture tasks with abort-on-stop

### Changed
- `geodata::needs_update()` accepts `Duration` instead of raw `u64`
- `subscription::update::reconcile_nodes()` delegates to `reconcile_with_counts()`
- Extracted `parse_url_transport()` and `parse_url_tls()` from duplicated parser code
- HTTP user-agent now uses `CARGO_PKG_VERSION` instead of hardcoded `"v2ray-rs/0.1"`
- Replaced 30+ `let _ = persist()` sites with `if let Err(e)` + `log::error!`
- Replaced `Mutex::lock().unwrap()` with `if let Ok(guard)` in UI statics
- Added `#[serde(skip_serializing_if = "Option::is_none")]` to all `Option<T>` proxy fields
- Changed `SubscriptionNode::last_latency_ms` from `#[serde(skip)]` to `#[serde(skip_serializing, default)]`
- Extracted named constants for timeouts, window size, channel capacity, retry limits, and ruleset URLs
- sing-box config uses `rule_set` (remote binary `.srs`) instead of deprecated `geoip`/`geosite` databases

### Fixed
- Panic on `target.parent().unwrap()` in geodata download path
- Potential UTF-8 panic in tray error message truncation
- PID file `kill(pid, 0)` race condition on PID reuse
- Untracked `tokio::spawn` tasks for log capture leaked on process stop
- All signal exits (130/137/143) incorrectly counted toward crash threshold

### Removed
- Dead `status_bar.rs` module (127 lines, unused)
- Duplicate `outbound_tag()` from `v2ray.rs` and `singbox.rs`
- Duplicate test fixture functions from individual config test modules
- Legacy PNG tray icons (replaced by symbolic SVGs)

## [0.1.0] - 2026-02-11

### Added
- Initial release
- Core architecture with Clean Layered Design
- Workspace with three crates:
  - `v2ray-rs-core`: Domain models, persistence, backend detection, routing, config generation
  - `v2ray-rs-subscription`: Subscription fetching and URI parsing
  - `v2ray-rs-process`: Tokio-based process lifecycle management
- Initial project structure and workspace configuration
- Core domain models for proxy protocols (VLESS, VMess, Shadowsocks, Trojan)
- Subscription management with URL and file sources
- Routing rule engine with GeoIP, GeoSite, domain pattern, and IP CIDR matching
- XDG-compliant file persistence with atomic writes
- Backend detection for v2ray, xray, and sing-box
- Geodata manager for GeoIP/GeoSite database updates
- Backend config generation for v2ray, xray, and sing-box
- Async process lifecycle management with crash recovery
- Circular log buffer for process output capture
- Graceful shutdown with SIGTERM/SIGKILL handling
- Built-in routing presets (RU Direct, CN Direct, Block Ads)
- Validation framework for country codes, IP CIDRs, and domain patterns
- Comprehensive test coverage with isolated environments

### Features

**Core (`v2ray-rs-core`)**
- Domain models: `ProxyNode`, `Subscription`, `RoutingRuleSet`, `AppSettings`
- Transport and TLS settings for proxy nodes
- Rule matching: GeoIP, GeoSite, domain pattern, IP CIDR
- Rule actions: Proxy, Direct, Block
- XDG-compliant persistence (settings.toml, subscriptions.json, routing.json)
- Atomic file writes via `tempfile::NamedTempFile`
- Backend binary detection (v2ray, xray, sing-box)
- Backend version extraction and validation
- Geodata management with GitHub release fetching
- Config generation for all three backends
- Routing manager with CRUD operations and rule validation
- Built-in routing presets

**Subscription (`v2ray-rs-subscription`)**
- HTTP subscription fetching with rustls-tls
- Local file subscription reading
- Base64-encoded content decoding
- Multi-line subscription parsing
- URI parsers for VLESS, VMess, Shadowsocks, Trojan

**Process (`v2ray-rs-process`)**
- Async process spawning via `tokio::process::Command`
- Process state machine (Stopped, Starting, Running, Stopping, Error)
- State change events via tokio broadcast channel
- Circular log buffer (10K lines)
- Async stdout/stderr capture and buffering
- PID file lifecycle management
- Orphaned process detection and cleanup
- Graceful stop with configurable timeout
- Crash recovery with backoff (max 3 crashes/minute)

**Testing**
- Comprehensive test coverage
- Isolated temp directory testing for persistence
- Table-driven tests for validation
- Process lifecycle tests
- URI parsing tests

**Infrastructure**
- Rust Edition 2024
- Workspace dependencies management
- Makefile for build automation
- GitHub Actions CI configuration
- CLAUDE.md development guidelines
[Unreleased]: https://github.com/victorzhuk/v2ray-rs/compare/v0.17.3...HEAD
[0.17.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.17.2...v0.17.3
[0.17.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.17.1...v0.17.2
[0.17.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.16.1...v0.17.0
[0.16.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.13.1...v0.14.0
[0.13.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.10.2...v0.11.0
[0.10.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.10.1...v0.10.2
[0.10.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.8.1...v0.9.0
[0.8.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.4...v0.8.0
[0.7.4]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.2...v0.7.0
[0.6.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.11...v0.4.0
[0.3.11]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.7...v0.3.11
[0.3.7]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/victorzhuk/v2ray-rs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/victorzhuk/v2ray-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/victorzhuk/v2ray-rs/releases/tag/v0.1.0
