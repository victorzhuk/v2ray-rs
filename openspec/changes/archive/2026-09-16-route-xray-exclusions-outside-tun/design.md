## Context

- Generator: under xray TUN, `build_routing` pushes `{"ip": exclude_routes, "outboundTag": "direct"}` after the `dns-internal` rule (`crates/core/src/config/v2ray.rs:501-506`). The live config has `{"ip":["10.15.12.100/32","10.15.12.200/32"],"outboundTag":"direct"}`. `direct` carries `sockopt.mark: 255`, so its re-emitted packets take pref 9000 to `main`.
- sing-box: `build_tun_inbound` sets `route_exclude_address` from `exclude_routes` (`crates/core/src/config/singbox.rs:197-199`), test `test_singbox_tun_excluded_routes_mapped` (`:1103`).
- Helper rule layout (`crates/netctl/src/net.rs`): 8998 bypass UID → `main` (`:30`), 8999 unmarked udp/tcp port 53 → table 2023 (`:37`), 9000 fwmark 255 → `main`, 9001 unmarked → `main` with `suppress_prefixlength 0`, 9002 → table 2023 (`:42-44`). `add_rule` treats EEXIST as success (`:321-350`). `del_xray_rules` deletes by reserved priority list in `is_xray_rule` (`:386-413`); `recover_xray` calls `xray_down` (`:170-174`). IPv6 host probe `host_has_ipv6` (`:137-139`).
- CLI (`crates/netctl/src/main.rs`): `XrayUp { iface, addr, addr6, bypass_uid, capture_dns, strict }`; values validated by `validate::parse_cidr` (`crates/netctl/src/validate.rs:29-44`), which accepts host bits and prefix 0.
- Process: `TunRuntime` (`crates/process/src/tun.rs:31-45`), `xray_up_args` (`:222-243`); runtime built in `build_tun_runtime` from effective settings (`crates/ui/src/connection.rs:367-396`).
- A destination with a more specific route in `main` (e.g. a LAN or VPN prefix) already escapes the tunnel at pref 9001; only destinations whose best `main` route is the default reach pref 9002. The exclusion rule matters for exactly those, plus port-53 traffic captured at 8999 regardless of route.
- Privileged namespace tests live in `crates/netctl/tests/privileged.rs` behind the `privileged-tests` feature (e.g. `capture_dns_steers_port_53_into_the_tunnel_table`, `reup_across_recreated_device_leaves_one_copy`).

## Goals / Non-Goals

**Goals:**
- An xray TUN excluded CIDR never enters the TUN device, including port 53.

**Non-Goals:**
- Kernel-level exclusion for excluded domains; the kernel sees no names.
- Changing sing-box exclusions.
- Removing the generator's `ip → direct` rule: it still covers traffic reaching xray through the socks/http inbounds while TUN is on, and packets in flight during a helper re-run.

## Decisions

- **Pref 8997, lookup `main` without suppression.** Lower than 8998/8999 so an excluded resolver is not captured, and `main`'s default route is the point of excluding. Rule: family by CIDR, destination prefix, action to table `main`. Alternative rejected: pref between 8999 and 9000 — DNS capture would still pull excluded resolvers in. Alternative rejected: routes in table 2023 pointing at the real gateway — needs gateway discovery and breaks on network change.
- **Replace the set on `xray-up`.** Delete every pref-8997 rule for both families, then add the current list. `xray-up` runs on every (re)spawn, so a changed list takes effect on reconnect without a separate verb. The gap between delete and add is covered by the generator's `ip → direct` rule. Alternative rejected: diffing existing rules — more netlink parsing for no observable gain.
- **Validation in the helper.** Reuse `parse_cidr`, then refuse prefix length 0 (it would send all traffic around the tunnel), clear host bits, and cap the count at 256 values. Rejected before any netlink call, error names the value. The helper is `setcap cap_net_admin` and callable by any local process, so it must not trust the app's validation.
- **IPv6 exclusions on IPv6-less hosts are skipped**, not an error, mirroring `v6_rules_needed`; an IPv6 rule add would fail with an unsupported address family.
- **Plumbing.** `TunRuntime` gains `exclude_routes: Vec<String>`; `xray_up_args` emits `--exclude <cidr>` per entry; `build_tun_runtime` copies `settings.tun.exclude_routes` for xray (empty for sing-box).
- **App validation matches the helper.** `TunConfig::validate` and the TUN preferences refuse prefix length 0 and more than 256 excluded routes, so a value the helper would refuse never reaches connect.
- **UI wording only.** No new controls; group descriptions switch on the active backend.

## Risks / Trade-offs

- [A local process calls the helper with `--exclude` to steer traffic around the tunnel] → it could already call `xray-down`; exclusions only move traffic to `main`, never into another table, and prefix 0 is refused.
- [Excluded prefixes stay reachable while strict route blocks everything else after a crash] → intended: excluded means outside the tunnel's guarantees; the strict requirement names only traffic destined for the tunnel.
- [Many exclusions slow `xray-up`] → bounded at 256; the helper timeout already bounds the call.

## Open Questions

- Private destinations sometimes go to the proxy (`accepted tcp:192.168.10.20:1 [tun-in >> proxy]` in `backend.log`). xray sniffing with the default `routeOnly: false` replaces the destination IP with the sniffed domain (fact recorded in archived `harden-tun-dns-resolution`), so an `ip` rule may no longer see `192.168.10.20`. Whether that explains these lines, and whether `routeOnly: true` is the fix, is checked by task 4.3; if confirmed it becomes its own change. Kernel-level exclusion of such prefixes through this change avoids the question for listed CIDRs.

## Implementation plan

Tier `heavy`, mode `existing-service-strict`, lenses `spec`, `quality`, `sec` (`sec`: the route helper is setcap cap_net_admin and callable by any local process). Planned at `d9ded0a7`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.

Scope decisions taken while planning: the sing-box excluded-domains description gains the next-connect clause ("Domain suffixes that bypass the tunnel; applies on next connect"); app-side validation refuses prefix-0 excluded routes and lists over 256 (task 2.3), matching the route helper so no persisted value fails only at connect.

### `x1` — tasks 1.1 — seam `S1-netctl-validate`

- order: parallel, shard `netctl`; coder `rust-coder`; packages `v2ray-rs-netctl`
- sites:
  - `crates/netctl/src/validate.rs` · `parse_cidr (new sibling exclusion parser)` · anchor `pub fn parse_cidr(cidr: &str) -> Result<(IpAddr, u8), String> {` — new fn after parse_cidr: parse each value via parse_cidr, refuse prefix 0, mask host bits (v4/v6), refuse count > 256; errors Result<_, String> naming the value
  - `crates/netctl/src/validate.rs` · `tests` · anchor `fn cidr_rejects_bad() {` — new tests after cidr_rejects_bad: 10.15.12.100/32 ok, 10.1.2.3/8 -> 10.0.0.0/8, fd00::1/64 -> fd00::/64, 0.0.0.0/0 and ::/0 refused, 1.2.3.4/33 refused, 257 values refused
- work, in order:
  - 1.1 tests first in validate.rs mod tests (names above; v4+v6 normalization, both /0 refused, /33 refused, check_exclusion_count(256) Ok and (257) Err with text containing "257")
  - 1.1 impl: parse_exclusion = parse_cidr -> refuse prefix 0 -> mask host bits (v4 via u32, v6 via u128); check_exclusion_count
  - MAX_EXCLUSIONS gets a comment that the app enforces the same cap (MAX_EXCLUDE_ROUTES in crates/core/src/models/tun.rs)
- names: `pub const MAX_EXCLUSIONS: usize = 256;`; `pub fn parse_exclusion(cidr: &str) -> Result<(IpAddr, u8), String>`; `pub fn check_exclusion_count(count: usize) -> Result<(), String>`; `prefix-0 error: format!("exclusion prefix length 0 refused: {cidr:?}")`; `count error: format!("too many exclusions: {count} (max {MAX_EXCLUSIONS})")`; `reused parse_cidr errors: "invalid cidr (missing prefix): {cidr:?}", "invalid cidr address: {cidr:?}", "invalid cidr prefix: {cidr:?}", "cidr prefix out of range: {cidr:?}"`; `tests: exclusion_accepts_host_route, exclusion_clears_host_bits, exclusion_refuses_default_prefix, exclusion_refuses_out_of_range_prefix, exclusion_count_capped_at_256`
- verify: `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4`

### `x2` — tasks 1.3, 1.4 — seam `S3-netctl-rules`

- order: after `x1`, shared `crates/netctl/src`, shard `netctl`; coder `rust-coder`; packages `v2ray-rs-netctl`
- sites:
  - `crates/netctl/src/net.rs` · `RULE_PREF_EXCLUDE` · anchor `const RULE_PREF_BYPASS_UID: u32 = 8998;` — new const RULE_PREF_EXCLUDE: u32 = 8997 near it
  - `crates/netctl/src/net.rs` · `xray_up` · anchor `pub async fn xray_up(` — signature unchanged (already 7 params; an 8th trips clippy::too_many_arguments). Exclusion handling lives in new pub async fn replace_exclusions(handle, &[(IpAddr, u8)]) next to it: delete every pref-8997 rule for both families, then add one destination-prefix rule to table main per installable entry, in order; EEXIST tolerated
  - `crates/netctl/src/net.rs` · `add_rule / new add_exclude_rule` · anchor `async fn add_rule(` — add_rule has no destination attr; new add_exclusion_rule pushing RuleAttribute::Destination + header.dst_len, family by address, table main; error format!("add exclusion rule {ip}/{prefix} (pref {RULE_PREF_EXCLUDE}): {e}")
  - `crates/netctl/src/net.rs` · `del_xray_rules (pattern for pref-8997 delete)` · anchor `async fn del_xray_rules(handle: &Handle) {` — new sibling deleting only pref-8997 rules for IpVersion::V4/V6 (same get/try_next/del loop)
  - `crates/netctl/src/net.rs` · `is_xray_rule` · anchor `RULE_PREF_BYPASS_UID,` — factor the reserved list into const XRAY_RULE_PREFS: [u32; 6] including RULE_PREF_EXCLUDE; is_xray_rule matches against it; pure test xray_rule_prefs_cover_exclusions asserts XRAY_RULE_PREFS contains RULE_PREF_EXCLUDE (8997)
  - `crates/netctl/src/net.rs` · `v6_rules_needed (pattern) / tests` · anchor `fn v6_rules_follow_address_strict_and_host_support() {` — new pure fn for v6 exclusion skip decision + table test next to this one; extend `use super::{XRAY_FWMARK, v6_rules_needed};`
  - `crates/netctl/tests/privileged.rs` · `NS consts` · anchor `const NS_GONE: &str = "nctl-gone-ns";` — new namespace const for exclusion test
  - `crates/netctl/tests/privileged.rs` · `new test` · anchor `fn strict_state_refuses_unmarked_traffic_without_device() {` — new #[test] (MAIN_IFACE uplink setup as in this test): up with two --exclude -> two 8997: rules, route get excluded addr via MAIN_IFACE; --capture-dns + excluded :53 (ip route get ... ipproto udp dport 53) via main; re-up one exclusion -> one rule; xray-down and recover --xray -> no 8997:
  - `crates/netctl/tests/privileged.rs` · `assert_xray_state_cleared` · anchor `for pref in ["8998:", "8999:", "9000:", "9001:", "9002:"] {` — optionally add "8997:" to leaked-pref list
- work, in order:
  - 1.3 unit tests first in net.rs mod tests: exclusion_installable_skips_v6_without_host_ipv6 (table over v4/v6 x host true/false); xray_rule_prefs_cover_exclusions (XRAY_RULE_PREFS contains RULE_PREF_EXCLUDE)
  - 1.3 impl: RULE_PREF_EXCLUDE = 8997, XRAY_RULE_PREFS const used by is_xray_rule, exclusion_installable, del_rules_with_priority, add_exclusion_rule, replace_exclusions (delete all 8997 both families, then add each installable entry in order; only ever table main, dst_len ≥ 1); update RULE_PREF docs; xray_up signature unchanged
  - 1.4 privileged tests (compile under clippy --all-features, file gated by #![cfg(feature = "privileged-tests")] privileged.rs:6): (a) up --capture-dns with --exclude 198.51.100.7/32 --exclude 192.0.2.53/32 and MAIN_IFACE default: two lines starting "8997:" containing "lookup main"; `ip route get 198.51.100.7` shows dev MAIN_IFACE; `ip route get 192.0.2.53 ipproto udp dport 53` shows dev MAIN_IFACE; 8997 line precedes 8999 line; (b) up with two, re-up with one -> exactly one 8997 line for the kept prefix; up without --exclude -> none; xray-down -> none; up again then recover --xray -> none; up again then recover --singbox -> no 8997 line (or, if recover --singbox does not touch xray rules at HEAD, assert and note that behavior unchanged); run the sudo command
- names: `const RULE_PREF_EXCLUDE: u32 = 8997;`; `pub async fn replace_exclusions(handle: &Handle, exclude: &[(IpAddr, u8)]) -> Result<(), String>`; `fn exclusion_installable(ip: IpAddr, host_has_ipv6: bool) -> bool  (true for v4; host_has_ipv6 for v6)`; `async fn del_rules_with_priority(handle: &Handle, priority: u32)  (both IpVersion, errors ignored like del_xray_rules)`; `add error: format!("add exclusion rule {ip}/{prefix} (pref {RULE_PREF_EXCLUDE}): {e}")`; `unit test: exclusion_installable_skips_v6_without_host_ipv6`; `privileged tests: exclusions_route_outside_tunnel_ahead_of_dns_capture, exclusions_replaced_on_reup_and_cleared_on_teardown; namespaces NS_EXCL = "nctl-excl-ns", NS_EXCL_REUP = "nctl-exclreup-ns"`; `assert_xray_state_cleared pref list (privileged.rs:392) gains "8997:"`
- verify: `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4`

### `x3` — tasks 1.2, 1.5 — seam `S2-netctl-cli`

- order: after `x2`, shared `crates/netctl/src`, shard `netctl`; coder `rust-coder`; packages `v2ray-rs-netctl`
- sites:
  - `crates/netctl/src/main.rs` · `Command::XrayUp` · anchor `strict: bool,` — new field `#[arg(long = "exclude", value_parser = validate::parse_exclusion)] exclude: Vec<(IpAddr, u8)>` after strict; doc comment notes that any caller can route prefixes (even 0.0.0.0/1 + 128.0.0.0/1) to main, which is no more than xray-down already allows
  - `crates/netctl/src/main.rs` · `run` · anchor `let v6 = addr6.as_deref().map(validate::parse_cidr).transpose()?;` — destructure exclude; validate::check_exclusion_count(exclude.len())? before `let handle = net::connect()?;`
  - `crates/netctl/src/main.rs` · `run (xray_up call)` · anchor `net::xray_up(&handle, &iface, v4, v6, bypass_uid, capture_dns, strict).await` — xray_up call unchanged; after it returns Ok, `net::replace_exclusions(&handle, &exclude).await?` so a failed start installs no new exclusion rules
  - `crates/netctl/src/main.rs` · `tests` · anchor `fn xray_up_parses_strict_flag() {` — xray_up_parses_repeated_exclude (two values parse in order to normalized tuples) and xray_up_refuses_invalid_exclude_naming_it (try_parse_from with 10.0.0.0/33 is Err and err.to_string() contains "10.0.0.0/33")
- work, in order:
  - 1.2 tests first in main.rs mod tests next to xray_up_parses_strict_flag: two values parse in order to normalized tuples; try_parse_from with 10.0.0.0/33 is Err and err.to_string() contains "10.0.0.0/33"
  - 1.2 impl: add field + doc comment (value_parser = validate::parse_exclusion); in run() validate::check_exclusion_count(exclude.len())? after parse_cidr(addr/addr6) and before net::connect(); call net::xray_up(...) unchanged, then net::replace_exclusions(&handle, &exclude).await? only after it succeeds
  - 1.5 verify: timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 green
- names: `CLI flag: --exclude <CIDR> (repeatable)`; `field: Command::XrayUp { exclude: Vec<(IpAddr, u8)>, .. } with #[arg(long, value_parser = validate::parse_exclusion)]`; `tests: xray_up_parses_repeated_exclude, xray_up_refuses_invalid_exclude_naming_it`
- verify: `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings`

### `x4` — tasks 2.1, 2.2 — seam `S4-process-ui-plumbing`

- order: parallel, shard `app`; coder `rust-coder`; packages `v2ray-rs-process`, `v2ray-rs-ui`
- sites:
  - `crates/process/src/tun.rs` · `TunRuntime` · anchor `pub strict: bool,` — new field pub exclude_routes: Vec<String>
  - `crates/process/src/tun.rs` · `xray_up_args` · anchor `if rt.strict {` — append --exclude <cidr> per entry (order preserved)
  - `crates/process/src/tun.rs` · `xray_needs_helper_singbox_does_not (literal)` · anchor `let mk = |backend| TunRuntime {` — add exclude_routes: Vec::new()
  - `crates/process/src/tun.rs` · `xray_rt (literal fixture)` · anchor `fn xray_rt(strict: bool) -> TunRuntime {` — add exclude_routes; update xray_up_args_include_strict_when_set expectation
  - `crates/process/src/tun.rs` · `xray_up_args_omit_strict_when_off` · anchor `fn xray_up_args_omit_strict_when_off() {` — uses ..xray_rt(false) (no compile break); override exclude_routes if fixture gains entries; new test for two exclusions in order
  - `crates/process/src/manager.rs` · `xray_on_lo (literal)` · anchor `fn xray_on_lo(helper: PathBuf) -> TunRuntime {` — add exclude_routes: Vec::new()
  - `crates/process/src/manager.rs` · `test literals x3 (.with_tun(Some(TunRuntime { ... helper_path: dir.path().join("missing-netctl") ...)` · anchor `.with_tun(Some(TunRuntime {` — 3 occurrences (2 Xray, 1 SingBox); add exclude_routes: Vec::new() to each
  - `crates/process/src/manager.rs` · `test literals x2 (mgr.tun = Some(TunRuntime {)` · anchor `mgr.tun = Some(TunRuntime {` — 2 occurrences (strict false / strict true); add exclude_routes: Vec::new()
  - `crates/process/src/manager.rs` · `test literals x2 (let rt = TunRuntime {, iface tun-test)` · anchor `let rt = TunRuntime {` — 2 occurrences; add exclude_routes: Vec::new()
  - `crates/ui/src/connection.rs` · `probe_skipped_when_parked_manager_holds_tun (literal)` · anchor `let tunneled = manager().with_tun(Some(TunRuntime {` — add exclude_routes: Vec::new()
  - `crates/ui/src/connection.rs` · `build_tun_runtime (literal)` · anchor `fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {` — exclude_routes: settings.tun.exclude_routes.clone() when backend == Xray, else Vec::new(); caller passes &effective_settings already (`let tun = build_tun_runtime(&effective_settings, pinned);`)
  - `crates/ui/src/connection.rs` · `tests` · anchor `fn pinned_hostname_arms_capture() {` — new tests nearby using xray_tun_settings()/tun_settings(): xray carries list, sing-box empty, effective-vs-persisted differing list carries the passed (effective) settings' list
- work, in order:
  - 2.1 tests first: extend the two xray_up_args fixtures, add xray_up_args_pass_exclusions_in_order; then add the field, emit pairs in xray_up_args, add exclude_routes: Vec::new() to every literal listed in summary (process + ui test literal); xray_up_args_omit_strict_when_off (uses ..xray_rt) must also set exclude_routes: Vec::new() or its assertion fails
  - 2.2 tests first in connection.rs: three tests named above; then build_tun_runtime sets exclude_routes: if backend == BackendType::Xray { settings.tun.exclude_routes.clone() } else { Vec::new() }
- names: `pub exclude_routes: Vec<String> on TunRuntime (after strict)`; `argument pair: "--exclude", <cidr>`; `xray_rt fixture gains exclude_routes: vec!["10.15.12.100/32".into()] and xray_up_args_include_strict_when_set expects "--exclude", "10.15.12.100/32" after "--strict"; xray_up_args_omit_strict_when_off sets exclude_routes: Vec::new()`; `tests: xray_up_args_pass_exclusions_in_order (process); xray_runtime_carries_exclude_routes, singbox_runtime_has_no_exclude_routes, runtime_uses_effective_exclude_routes (ui connection.rs mod tests)`
- verify: `make test-process && make test-ui && make clippy`

### `x5` — tasks 2.3 — seam `S6-core-validation`

- order: after `x4`, shared `crates/ui/src`, shard `app`; coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-ui`
- sites:
  - `crates/core/src/models/validation.rs` · `ValidationError::TooManyExcludedRoutes (new)` · anchor `InvalidTunMtu(u16),` — new variant with #[error("too many excluded routes: {0} (at most {max})", max = MAX_EXCLUDE_ROUTES)]; import MAX_EXCLUDE_ROUTES from the tun model
  - `crates/core/src/models/validation.rs` · `validate_exclude_route (new)` · anchor `.map_err(|_| ValidationError::InvalidIpCidr(cidr.to_string()))` — new pub fn next to validate_ip_cidr: parse as IpNet, refuse prefix_len 0 with InvalidIpCidr(value)
  - `crates/core/src/models/tun.rs` · `TunConfig::validate` · anchor `validate_ip_cidr(route)?;` — count check against MAX_EXCLUDE_ROUTES, then validate_exclude_route per route
  - `crates/core/src/models/tun.rs` · `tests` · anchor `let bad_exclude = TunConfig {` — add tun_config_refuses_more_than_256_exclude_routes next to it
  - `crates/ui/src/preferences/tun.rs` · `excluded-route add dialog` · anchor `if validate_ip_cidr(&value).is_ok() {` — use validate_exclude_route
- work, in order:
  - tests first: validation.rs exclude_route_refuses_whole_address_space (0.0.0.0/0, ::/0 → InvalidIpCidr naming the value), exclude_route_accepts_prefix (10.0.0.0/8, fd00::/64); tun.rs tun_config_refuses_more_than_256_exclude_routes (256 ok, 257 → TooManyExcludedRoutes(257)); existing bad_exclude assertion still InvalidIpCidr
  - validation.rs: validate_exclude_route = validate_ip_cidr then refuse prefix_len 0 (ipnet::IpNet::prefix_len); add TooManyExcludedRoutes variant; export via models (pub use validation::* already)
  - tun.rs TunConfig::validate: count check against MAX_EXCLUDE_ROUTES before the per-route loop, loop uses validate_exclude_route
  - ui preferences/tun.rs excluded-route add dialog: validate_exclude_route instead of validate_ip_cidr (import update); address rows unchanged
- names: `pub fn validate_exclude_route(cidr: &str) -> Result<(), ValidationError>`; `prefix 0 → ValidationError::InvalidIpCidr(value) (existing variant, names the value)`; `new variant ValidationError::TooManyExcludedRoutes(usize) with #[error("too many excluded routes: {0} (at most {max})", max = MAX_EXCLUDE_ROUTES)]`; `pub const MAX_EXCLUDE_ROUTES: usize = 256 in crates/core/src/models/tun.rs, with a comment that the route helper enforces the same cap (MAX_EXCLUSIONS in crates/netctl/src/validate.rs); x1 adds the reverse pointer on MAX_EXCLUSIONS`; `tests: exclude_route_refuses_whole_address_space, exclude_route_accepts_prefix, tun_config_refuses_more_than_256_exclude_routes`
- verify: `make test-core && make test-ui && make clippy`

### `x6` — tasks 3.1, 4.1 — seam `S5-ui-wording`

- order: after `x5`, shared `crates/ui/src`, shard `integration`; coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/preferences/tun.rs` · `routes_group` · anchor `.description("CIDRs that bypass the tunnel")` — description -> "CIDRs routed outside the tunnel; applies on next connect"
  - `crates/ui/src/preferences/tun.rs` · `domains_group` · anchor `.description("Domain suffixes that bypass the tunnel")` — description from new pure helper keyed by backend (initial backend var `backend`)
  - `crates/ui/src/preferences/tun.rs` · `subscribe_settings backend gating closure` · anchor `domains_group.set_sensitive(backend != BackendType::V2ray);` — first occurrence (inside subscribe_settings closure): add domains_group.set_description(helper(backend))
  - `crates/ui/src/preferences/tun.rs` · `new pure helper + test` · anchor `fn strict_route_applies(backend: BackendType) -> bool {` — new fn returning &'static str per backend next to strict_route_applies; unit test next to strict_route_row_sensitive_for_xray
- work, in order:
  - precondition: runs in the integration worktree only after both shard chains (netctl x1–x3 and app x4–x5) are merged into it
  - 3.1 tests first in preferences/tun.rs mod tests; then const + fn, use in builders, add domains_group.set_description(Some(excluded_domains_description(backend))) in subscribe_settings closure
  - 4.1 verify: timeout 10m cargo test --workspace -- --test-threads=4
  - docs: docs/ARCHITECTURE.md netctl paragraph and CLAUDE.md `crates/netctl` section gain `--exclude` on `xray-up` (pref 8997 rules to main, replaced on every xray-up, removed by xray-down/recover); verify each claim against the code before writing
  - CHANGELOG.md [Unreleased]: Fixed — xray TUN excluded routes now bypass the tunnel device (including DNS capture); Changed — excluded routes with prefix length 0 or lists over 256 entries are refused (a list above 256 must be trimmed in settings.toml)
- names: `const EXCLUDED_ROUTES_DESCRIPTION: &str = "CIDRs routed outside the tunnel; applies on next connect";`; `fn excluded_domains_description(backend: BackendType) -> &'static str`; `xray text: "Matching traffic still passes through the tunnel and is sent directly by xray; applies on next connect"`; `sing-box/v2ray text: "Domain suffixes that bypass the tunnel; applies on next connect"`; `tests: excluded_routes_description_names_next_connect, excluded_domains_description_per_backend`
- verify: `make test-ui && make clippy && timeout 10m cargo test --workspace -- --test-threads=4`

### Contracts

#### `S1-netctl-validate` — tasks 1.1

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Exclusion validation in crates/netctl/src/validate.rs on top of parse_cidr (validate.rs:29-44, which accepts prefix 0 and host bits). Pure, no netlink. v2ray-rs-netctl is a binary crate with private modules: items added in x1/x2 stay uncalled until x3 wires them, so x1/x2 verify with cargo test only and the clippy gate runs in x3; no #[allow(dead_code)] placeholders.

States: `accepted-normalized`, `refused`

| input | state | effect | evidence |
|---|---|---|---|
| "10.15.12.100/32" | `accepted-normalized` | set | tasks.md 1.1; returns (10.15.12.100, 32) unchanged |
| "10.1.2.3/8" (host bits set, v4) | `accepted-normalized` | forced | tasks.md 1.1; returns (10.0.0.0, 8) |
| "fd00::1/64" (host bits set, v6) | `accepted-normalized` | forced | tasks.md 1.1; returns (fd00::, 64) |
| "0.0.0.0/0" or "::/0" or any addr with prefix 0 | `refused` | set | design.md Decisions 'Validation in the helper'; spec tun-mode prefix 1..=32/128 |
| "1.2.3.4/33" or "::1/129" | `refused` | set | parse_cidr validate.rs:39-42 |
| non-CIDR ("garbage", "1.2.3.4", "1.2.3.4/x") | `refused` | set | parse_cidr validate.rs:30-38; spec scenario 'Invalid exclusion refused' |
| count 0..=256 | `accepted-normalized` | no-op | check_exclusion_count Ok |
| count 257 | `refused` | set | tasks.md 1.1 '257 values refused' |

Forbidden:
- accepted value with prefix 0 in either family
- accepted value whose host bits are non-zero
- host-bit mask computed before the prefix-0 refusal (u32 << 32 / u128 << 128 overflow)
- error text that does not contain the offending value (Debug-quoted, like parse_cidr)

Seeding:
- accepted-normalized: call validate::parse_exclusion(&str) directly
- refused: call validate::parse_exclusion with a bad value, or validate::check_exclusion_count(257)

Budgets:
- MAX_EXCLUSIONS = 256 values
- IPv4 prefix 1..=32
- IPv6 prefix 1..=128

#### `S2-netctl-cli` — tasks 1.2, 1.5

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. XrayUp gains repeatable --exclude; every value validated by clap value_parser at parse time and the count checked in run() before net::connect() (main.rs:94). Refusal layer: clap parse (per value, process exits 2 with clap error naming the value) and run() (count, exits 1 with 'netctl: ...' main.rs:68).

States: `parsed`, `parse-refused`, `count-refused`

| input | state | effect | evidence |
|---|---|---|---|
| no --exclude | `parsed` | no-op | exclude is empty Vec; replace still runs and clears prior 8997 rules (S3) |
| --exclude 10.0.0.0/8 --exclude 192.0.2.0/24 | `parsed` | set | tasks.md 1.2; Vec in argument order |
| --exclude 10.0.0.0/33 or --exclude garbage | `parse-refused` | set | spec scenario 'Invalid exclusion refused'; Cli::try_parse_from Err, message contains the value |
| 257 valid --exclude values | `count-refused` | set | run(): validate::check_exclusion_count before net::connect() |
| valid values but iface not a TUN device | `count-refused` | no-op | existing refusal main.rs:89-91 still precedes connect; no netlink change |

Forbidden:
- net::connect() reached before check_exclusion_count and all parse_exclusion calls complete
- an invalid value silently dropped instead of failing the command

Seeding:
- parsed / parse-refused: Cli::try_parse_from([... "xray-up", "--iface", "xtun0", "--addr", "172.19.0.1/30", "--exclude", v]) using the existing parse() helper pattern main.rs:119-130 (try variant for refusal)
- count-refused: not unit-reachable without netlink order; covered by S1 check_exclusion_count test plus code review of run() ordering

Budgets:
- MAX_EXCLUSIONS = 256
- unit run: timeout 5m, --test-threads=4

#### `S3-netctl-rules` — tasks 1.3, 1.4

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Pref-8997 destination rules to table main in crates/netctl/src/net.rs. Replace semantics in a separate pub fn net::replace_exclusions because adding an 8th parameter to xray_up (net.rs:72-80, 7 params today) trips clippy::too_many_arguments under -D warnings (no clippy.toml, no allow). Rule shape mirrors add_bypass_uid_rule (net.rs:357-382): header.family by addr, header.action ToTable, header.table = RT_TABLE_MAIN as u8, header.dst_len = prefix, RuleAttribute::Destination(ip) (netlink-packet-route 0.30.0 rule/attribute.rs:45), RuleAttribute::Priority(RULE_PREF_EXCLUDE). Teardown via is_xray_rule list (net.rs:397-412). v2ray-rs-netctl is a binary crate with private modules: items added in x1/x2 stay uncalled until x3 wires them, so x1/x2 verify with cargo test only and the clippy gate runs in x3; no #[allow(dead_code)] placeholders.

States: `no-8997-rules`, `8997-rules-v4`, `8997-rules-v6`

| input | state | effect | evidence |
|---|---|---|---|
| xray-up with N valid v4 exclusions, none installed | `8997-rules-v4` | set | spec 'Bring xray TUN routes up'; one rule per CIDR |
| xray-up with v6 exclusions, /proc/sys/net/ipv6 present | `8997-rules-v6` | set | host_has_ipv6 net.rs:137-139 |
| xray-up with v6 exclusion, host has IPv6 disabled | `8997-rules-v6` | no-op | design.md 'IPv6 exclusions ... skipped'; exclusion_installable returns false |
| xray-up with list M after prior list | `8997-rules-v4` | forced | spec 'Exclusions replaced on re-run': delete all pref-8997 both families, then add M |
| xray-up with no --exclude after prior list | `no-8997-rules` | clear | replace runs with empty list |
| two values equal after normalization (10.1.2.3/8, 10.0.0.0/8) | `8997-rules-v4` | no-op | EEXIST tolerated, is_exists net.rs:456-458; one rule |
| --capture-dns with --exclude 10.15.12.100/32; unmarked udp to :53 | `8997-rules-v4` | set | spec 'Excluded resolver skips DNS capture'; 8997 < 8999 (net.rs:37) |
| xray-down | `no-8997-rules` | clear | del_xray_rules net.rs:386-395 with 8997 added to is_xray_rule |
| recover --xray | `no-8997-rules` | clear | recover_xray -> xray_down net.rs:170-174 |
| recover --singbox | `no-8997-rules` | clear | recover_singbox -> xray_down net.rs:178-179 |
| non-EEXIST netlink error on add | `8997-rules-v4` | forced | returns Err -> xray-up exits 1 -> ProcessManager stops child with TunHelper, manager.rs:667-673 |

Forbidden:
- pref-8997 rule with any table other than main (254)
- pref-8997 rule with dst_len 0
- pref-8997 rule carrying fwmark, suppress_prefixlength, uidrange or port match
- more than 256 pref-8997 rules installed by one call
- pref-8997 rule surviving xray-down or recover
- rule from an earlier xray-up surviving a later xray-up whose list omits it
- IPv6 rule add attempted when host_has_ipv6() is false

Seeding:
- exclusion_installable: call the pure fn directly with (IpAddr, bool)
- no-8997-rules / 8997-rules-v4 / 8997-rules-v6: only via the netctl binary inside a namespace (netctl_in, privileged.rs:77-81); never `ip rule add pref 8997` by hand
- main default via physical-iface stand-in: MAIN_IFACE tuntap + addr + link up + `route add default dev MAIN_IFACE`, pattern privileged.rs:540-552

Budgets:
- pref 8997
- table main = 254 (RT_TABLE_MAIN net.rs:26)
- <= 256 rules per family per call
- helper call bounded by HELPER_TIMEOUT 10s (process tun.rs:15)
- privileged run: sudo -E timeout 10m, --test-threads=1

#### `S4-process-ui-plumbing` — tasks 2.1, 2.2

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. TunRuntime.exclude_routes (process tun.rs:31-45) emitted by xray_up_args (tun.rs:229-248) and filled by build_tun_runtime (ui connection.rs:822-850) from effective_settings (connection.rs:302). This seam fixes every TunRuntime literal: process tun.rs:432, 447 (481 uses ..xray_rt); manager.rs:1623, 1844, 1880, 1957, 2104, 2138, 2345, 2385; ui connection.rs:834 (prod), 2272 (test). Both tasks in one seam so the ui prod literal gets the real value, not a placeholder.

States: `no-exclude-args`, `exclude-args`

| input | state | effect | evidence |
|---|---|---|---|
| TunRuntime.exclude_routes empty | `no-exclude-args` | no-op | spec scenario 'No exclusions' |
| exclude_routes ["10.15.12.100/32", "91.230.107.224/32"] | `exclude-args` | set | spec 'Excluded routes reach the helper'; pairs appended after --strict, in order |
| build_tun_runtime, backend Xray, tun.enabled, exclude_routes non-empty | `exclude-args` | set | tasks.md 2.2; clone of settings.tun.exclude_routes |
| build_tun_runtime, backend SingBox | `no-exclude-args` | forced | spec: sing-box uses route_exclude_address (singbox.rs:197-199); needs_helper false tun.rs:50-52 |
| build_tun_runtime given effective settings whose exclude_routes differ from persisted | `exclude-args` | set | tasks.md 2.2; fn reads only its settings argument |
| tun disabled or backend V2ray | `no-exclude-args` | no-op | returns None connection.rs:823-829 |

Forbidden:
- sing-box runtime with non-empty exclude_routes
- xray_up_args reordering or deduplicating exclude_routes
- --exclude emitted without a following value, or joined as one comma string
- exclude_routes read from anything but the settings argument (persisted settings)

Seeding:
- xray_up_args: construct TunRuntime via xray_rt(strict) fixture (tun.rs:446-457) with struct update ..
- build_tun_runtime: tun_settings() (connection.rs:2842-2850) then set backend.backend_type and tun.exclude_routes; 'effective differs' = clone persisted, mutate clone's exclude_routes, pass clone

Budgets:
- make test-process / make test-ui: timeout 5m, --test-threads=4

#### `S5-ui-wording` — tasks 3.1, 4.1

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Group descriptions in crates/ui/src/preferences/tun.rs: routes_group (tun.rs:157-160) and domains_group (tun.rs:255-258); refresh in the subscribe_settings closure (tun.rs:723-738) via set_description. Owns the workspace floor 4.1.

States: `xray-wording`, `singbox-wording`

| input | state | effect | evidence |
|---|---|---|---|
| backend Xray at build or on settings change | `xray-wording` | set | spec 'xray domain exclusion wording' |
| backend SingBox at build or on settings change | `singbox-wording` | set | spec: sing-box text stays |
| backend V2ray | `singbox-wording` | forced | groups insensitive for v2ray tun.rs:734-735; default arm |
| any backend, routes group | `xray-wording` | no-op | spec 'Route exclusion wording': same text both backends |

Forbidden:
- routes description varying by backend
- xray domains text claiming traffic bypasses/stays off the tunnel
- description updated only at build and not on backend change

Seeding:
- call excluded_domains_description(BackendType::X) and read EXCLUDED_ROUTES_DESCRIPTION directly; no GTK in tests

Budgets:
- make test-ui: timeout 5m, --test-threads=4
- 4.1: timeout 10m cargo test --workspace -- --test-threads=4

#### `S6-core-validation` — tasks 2.3

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. App-side exclusion validation matching the helper: validate_exclude_route in crates/core/src/models/validation.rs, used by TunConfig::validate (crates/core/src/models/tun.rs) and by the excluded-route add dialog in crates/ui/src/preferences/tun.rs.

States: `route-accepted`, `route-refused`, `count-refused`

| input | state | effect | evidence |
|---|---|---|---|
| validate_exclude_route("10.0.0.0/8") | `route-accepted` | set | tasks.md 2.3 |
| validate_exclude_route("0.0.0.0/0") or ("::/0") | `route-refused` | set | spec tun-mode Scenario: Whole-address-space exclusion is refused |
| validate_exclude_route("bogus") (not a CIDR) | `route-refused` | set | existing validate_ip_cidr behavior |
| TunConfig with 257 exclude_routes | `count-refused` | set | spec tun-mode Scenario: Too many exclusions are refused |
| TunConfig with 256 valid exclude_routes | `route-accepted` | no-op | cap is inclusive of 256 |
| add dialog value refused by validate_exclude_route | `route-refused` | no-op | preferences/tun.rs add dialog keeps the existing silent-ignore on invalid input; nothing is pushed |

Forbidden:
- a prefix-0 route persisted through the add dialog
- address_v4/address_v6 validation changing (they keep validate_ip_cidr)
- a new UI control or error surface

Seeding:
- plain values: TunConfig { exclude_routes: vec![..], ..TunConfig::default() }; (0..257).map(|i| format!("10.{}.{}.0/24", i / 256, i % 256))

Budgets:
- MAX_EXCLUDE_ROUTES = 256 (same number as the helper cap)
- prefix length ≥ 1 for both families

### Floor

make fmt && make clippy && make test TEST_TIMEOUT=10m. MANUAL: privileged namespace tests (sudo -E timeout 10m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=1); tasks 4.2/4.3 live checks on xray TUN.

### Requirements map

- The excluded-routes and excluded-domains groups SHALL describe what each list does on the active backend. Excluded routes SHALL be described as destinations routed outside the tunnel on both backends. For xray, excluded domains SHALL be described as traffic that still enters the tunnel and is sent directly by xray after it recognizes the domain, so they do not keep traffic off the TUN device; for sing-box, the description SHALL describe domain suffixes that bypass the tunnel. Both groups SHALL state that changes apply on the next connect. → `excluded_routes_description_names_next_connect`, `excluded_domains_description_per_backend`
- **THEN** the excluded-domains description SHALL state that matching traffic still passes through the tunnel and is sent directly by xray → `excluded_domains_description_per_backend`
- **THEN** the excluded-routes description SHALL state that the CIDRs are routed outside the tunnel and that changes apply on the next connect → `excluded_routes_description_names_next_connect`
- The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998; with `--capture-dns`, unmarked udp and tcp traffic to port 53 looks up the TUN table (pref 8999), so a resolver on the local subnet is reached through the tunnel rather than the LAN route pref 9001 preserves. With one or more `--exclude <CIDR>` arguments, `xray-up` SHALL install, for each CIDR, a rule sending traffic destined to that prefix to `main` at pref 8997, ahead of DNS capture and the tunnel table, so excluded destinations — including resolvers on port 53 — never enter the TUN device; the pref 8997 rule set SHALL be replaced on every `xray-up`, so exclusions removed since the previous run do not linger. Each `--exclude` value SHALL be validated as an IPv4 or IPv6 CIDR before any netlink call, host bits SHALL be cleared, the number of values SHALL be bounded, and IPv6 exclusions SHALL be skipped when the host has IPv6 disabled. IPv6 equivalents SHALL be installed when an IPv6 address is supplied. With `--strict`, `xray-up` SHALL additionally install an `unreachable` default route with the lowest priority in table 2023 for IPv4 and IPv6, and SHALL install the IPv6 policy rules even when no IPv6 address is supplied — unless the host has IPv6 disabled, in which case the IPv6 fallback route and rules are skipped — so that traffic destined for the tunnel is refused rather than routed through the real default route whenever the TUN device is absent. Re-running `xray-up` against a recreated device of the same name SHALL succeed and leave exactly one copy of each route and rule. → `exclusions_route_outside_tunnel_ahead_of_dns_capture`, `exclusions_replaced_on_reup_and_cleared_on_teardown`, `exclusion_refuses_default_prefix`, `xray_up_refuses_invalid_exclude_naming_it`, `up_down_is_idempotent_in_namespace`, `exclusion_clears_host_bits`, `exclusion_installable_skips_v6_without_host_ipv6`, `xray_rule_prefs_cover_exclusions`
- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given, the pref 8999 port-53 rules when `--capture-dns` is given, and one pref 8997 rule per `--exclude` CIDR), each step idempotent → `up_down_is_idempotent_in_namespace`, `exclusions_route_outside_tunnel_ahead_of_dns_capture`
- **THEN** they SHALL match only unmarked traffic, so the backend's own resolver queries keep egressing the real interface through the pref 9000 rule → `capture_dns_steers_port_53_into_the_tunnel_table`
- **THEN** `ip route get 91.230.107.224` for an unmarked packet SHALL resolve through `main` to the physical interface, not the TUN device → `exclusions_route_outside_tunnel_ahead_of_dns_capture`
- **THEN** unmarked udp traffic to `10.15.12.100:53` SHALL match the pref 8997 rule before the pref 8999 capture rule and SHALL NOT enter the TUN device → `exclusions_route_outside_tunnel_ahead_of_dns_capture`
- **THEN** exactly one pref 8997 rule SHALL exist, for `10.0.0.0/8` → `exclusions_replaced_on_reup_and_cleared_on_teardown`
- **THEN** the helper SHALL exit with an error naming the value and SHALL make no netlink change → `xray_up_refuses_invalid_exclude_naming_it`, `exclusion_refuses_default_prefix`, `exclusion_refuses_out_of_range_prefix`, `exclusion_count_capped_at_256`
- **THEN** table 2023 SHALL contain an `unreachable` default route for IPv4 and for IPv6 at a lower priority than the device route, and the IPv6 pref 9000/9001/9002 rules SHALL be present whether or not an IPv6 address was supplied, on any host with IPv6 enabled → `strict_up_installs_fallback_routes_and_v6_rules`
- **THEN** unmarked traffic to a destination outside the on-link routes SHALL be refused with host-unreachable rather than sent through the real default route, while fwmark-255 and bypass-uid traffic SHALL keep using `main` → `strict_state_refuses_unmarked_traffic_without_device`
- **THEN** the helper SHALL succeed and the resulting routes and rules SHALL match a single fresh `xray-up` → `reup_across_recreated_device_leaves_one_copy`, `exclusions_replaced_on_reup_and_cleared_on_teardown`
- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences, including pref 8997) for both address families, remove the table-2023 fallback routes, delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when all are already absent → `down_and_recover_clear_strict_state_both_families`, `exclusions_replaced_on_reup_and_cleared_on_teardown`, `xray_rule_prefs_cover_exclusions`
- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray) including the fallback routes, and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean → `down_and_recover_clear_strict_state_both_families`, `exclusions_replaced_on_reup_and_cleared_on_teardown`
- For an xray TUN connection, the system SHALL pass every entry of the effective `tun.exclude_routes` to the route helper as an exclusion, built from the same effective settings the config was generated from. The generated xray routing rule sending those CIDRs to `direct` SHALL remain. sing-box connections SHALL keep expressing exclusions through `route_exclude_address` and SHALL NOT invoke the helper for them. Each excluded route SHALL be refused by settings validation when its prefix length is 0, and more than 256 excluded routes SHALL be refused, so the TUN preferences reject such an entry before it is saved and config generation fails with an error naming the value instead of the helper refusing it at connect. → `xray_runtime_carries_exclude_routes`, `singbox_runtime_has_no_exclude_routes`, `runtime_uses_effective_exclude_routes`, `xray_up_args_pass_exclusions_in_order`, `exclude_route_refuses_whole_address_space`, `tun_config_refuses_more_than_256_exclude_routes`
- **THEN** the route helper SHALL be invoked with `--exclude 10.15.12.100/32 --exclude 91.230.107.224/32` → `xray_up_args_pass_exclusions_in_order`, `xray_runtime_carries_exclude_routes`
- **THEN** the route helper SHALL be invoked without `--exclude` → `xray_up_args_omit_strict_when_off`, `singbox_runtime_has_no_exclude_routes`
- **THEN** the TUN preferences SHALL reject the entry, and settings carrying it SHALL fail validation with an error naming the value → `exclude_route_refuses_whole_address_space`
- **THEN** settings validation SHALL fail → `tun_config_refuses_more_than_256_exclude_routes`

### Test harness

- parse — crates/netctl/src/main.rs:    fn parse(extra: &[&str]) -> Cli { — Cli from xray-up --iface xtun0 --addr 172.19.0.1/30 + extra args
- run — crates/netctl/tests/privileged.rs:fn run(cmd: &str, args: &[&str]) -> bool { — runs a command, success bool
- ip_in — crates/netctl/tests/privileged.rs:fn ip_in(ns: &str, args: &[&str]) -> bool { — ip netns exec <ns> ip ..., bool
- ip_in_ns — crates/netctl/tests/privileged.rs:fn ip_in_ns(args: &[&str]) -> bool { — ip_in on NS
- ip_in_output — crates/netctl/tests/privileged.rs:fn ip_in_output(ns: &str, args: &[&str]) -> String { — stdout of ip in ns
- ip_in_ns_output — crates/netctl/tests/privileged.rs:fn ip_in_ns_output(args: &[&str]) -> String { — ip_in_output on NS
- ip_in_full — crates/netctl/tests/privileged.rs:fn ip_in_full(ns: &str, args: &[&str]) -> (bool, String) { — success + stdout+stderr (for route get)
- netctl_in — crates/netctl/tests/privileged.rs:fn netctl_in(ns: &str, args: &[&str]) -> bool { — runs helper binary BIN inside ns
- netctl — crates/netctl/tests/privileged.rs:fn netctl(args: &[&str]) -> bool { — netctl_in on NS
- device_exists — crates/netctl/tests/privileged.rs:fn device_exists() -> bool { — IFACE present in NS
- NsGuard — crates/netctl/tests/privileged.rs:struct NsGuard(&'static str); — deletes namespace on drop
- fallback_lines — crates/netctl/tests/privileged.rs:fn fallback_lines(ns: &str, family: &str) -> Vec<String> { — unreachable default lines of table 2023
- assert_xray_state_cleared — crates/netctl/tests/privileged.rs:fn assert_xray_state_cleared(ns: &str, after: &str) { — asserts empty table 2023 and no 8998-9002 rules both families
- xray_state — crates/netctl/tests/privileged.rs:fn xray_state(ns: &str) -> Vec<String> { — sorted rules + table-2023 routes both families
- consts — crates/netctl/tests/privileged.rs:const IFACE: &str = "nctltest0"; — IFACE nctltest0, MAIN_IFACE nctlmain0 (uplink stand-in), ADDR 172.31.255.1/30, ADDR6 fd00:ffff::1/64, NS/NS_DNS/NS_STRICT/NS_CLEAR/NS_REUP/NS_GONE
- uplink setup — crates/netctl/tests/privileged.rs:        &["route", "add", "default", "dev", MAIN_IFACE] — MAIN_IFACE tun with 10.99.0.1/24 + main default route (inline in strict_state_refuses_unmarked_traffic_without_device, not a helper)
- t — crates/process/src/tun.rs:    fn t(secs: u64) -> std::time::SystemTime { — epoch + secs
- xray_rt — crates/process/src/tun.rs:    fn xray_rt(strict: bool) -> TunRuntime { — xray runtime tun0, v4+v6 addr, bypass_uid 967, capture_dns true
- write_helper — crates/process/src/tun.rs:    fn write_helper(dir: &Path, body: &str) -> PathBuf { — executable sh script netctl in dir
- xray_on_lo — crates/process/src/manager.rs:    fn xray_on_lo(helper: PathBuf) -> TunRuntime { — xray runtime on lo with given helper
- tun_settings — crates/ui/src/connection.rs:    fn tun_settings() -> AppSettings { — default settings with tun.enabled
- xray_tun_settings — crates/ui/src/connection.rs:    fn xray_tun_settings() -> AppSettings { — tun_settings with backend Xray
- v2ray_settings — crates/ui/src/connection.rs:    fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings { — v2ray backend with ports/listen/tun flag
- singbox_settings — crates/ui/src/connection.rs:    fn singbox_settings() -> AppSettings { — sing-box settings
- strict_route_settings — crates/ui/src/connection.rs:    fn strict_route_settings() -> AppSettings { — strict-route settings
- node — crates/ui/src/connection.rs:    fn node(address: &str) -> ProxyNode { — shadowsocks node at address
- candidate — crates/ui/src/connection.rs:    fn candidate(address: &str) -> ConnectionCandidate { — connection candidate at address
- preferences/tun.rs tests — crates/ui/src/preferences/tun.rs:    fn strict_route_row_sensitive_for_xray() { — only test; no fixtures

### Plan review

Reviewer `zarchitect`, verdict `pass` after 2 rounds. Round 1 blocker: netctl is a binary crate, so items added in x1/x2 stay uncalled until x3 wires them and clippy -D warnings would fail those chunks (x1/x2 verify with cargo test only; the clippy gate runs in x3). Round 1 warnings folded in: xray_up signature unchanged with a separate replace_exclusions run only after xray_up succeeds, XRAY_RULE_PREFS pure test, linked 256 caps, CHANGELOG entry and upgrade note, privileged-only SHALLs recorded as manual proof. Round 2 warnings folded in: error text via MAX_EXCLUDE_ROUTES, cross-reference comment on MAX_EXCLUSIONS, recover --singbox assertion, x6 starts after both shard chains merge.

Risks:

- Privileged namespace tests (1.4) need root and /dev/net/tun; the floor cannot run them unattended. Mitigation: run the sudo command manually and record the result; the file is still compiled by make clippy (--all-features) so it must stay clippy-clean.
- Replace is delete-then-add; deletion errors are ignored (mirrors del_xray_rules), so a failed delete could leave a stale 8997 rule until xray-down. Mitigation: re-up test asserts exact count; generator ip->direct rule covers the gap.
- Pref 8997 matching in is_xray_rule deletes any foreign rule at that priority (same as existing reserved prefs). Mitigation: documented reserved range 8997-9002.
- `ip route get` against a tuntap device with no attached fd (NO-CARRIER) in the namespace: excluded address asserts dev MAIN_IFACE, so it does not depend on the TUN device resolving. Mitigation: assert on MAIN_IFACE only.
- docs/ARCHITECTURE.md and CLAUDE.md describe netctl xray-up flags; not in tasks. Mitigation: update the netctl paragraph (pref 8997 --exclude) and CHANGELOG [Unreleased] alongside 1.x.
- Privileged namespace tests (task 1.4) are compiled and clippy-checked by the floor but only run manually: sudo -E timeout 10m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=1
- Several helper SHALLs (8997 rules to main, route get via the physical interface, 8997 before 8999, replace on re-up, teardown by xray-down/recover) are proven only by the privileged namespace tests, which run manually with sudo; the run result is recorded before archiving.
- Settings saved before this change with a prefix-0 excluded route or more than 256 routes now fail validation: connect reports the value, and TUN preference edits are refused until the offending /0 entry is removed; a list above 256 cannot shrink through the UI one entry at a time (edit settings.toml). Rare; noted in CHANGELOG [Unreleased].

## Plan appendix

```json
{
  "v": 2,
  "change": "route-xray-exclusions-outside-tun",
  "baseSha": "d9ded0a70d6fc6e2f2c01bdbc2643421ac946b44",
  "generatedAt": "2026-09-16T15:00:04.229Z",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "sec"
  ],
  "chunks": [
    {
      "id": "x1",
      "taskIds": [
        "1.1"
      ],
      "seam": "S1-netctl-validate",
      "contract": {
        "states": [
          "accepted-normalized",
          "refused"
        ],
        "transitions": [
          {
            "input": "\"10.15.12.100/32\"",
            "state": "accepted-normalized",
            "effect": "set",
            "evidence": "tasks.md 1.1; returns (10.15.12.100, 32) unchanged"
          },
          {
            "input": "\"10.1.2.3/8\" (host bits set, v4)",
            "state": "accepted-normalized",
            "effect": "forced",
            "evidence": "tasks.md 1.1; returns (10.0.0.0, 8)"
          },
          {
            "input": "\"fd00::1/64\" (host bits set, v6)",
            "state": "accepted-normalized",
            "effect": "forced",
            "evidence": "tasks.md 1.1; returns (fd00::, 64)"
          },
          {
            "input": "\"0.0.0.0/0\" or \"::/0\" or any addr with prefix 0",
            "state": "refused",
            "effect": "set",
            "evidence": "design.md Decisions 'Validation in the helper'; spec tun-mode prefix 1..=32/128"
          },
          {
            "input": "\"1.2.3.4/33\" or \"::1/129\"",
            "state": "refused",
            "effect": "set",
            "evidence": "parse_cidr validate.rs:39-42"
          },
          {
            "input": "non-CIDR (\"garbage\", \"1.2.3.4\", \"1.2.3.4/x\")",
            "state": "refused",
            "effect": "set",
            "evidence": "parse_cidr validate.rs:30-38; spec scenario 'Invalid exclusion refused'"
          },
          {
            "input": "count 0..=256",
            "state": "accepted-normalized",
            "effect": "no-op",
            "evidence": "check_exclusion_count Ok"
          },
          {
            "input": "count 257",
            "state": "refused",
            "effect": "set",
            "evidence": "tasks.md 1.1 '257 values refused'"
          }
        ],
        "forbidden": [
          "accepted value with prefix 0 in either family",
          "accepted value whose host bits are non-zero",
          "host-bit mask computed before the prefix-0 refusal (u32 << 32 / u128 << 128 overflow)",
          "error text that does not contain the offending value (Debug-quoted, like parse_cidr)"
        ],
        "seeding": [
          "accepted-normalized: call validate::parse_exclusion(&str) directly",
          "refused: call validate::parse_exclusion with a bad value, or validate::check_exclusion_count(257)"
        ],
        "budgets": [
          "MAX_EXCLUSIONS = 256 values",
          "IPv4 prefix 1..=32",
          "IPv6 prefix 1..=128"
        ],
        "names": [
          "pub const MAX_EXCLUSIONS: usize = 256;",
          "pub fn parse_exclusion(cidr: &str) -> Result<(IpAddr, u8), String>",
          "pub fn check_exclusion_count(count: usize) -> Result<(), String>",
          "prefix-0 error: format!(\"exclusion prefix length 0 refused: {cidr:?}\")",
          "count error: format!(\"too many exclusions: {count} (max {MAX_EXCLUSIONS})\")",
          "reused parse_cidr errors: \"invalid cidr (missing prefix): {cidr:?}\", \"invalid cidr address: {cidr:?}\", \"invalid cidr prefix: {cidr:?}\", \"cidr prefix out of range: {cidr:?}\"",
          "tests: exclusion_accepts_host_route, exclusion_clears_host_bits, exclusion_refuses_default_prefix, exclusion_refuses_out_of_range_prefix, exclusion_count_capped_at_256"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1 tests first in validate.rs mod tests (names above; v4+v6 normalization, both /0 refused, /33 refused, check_exclusion_count(256) Ok and (257) Err with text containing \"257\")",
        "1.1 impl: parse_exclusion = parse_cidr -> refuse prefix 0 -> mask host bits (v4 via u32, v6 via u128); check_exclusion_count",
        "MAX_EXCLUSIONS gets a comment that the app enforces the same cap (MAX_EXCLUDE_ROUTES in crates/core/src/models/tun.rs)"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "shard": "netctl",
      "pkgDirs": [
        "crates/netctl/src"
      ],
      "pkgs": [
        "v2ray-rs-netctl"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/netctl/src/validate.rs",
          "symbol": "parse_cidr (new sibling exclusion parser)",
          "anchor": "pub fn parse_cidr(cidr: &str) -> Result<(IpAddr, u8), String> {",
          "change": "new fn after parse_cidr: parse each value via parse_cidr, refuse prefix 0, mask host bits (v4/v6), refuse count > 256; errors Result<_, String> naming the value"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/validate.rs",
          "symbol": "tests",
          "anchor": "    fn cidr_rejects_bad() {",
          "change": "new tests after cidr_rejects_bad: 10.15.12.100/32 ok, 10.1.2.3/8 -> 10.0.0.0/8, fd00::1/64 -> fd00::/64, 0.0.0.0/0 and ::/0 refused, 1.2.3.4/33 refused, 257 values refused"
        }
      ],
      "verify": "timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4"
    },
    {
      "id": "x2",
      "taskIds": [
        "1.3",
        "1.4"
      ],
      "seam": "S3-netctl-rules",
      "contract": {
        "states": [
          "no-8997-rules",
          "8997-rules-v4",
          "8997-rules-v6"
        ],
        "transitions": [
          {
            "input": "xray-up with N valid v4 exclusions, none installed",
            "state": "8997-rules-v4",
            "effect": "set",
            "evidence": "spec 'Bring xray TUN routes up'; one rule per CIDR"
          },
          {
            "input": "xray-up with v6 exclusions, /proc/sys/net/ipv6 present",
            "state": "8997-rules-v6",
            "effect": "set",
            "evidence": "host_has_ipv6 net.rs:137-139"
          },
          {
            "input": "xray-up with v6 exclusion, host has IPv6 disabled",
            "state": "8997-rules-v6",
            "effect": "no-op",
            "evidence": "design.md 'IPv6 exclusions ... skipped'; exclusion_installable returns false"
          },
          {
            "input": "xray-up with list M after prior list",
            "state": "8997-rules-v4",
            "effect": "forced",
            "evidence": "spec 'Exclusions replaced on re-run': delete all pref-8997 both families, then add M"
          },
          {
            "input": "xray-up with no --exclude after prior list",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "replace runs with empty list"
          },
          {
            "input": "two values equal after normalization (10.1.2.3/8, 10.0.0.0/8)",
            "state": "8997-rules-v4",
            "effect": "no-op",
            "evidence": "EEXIST tolerated, is_exists net.rs:456-458; one rule"
          },
          {
            "input": "--capture-dns with --exclude 10.15.12.100/32; unmarked udp to :53",
            "state": "8997-rules-v4",
            "effect": "set",
            "evidence": "spec 'Excluded resolver skips DNS capture'; 8997 < 8999 (net.rs:37)"
          },
          {
            "input": "xray-down",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "del_xray_rules net.rs:386-395 with 8997 added to is_xray_rule"
          },
          {
            "input": "recover --xray",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "recover_xray -> xray_down net.rs:170-174"
          },
          {
            "input": "recover --singbox",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "recover_singbox -> xray_down net.rs:178-179"
          },
          {
            "input": "non-EEXIST netlink error on add",
            "state": "8997-rules-v4",
            "effect": "forced",
            "evidence": "returns Err -> xray-up exits 1 -> ProcessManager stops child with TunHelper, manager.rs:667-673"
          }
        ],
        "forbidden": [
          "pref-8997 rule with any table other than main (254)",
          "pref-8997 rule with dst_len 0",
          "pref-8997 rule carrying fwmark, suppress_prefixlength, uidrange or port match",
          "more than 256 pref-8997 rules installed by one call",
          "pref-8997 rule surviving xray-down or recover",
          "rule from an earlier xray-up surviving a later xray-up whose list omits it",
          "IPv6 rule add attempted when host_has_ipv6() is false"
        ],
        "seeding": [
          "exclusion_installable: call the pure fn directly with (IpAddr, bool)",
          "no-8997-rules / 8997-rules-v4 / 8997-rules-v6: only via the netctl binary inside a namespace (netctl_in, privileged.rs:77-81); never `ip rule add pref 8997` by hand",
          "main default via physical-iface stand-in: MAIN_IFACE tuntap + addr + link up + `route add default dev MAIN_IFACE`, pattern privileged.rs:540-552"
        ],
        "budgets": [
          "pref 8997",
          "table main = 254 (RT_TABLE_MAIN net.rs:26)",
          "<= 256 rules per family per call",
          "helper call bounded by HELPER_TIMEOUT 10s (process tun.rs:15)",
          "privileged run: sudo -E timeout 10m, --test-threads=1"
        ],
        "names": [
          "const RULE_PREF_EXCLUDE: u32 = 8997;",
          "pub async fn replace_exclusions(handle: &Handle, exclude: &[(IpAddr, u8)]) -> Result<(), String>",
          "fn exclusion_installable(ip: IpAddr, host_has_ipv6: bool) -> bool  (true for v4; host_has_ipv6 for v6)",
          "async fn del_rules_with_priority(handle: &Handle, priority: u32)  (both IpVersion, errors ignored like del_xray_rules)",
          "add error: format!(\"add exclusion rule {ip}/{prefix} (pref {RULE_PREF_EXCLUDE}): {e}\")",
          "unit test: exclusion_installable_skips_v6_without_host_ipv6",
          "privileged tests: exclusions_route_outside_tunnel_ahead_of_dns_capture, exclusions_replaced_on_reup_and_cleared_on_teardown; namespaces NS_EXCL = \"nctl-excl-ns\", NS_EXCL_REUP = \"nctl-exclreup-ns\"",
          "assert_xray_state_cleared pref list (privileged.rs:392) gains \"8997:\""
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.3 unit tests first in net.rs mod tests: exclusion_installable_skips_v6_without_host_ipv6 (table over v4/v6 x host true/false); xray_rule_prefs_cover_exclusions (XRAY_RULE_PREFS contains RULE_PREF_EXCLUDE)",
        "1.3 impl: RULE_PREF_EXCLUDE = 8997, XRAY_RULE_PREFS const used by is_xray_rule, exclusion_installable, del_rules_with_priority, add_exclusion_rule, replace_exclusions (delete all 8997 both families, then add each installable entry in order; only ever table main, dst_len ≥ 1); update RULE_PREF docs; xray_up signature unchanged",
        "1.4 privileged tests (compile under clippy --all-features, file gated by #![cfg(feature = \"privileged-tests\")] privileged.rs:6): (a) up --capture-dns with --exclude 198.51.100.7/32 --exclude 192.0.2.53/32 and MAIN_IFACE default: two lines starting \"8997:\" containing \"lookup main\"; `ip route get 198.51.100.7` shows dev MAIN_IFACE; `ip route get 192.0.2.53 ipproto udp dport 53` shows dev MAIN_IFACE; 8997 line precedes 8999 line; (b) up with two, re-up with one -> exactly one 8997 line for the kept prefix; up without --exclude -> none; xray-down -> none; up again then recover --xray -> none; up again then recover --singbox -> no 8997 line (or, if recover --singbox does not touch xray rules at HEAD, assert and note that behavior unchanged); run the sudo command"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "x1",
      "sharedPkg": "crates/netctl/src",
      "parallel": false,
      "shard": "netctl",
      "pkgDirs": [
        "crates/netctl/src",
        "crates/netctl/tests"
      ],
      "pkgs": [
        "v2ray-rs-netctl"
      ],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "RULE_PREF_EXCLUDE",
          "anchor": "const RULE_PREF_BYPASS_UID: u32 = 8998;",
          "change": "new const RULE_PREF_EXCLUDE: u32 = 8997 near it"
        },
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "xray_up",
          "anchor": "pub async fn xray_up(",
          "change": "signature unchanged (already 7 params; an 8th trips clippy::too_many_arguments). Exclusion handling lives in new pub async fn replace_exclusions(handle, &[(IpAddr, u8)]) next to it: delete every pref-8997 rule for both families, then add one destination-prefix rule to table main per installable entry, in order; EEXIST tolerated"
        },
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "add_rule / new add_exclude_rule",
          "anchor": "async fn add_rule(",
          "change": "add_rule has no destination attr; new add_exclusion_rule pushing RuleAttribute::Destination + header.dst_len, family by address, table main; error format!(\"add exclusion rule {ip}/{prefix} (pref {RULE_PREF_EXCLUDE}): {e}\")"
        },
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "del_xray_rules (pattern for pref-8997 delete)",
          "anchor": "async fn del_xray_rules(handle: &Handle) {",
          "change": "new sibling deleting only pref-8997 rules for IpVersion::V4/V6 (same get/try_next/del loop)"
        },
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "is_xray_rule",
          "anchor": "                    RULE_PREF_BYPASS_UID,",
          "change": "factor the reserved list into const XRAY_RULE_PREFS: [u32; 6] including RULE_PREF_EXCLUDE; is_xray_rule matches against it; pure test xray_rule_prefs_cover_exclusions asserts XRAY_RULE_PREFS contains RULE_PREF_EXCLUDE (8997)"
        },
        {
          "task": "1.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "v6_rules_needed (pattern) / tests",
          "anchor": "    fn v6_rules_follow_address_strict_and_host_support() {",
          "change": "new pure fn for v6 exclusion skip decision + table test next to this one; extend `use super::{XRAY_FWMARK, v6_rules_needed};`"
        },
        {
          "task": "1.4",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "NS consts",
          "anchor": "const NS_GONE: &str = \"nctl-gone-ns\";",
          "change": "new namespace const for exclusion test"
        },
        {
          "task": "1.4",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "new test",
          "anchor": "fn strict_state_refuses_unmarked_traffic_without_device() {",
          "change": "new #[test] (MAIN_IFACE uplink setup as in this test): up with two --exclude -> two 8997: rules, route get excluded addr via MAIN_IFACE; --capture-dns + excluded :53 (ip route get ... ipproto udp dport 53) via main; re-up one exclusion -> one rule; xray-down and recover --xray -> no 8997:"
        },
        {
          "task": "1.4",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "assert_xray_state_cleared",
          "anchor": "        for pref in [\"8998:\", \"8999:\", \"9000:\", \"9001:\", \"9002:\"] {",
          "change": "optionally add \"8997:\" to leaked-pref list"
        }
      ],
      "verify": "timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4"
    },
    {
      "id": "x3",
      "taskIds": [
        "1.2",
        "1.5"
      ],
      "seam": "S2-netctl-cli",
      "contract": {
        "states": [
          "parsed",
          "parse-refused",
          "count-refused"
        ],
        "transitions": [
          {
            "input": "no --exclude",
            "state": "parsed",
            "effect": "no-op",
            "evidence": "exclude is empty Vec; replace still runs and clears prior 8997 rules (S3)"
          },
          {
            "input": "--exclude 10.0.0.0/8 --exclude 192.0.2.0/24",
            "state": "parsed",
            "effect": "set",
            "evidence": "tasks.md 1.2; Vec in argument order"
          },
          {
            "input": "--exclude 10.0.0.0/33 or --exclude garbage",
            "state": "parse-refused",
            "effect": "set",
            "evidence": "spec scenario 'Invalid exclusion refused'; Cli::try_parse_from Err, message contains the value"
          },
          {
            "input": "257 valid --exclude values",
            "state": "count-refused",
            "effect": "set",
            "evidence": "run(): validate::check_exclusion_count before net::connect()"
          },
          {
            "input": "valid values but iface not a TUN device",
            "state": "count-refused",
            "effect": "no-op",
            "evidence": "existing refusal main.rs:89-91 still precedes connect; no netlink change"
          }
        ],
        "forbidden": [
          "net::connect() reached before check_exclusion_count and all parse_exclusion calls complete",
          "an invalid value silently dropped instead of failing the command"
        ],
        "seeding": [
          "parsed / parse-refused: Cli::try_parse_from([... \"xray-up\", \"--iface\", \"xtun0\", \"--addr\", \"172.19.0.1/30\", \"--exclude\", v]) using the existing parse() helper pattern main.rs:119-130 (try variant for refusal)",
          "count-refused: not unit-reachable without netlink order; covered by S1 check_exclusion_count test plus code review of run() ordering"
        ],
        "budgets": [
          "MAX_EXCLUSIONS = 256",
          "unit run: timeout 5m, --test-threads=4"
        ],
        "names": [
          "CLI flag: --exclude <CIDR> (repeatable)",
          "field: Command::XrayUp { exclude: Vec<(IpAddr, u8)>, .. } with #[arg(long, value_parser = validate::parse_exclusion)]",
          "tests: xray_up_parses_repeated_exclude, xray_up_refuses_invalid_exclude_naming_it"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.2 tests first in main.rs mod tests next to xray_up_parses_strict_flag: two values parse in order to normalized tuples; try_parse_from with 10.0.0.0/33 is Err and err.to_string() contains \"10.0.0.0/33\"",
        "1.2 impl: add field + doc comment (value_parser = validate::parse_exclusion); in run() validate::check_exclusion_count(exclude.len())? after parse_cidr(addr/addr6) and before net::connect(); call net::xray_up(...) unchanged, then net::replace_exclusions(&handle, &exclude).await? only after it succeeds",
        "1.5 verify: timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 green"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "x2",
      "sharedPkg": "crates/netctl/src",
      "parallel": false,
      "shard": "netctl",
      "pkgDirs": [
        "crates/netctl/src"
      ],
      "pkgs": [
        "v2ray-rs-netctl"
      ],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/netctl/src/main.rs",
          "symbol": "Command::XrayUp",
          "anchor": "        strict: bool,",
          "change": "new field `#[arg(long = \"exclude\", value_parser = validate::parse_exclusion)] exclude: Vec<(IpAddr, u8)>` after strict; doc comment notes that any caller can route prefixes (even 0.0.0.0/1 + 128.0.0.0/1) to main, which is no more than xray-down already allows"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/main.rs",
          "symbol": "run",
          "anchor": "            let v6 = addr6.as_deref().map(validate::parse_cidr).transpose()?;",
          "change": "destructure exclude; validate::check_exclusion_count(exclude.len())? before `let handle = net::connect()?;`"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/main.rs",
          "symbol": "run (xray_up call)",
          "anchor": "            net::xray_up(&handle, &iface, v4, v6, bypass_uid, capture_dns, strict).await",
          "change": "xray_up call unchanged; after it returns Ok, `net::replace_exclusions(&handle, &exclude).await?` so a failed start installs no new exclusion rules"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/main.rs",
          "symbol": "tests",
          "anchor": "    fn xray_up_parses_strict_flag() {",
          "change": "xray_up_parses_repeated_exclude (two values parse in order to normalized tuples) and xray_up_refuses_invalid_exclude_naming_it (try_parse_from with 10.0.0.0/33 is Err and err.to_string() contains \"10.0.0.0/33\")"
        }
      ],
      "verify": "timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings"
    },
    {
      "id": "x4",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "seam": "S4-process-ui-plumbing",
      "contract": {
        "states": [
          "no-exclude-args",
          "exclude-args"
        ],
        "transitions": [
          {
            "input": "TunRuntime.exclude_routes empty",
            "state": "no-exclude-args",
            "effect": "no-op",
            "evidence": "spec scenario 'No exclusions'"
          },
          {
            "input": "exclude_routes [\"10.15.12.100/32\", \"91.230.107.224/32\"]",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "spec 'Excluded routes reach the helper'; pairs appended after --strict, in order"
          },
          {
            "input": "build_tun_runtime, backend Xray, tun.enabled, exclude_routes non-empty",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "tasks.md 2.2; clone of settings.tun.exclude_routes"
          },
          {
            "input": "build_tun_runtime, backend SingBox",
            "state": "no-exclude-args",
            "effect": "forced",
            "evidence": "spec: sing-box uses route_exclude_address (singbox.rs:197-199); needs_helper false tun.rs:50-52"
          },
          {
            "input": "build_tun_runtime given effective settings whose exclude_routes differ from persisted",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "tasks.md 2.2; fn reads only its settings argument"
          },
          {
            "input": "tun disabled or backend V2ray",
            "state": "no-exclude-args",
            "effect": "no-op",
            "evidence": "returns None connection.rs:823-829"
          }
        ],
        "forbidden": [
          "sing-box runtime with non-empty exclude_routes",
          "xray_up_args reordering or deduplicating exclude_routes",
          "--exclude emitted without a following value, or joined as one comma string",
          "exclude_routes read from anything but the settings argument (persisted settings)"
        ],
        "seeding": [
          "xray_up_args: construct TunRuntime via xray_rt(strict) fixture (tun.rs:446-457) with struct update ..",
          "build_tun_runtime: tun_settings() (connection.rs:2842-2850) then set backend.backend_type and tun.exclude_routes; 'effective differs' = clone persisted, mutate clone's exclude_routes, pass clone"
        ],
        "budgets": [
          "make test-process / make test-ui: timeout 5m, --test-threads=4"
        ],
        "names": [
          "pub exclude_routes: Vec<String> on TunRuntime (after strict)",
          "argument pair: \"--exclude\", <cidr>",
          "xray_rt fixture gains exclude_routes: vec![\"10.15.12.100/32\".into()] and xray_up_args_include_strict_when_set expects \"--exclude\", \"10.15.12.100/32\" after \"--strict\"; xray_up_args_omit_strict_when_off sets exclude_routes: Vec::new()",
          "tests: xray_up_args_pass_exclusions_in_order (process); xray_runtime_carries_exclude_routes, singbox_runtime_has_no_exclude_routes, runtime_uses_effective_exclude_routes (ui connection.rs mod tests)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1 tests first: extend the two xray_up_args fixtures, add xray_up_args_pass_exclusions_in_order; then add the field, emit pairs in xray_up_args, add exclude_routes: Vec::new() to every literal listed in summary (process + ui test literal); xray_up_args_omit_strict_when_off (uses ..xray_rt) must also set exclude_routes: Vec::new() or its assertion fails",
        "2.2 tests first in connection.rs: three tests named above; then build_tun_runtime sets exclude_routes: if backend == BackendType::Xray { settings.tun.exclude_routes.clone() } else { Vec::new() }"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "shard": "app",
      "pkgDirs": [
        "crates/process/src",
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "TunRuntime",
          "anchor": "    pub strict: bool,",
          "change": "new field pub exclude_routes: Vec<String>"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_up_args",
          "anchor": "    if rt.strict {",
          "change": "append --exclude <cidr> per entry (order preserved)"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_needs_helper_singbox_does_not (literal)",
          "anchor": "        let mk = |backend| TunRuntime {",
          "change": "add exclude_routes: Vec::new()"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_rt (literal fixture)",
          "anchor": "    fn xray_rt(strict: bool) -> TunRuntime {",
          "change": "add exclude_routes; update xray_up_args_include_strict_when_set expectation"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_up_args_omit_strict_when_off",
          "anchor": "    fn xray_up_args_omit_strict_when_off() {",
          "change": "uses ..xray_rt(false) (no compile break); override exclude_routes if fixture gains entries; new test for two exclusions in order"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "xray_on_lo (literal)",
          "anchor": "    fn xray_on_lo(helper: PathBuf) -> TunRuntime {",
          "change": "add exclude_routes: Vec::new()"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "test literals x3 (.with_tun(Some(TunRuntime { ... helper_path: dir.path().join(\"missing-netctl\") ...)",
          "anchor": "        .with_tun(Some(TunRuntime {",
          "change": "3 occurrences (2 Xray, 1 SingBox); add exclude_routes: Vec::new() to each"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "test literals x2 (mgr.tun = Some(TunRuntime {)",
          "anchor": "        mgr.tun = Some(TunRuntime {",
          "change": "2 occurrences (strict false / strict true); add exclude_routes: Vec::new()"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "test literals x2 (let rt = TunRuntime {, iface tun-test)",
          "anchor": "        let rt = TunRuntime {",
          "change": "2 occurrences; add exclude_routes: Vec::new()"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "probe_skipped_when_parked_manager_holds_tun (literal)",
          "anchor": "        let tunneled = manager().with_tun(Some(TunRuntime {",
          "change": "add exclude_routes: Vec::new()"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "build_tun_runtime (literal)",
          "anchor": "fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {",
          "change": "exclude_routes: settings.tun.exclude_routes.clone() when backend == Xray, else Vec::new(); caller passes &effective_settings already (`let tun = build_tun_runtime(&effective_settings, pinned);`)"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests",
          "anchor": "    fn pinned_hostname_arms_capture() {",
          "change": "new tests nearby using xray_tun_settings()/tun_settings(): xray carries list, sing-box empty, effective-vs-persisted differing list carries the passed (effective) settings' list"
        }
      ],
      "verify": "make test-process && make test-ui && make clippy"
    },
    {
      "id": "x5",
      "taskIds": [
        "2.3"
      ],
      "seam": "S6-core-validation",
      "contract": {
        "states": [
          "route-accepted",
          "route-refused",
          "count-refused"
        ],
        "transitions": [
          {
            "input": "validate_exclude_route(\"10.0.0.0/8\")",
            "state": "route-accepted",
            "effect": "set",
            "evidence": "tasks.md 2.3"
          },
          {
            "input": "validate_exclude_route(\"0.0.0.0/0\") or (\"::/0\")",
            "state": "route-refused",
            "effect": "set",
            "evidence": "spec tun-mode Scenario: Whole-address-space exclusion is refused"
          },
          {
            "input": "validate_exclude_route(\"bogus\") (not a CIDR)",
            "state": "route-refused",
            "effect": "set",
            "evidence": "existing validate_ip_cidr behavior"
          },
          {
            "input": "TunConfig with 257 exclude_routes",
            "state": "count-refused",
            "effect": "set",
            "evidence": "spec tun-mode Scenario: Too many exclusions are refused"
          },
          {
            "input": "TunConfig with 256 valid exclude_routes",
            "state": "route-accepted",
            "effect": "no-op",
            "evidence": "cap is inclusive of 256"
          },
          {
            "input": "add dialog value refused by validate_exclude_route",
            "state": "route-refused",
            "effect": "no-op",
            "evidence": "preferences/tun.rs add dialog keeps the existing silent-ignore on invalid input; nothing is pushed"
          }
        ],
        "forbidden": [
          "a prefix-0 route persisted through the add dialog",
          "address_v4/address_v6 validation changing (they keep validate_ip_cidr)",
          "a new UI control or error surface"
        ],
        "seeding": [
          "plain values: TunConfig { exclude_routes: vec![..], ..TunConfig::default() }; (0..257).map(|i| format!(\"10.{}.{}.0/24\", i / 256, i % 256))"
        ],
        "budgets": [
          "MAX_EXCLUDE_ROUTES = 256 (same number as the helper cap)",
          "prefix length ≥ 1 for both families"
        ],
        "names": [
          "pub fn validate_exclude_route(cidr: &str) -> Result<(), ValidationError>",
          "prefix 0 → ValidationError::InvalidIpCidr(value) (existing variant, names the value)",
          "new variant ValidationError::TooManyExcludedRoutes(usize) with #[error(\"too many excluded routes: {0} (at most {max})\", max = MAX_EXCLUDE_ROUTES)]",
          "pub const MAX_EXCLUDE_ROUTES: usize = 256 in crates/core/src/models/tun.rs, with a comment that the route helper enforces the same cap (MAX_EXCLUSIONS in crates/netctl/src/validate.rs); x1 adds the reverse pointer on MAX_EXCLUSIONS",
          "tests: exclude_route_refuses_whole_address_space, exclude_route_accepts_prefix, tun_config_refuses_more_than_256_exclude_routes"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: validation.rs exclude_route_refuses_whole_address_space (0.0.0.0/0, ::/0 → InvalidIpCidr naming the value), exclude_route_accepts_prefix (10.0.0.0/8, fd00::/64); tun.rs tun_config_refuses_more_than_256_exclude_routes (256 ok, 257 → TooManyExcludedRoutes(257)); existing bad_exclude assertion still InvalidIpCidr",
        "validation.rs: validate_exclude_route = validate_ip_cidr then refuse prefix_len 0 (ipnet::IpNet::prefix_len); add TooManyExcludedRoutes variant; export via models (pub use validation::* already)",
        "tun.rs TunConfig::validate: count check against MAX_EXCLUDE_ROUTES before the per-route loop, loop uses validate_exclude_route",
        "ui preferences/tun.rs excluded-route add dialog: validate_exclude_route instead of validate_ip_cidr (import update); address rows unchanged"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "x4",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "shard": "app",
      "pkgDirs": [
        "crates/core/src/models",
        "crates/ui/src/preferences"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/core/src/models/validation.rs",
          "symbol": "ValidationError::TooManyExcludedRoutes (new)",
          "anchor": "    InvalidTunMtu(u16),",
          "change": "new variant with #[error(\"too many excluded routes: {0} (at most {max})\", max = MAX_EXCLUDE_ROUTES)]; import MAX_EXCLUDE_ROUTES from the tun model"
        },
        {
          "task": "2.3",
          "file": "crates/core/src/models/validation.rs",
          "symbol": "validate_exclude_route (new)",
          "anchor": "        .map_err(|_| ValidationError::InvalidIpCidr(cidr.to_string()))",
          "change": "new pub fn next to validate_ip_cidr: parse as IpNet, refuse prefix_len 0 with InvalidIpCidr(value)"
        },
        {
          "task": "2.3",
          "file": "crates/core/src/models/tun.rs",
          "symbol": "TunConfig::validate",
          "anchor": "            validate_ip_cidr(route)?;",
          "change": "count check against MAX_EXCLUDE_ROUTES, then validate_exclude_route per route"
        },
        {
          "task": "2.3",
          "file": "crates/core/src/models/tun.rs",
          "symbol": "tests",
          "anchor": "        let bad_exclude = TunConfig {",
          "change": "add tun_config_refuses_more_than_256_exclude_routes next to it"
        },
        {
          "task": "2.3",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "excluded-route add dialog",
          "anchor": "                if validate_ip_cidr(&value).is_ok() {",
          "change": "use validate_exclude_route"
        }
      ],
      "verify": "make test-core && make test-ui && make clippy"
    },
    {
      "id": "x6",
      "taskIds": [
        "3.1",
        "4.1"
      ],
      "seam": "S5-ui-wording",
      "contract": {
        "states": [
          "xray-wording",
          "singbox-wording"
        ],
        "transitions": [
          {
            "input": "backend Xray at build or on settings change",
            "state": "xray-wording",
            "effect": "set",
            "evidence": "spec 'xray domain exclusion wording'"
          },
          {
            "input": "backend SingBox at build or on settings change",
            "state": "singbox-wording",
            "effect": "set",
            "evidence": "spec: sing-box text stays"
          },
          {
            "input": "backend V2ray",
            "state": "singbox-wording",
            "effect": "forced",
            "evidence": "groups insensitive for v2ray tun.rs:734-735; default arm"
          },
          {
            "input": "any backend, routes group",
            "state": "xray-wording",
            "effect": "no-op",
            "evidence": "spec 'Route exclusion wording': same text both backends"
          }
        ],
        "forbidden": [
          "routes description varying by backend",
          "xray domains text claiming traffic bypasses/stays off the tunnel",
          "description updated only at build and not on backend change"
        ],
        "seeding": [
          "call excluded_domains_description(BackendType::X) and read EXCLUDED_ROUTES_DESCRIPTION directly; no GTK in tests"
        ],
        "budgets": [
          "make test-ui: timeout 5m, --test-threads=4",
          "4.1: timeout 10m cargo test --workspace -- --test-threads=4"
        ],
        "names": [
          "const EXCLUDED_ROUTES_DESCRIPTION: &str = \"CIDRs routed outside the tunnel; applies on next connect\";",
          "fn excluded_domains_description(backend: BackendType) -> &'static str",
          "xray text: \"Matching traffic still passes through the tunnel and is sent directly by xray; applies on next connect\"",
          "sing-box/v2ray text: \"Domain suffixes that bypass the tunnel; applies on next connect\"",
          "tests: excluded_routes_description_names_next_connect, excluded_domains_description_per_backend"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "precondition: runs in the integration worktree only after both shard chains (netctl x1–x3 and app x4–x5) are merged into it",
        "3.1 tests first in preferences/tun.rs mod tests; then const + fn, use in builders, add domains_group.set_description(Some(excluded_domains_description(backend))) in subscribe_settings closure",
        "4.1 verify: timeout 10m cargo test --workspace -- --test-threads=4",
        "docs: docs/ARCHITECTURE.md netctl paragraph and CLAUDE.md `crates/netctl` section gain `--exclude` on `xray-up` (pref 8997 rules to main, replaced on every xray-up, removed by xray-down/recover); verify each claim against the code before writing",
        "CHANGELOG.md [Unreleased]: Fixed — xray TUN excluded routes now bypass the tunnel device (including DNS capture); Changed — excluded routes with prefix length 0 or lists over 256 entries are refused (a list above 256 must be trimmed in settings.toml)"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "x5",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "shard": "",
      "pkgDirs": [
        "crates/ui/src/preferences"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "routes_group",
          "anchor": "        .description(\"CIDRs that bypass the tunnel\")",
          "change": "description -> \"CIDRs routed outside the tunnel; applies on next connect\""
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "domains_group",
          "anchor": "        .description(\"Domain suffixes that bypass the tunnel\")",
          "change": "description from new pure helper keyed by backend (initial backend var `backend`)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "subscribe_settings backend gating closure",
          "anchor": "            domains_group.set_sensitive(backend != BackendType::V2ray);",
          "change": "first occurrence (inside subscribe_settings closure): add domains_group.set_description(helper(backend))"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "new pure helper + test",
          "anchor": "fn strict_route_applies(backend: BackendType) -> bool {",
          "change": "new fn returning &'static str per backend next to strict_route_applies; unit test next to strict_route_row_sensitive_for_xray"
        }
      ],
      "verify": "make test-ui && make clippy && timeout 10m cargo test --workspace -- --test-threads=4"
    }
  ],
  "seams": [
    {
      "id": "S1-netctl-validate",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Exclusion validation in crates/netctl/src/validate.rs on top of parse_cidr (validate.rs:29-44, which accepts prefix 0 and host bits). Pure, no netlink. v2ray-rs-netctl is a binary crate with private modules: items added in x1/x2 stay uncalled until x3 wires them, so x1/x2 verify with cargo test only and the clippy gate runs in x3; no #[allow(dead_code)] placeholders.",
      "contract": {
        "states": [
          "accepted-normalized",
          "refused"
        ],
        "transitions": [
          {
            "input": "\"10.15.12.100/32\"",
            "state": "accepted-normalized",
            "effect": "set",
            "evidence": "tasks.md 1.1; returns (10.15.12.100, 32) unchanged"
          },
          {
            "input": "\"10.1.2.3/8\" (host bits set, v4)",
            "state": "accepted-normalized",
            "effect": "forced",
            "evidence": "tasks.md 1.1; returns (10.0.0.0, 8)"
          },
          {
            "input": "\"fd00::1/64\" (host bits set, v6)",
            "state": "accepted-normalized",
            "effect": "forced",
            "evidence": "tasks.md 1.1; returns (fd00::, 64)"
          },
          {
            "input": "\"0.0.0.0/0\" or \"::/0\" or any addr with prefix 0",
            "state": "refused",
            "effect": "set",
            "evidence": "design.md Decisions 'Validation in the helper'; spec tun-mode prefix 1..=32/128"
          },
          {
            "input": "\"1.2.3.4/33\" or \"::1/129\"",
            "state": "refused",
            "effect": "set",
            "evidence": "parse_cidr validate.rs:39-42"
          },
          {
            "input": "non-CIDR (\"garbage\", \"1.2.3.4\", \"1.2.3.4/x\")",
            "state": "refused",
            "effect": "set",
            "evidence": "parse_cidr validate.rs:30-38; spec scenario 'Invalid exclusion refused'"
          },
          {
            "input": "count 0..=256",
            "state": "accepted-normalized",
            "effect": "no-op",
            "evidence": "check_exclusion_count Ok"
          },
          {
            "input": "count 257",
            "state": "refused",
            "effect": "set",
            "evidence": "tasks.md 1.1 '257 values refused'"
          }
        ],
        "forbidden": [
          "accepted value with prefix 0 in either family",
          "accepted value whose host bits are non-zero",
          "host-bit mask computed before the prefix-0 refusal (u32 << 32 / u128 << 128 overflow)",
          "error text that does not contain the offending value (Debug-quoted, like parse_cidr)"
        ],
        "seeding": [
          "accepted-normalized: call validate::parse_exclusion(&str) directly",
          "refused: call validate::parse_exclusion with a bad value, or validate::check_exclusion_count(257)"
        ],
        "budgets": [
          "MAX_EXCLUSIONS = 256 values",
          "IPv4 prefix 1..=32",
          "IPv6 prefix 1..=128"
        ],
        "names": [
          "pub const MAX_EXCLUSIONS: usize = 256;",
          "pub fn parse_exclusion(cidr: &str) -> Result<(IpAddr, u8), String>",
          "pub fn check_exclusion_count(count: usize) -> Result<(), String>",
          "prefix-0 error: format!(\"exclusion prefix length 0 refused: {cidr:?}\")",
          "count error: format!(\"too many exclusions: {count} (max {MAX_EXCLUSIONS})\")",
          "reused parse_cidr errors: \"invalid cidr (missing prefix): {cidr:?}\", \"invalid cidr address: {cidr:?}\", \"invalid cidr prefix: {cidr:?}\", \"cidr prefix out of range: {cidr:?}\"",
          "tests: exclusion_accepts_host_route, exclusion_clears_host_bits, exclusion_refuses_default_prefix, exclusion_refuses_out_of_range_prefix, exclusion_count_capped_at_256"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1 tests first in validate.rs mod tests (names above; v4+v6 normalization, both /0 refused, /33 refused, check_exclusion_count(256) Ok and (257) Err with text containing \"257\")",
        "1.1 impl: parse_exclusion = parse_cidr -> refuse prefix 0 -> mask host bits (v4 via u32, v6 via u128); check_exclusion_count",
        "MAX_EXCLUSIONS gets a comment that the app enforces the same cap (MAX_EXCLUDE_ROUTES in crates/core/src/models/tun.rs)"
      ]
    },
    {
      "id": "S2-netctl-cli",
      "tasks": [
        "1.2",
        "1.5"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. XrayUp gains repeatable --exclude; every value validated by clap value_parser at parse time and the count checked in run() before net::connect() (main.rs:94). Refusal layer: clap parse (per value, process exits 2 with clap error naming the value) and run() (count, exits 1 with 'netctl: ...' main.rs:68).",
      "contract": {
        "states": [
          "parsed",
          "parse-refused",
          "count-refused"
        ],
        "transitions": [
          {
            "input": "no --exclude",
            "state": "parsed",
            "effect": "no-op",
            "evidence": "exclude is empty Vec; replace still runs and clears prior 8997 rules (S3)"
          },
          {
            "input": "--exclude 10.0.0.0/8 --exclude 192.0.2.0/24",
            "state": "parsed",
            "effect": "set",
            "evidence": "tasks.md 1.2; Vec in argument order"
          },
          {
            "input": "--exclude 10.0.0.0/33 or --exclude garbage",
            "state": "parse-refused",
            "effect": "set",
            "evidence": "spec scenario 'Invalid exclusion refused'; Cli::try_parse_from Err, message contains the value"
          },
          {
            "input": "257 valid --exclude values",
            "state": "count-refused",
            "effect": "set",
            "evidence": "run(): validate::check_exclusion_count before net::connect()"
          },
          {
            "input": "valid values but iface not a TUN device",
            "state": "count-refused",
            "effect": "no-op",
            "evidence": "existing refusal main.rs:89-91 still precedes connect; no netlink change"
          }
        ],
        "forbidden": [
          "net::connect() reached before check_exclusion_count and all parse_exclusion calls complete",
          "an invalid value silently dropped instead of failing the command"
        ],
        "seeding": [
          "parsed / parse-refused: Cli::try_parse_from([... \"xray-up\", \"--iface\", \"xtun0\", \"--addr\", \"172.19.0.1/30\", \"--exclude\", v]) using the existing parse() helper pattern main.rs:119-130 (try variant for refusal)",
          "count-refused: not unit-reachable without netlink order; covered by S1 check_exclusion_count test plus code review of run() ordering"
        ],
        "budgets": [
          "MAX_EXCLUSIONS = 256",
          "unit run: timeout 5m, --test-threads=4"
        ],
        "names": [
          "CLI flag: --exclude <CIDR> (repeatable)",
          "field: Command::XrayUp { exclude: Vec<(IpAddr, u8)>, .. } with #[arg(long, value_parser = validate::parse_exclusion)]",
          "tests: xray_up_parses_repeated_exclude, xray_up_refuses_invalid_exclude_naming_it"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.2 tests first in main.rs mod tests next to xray_up_parses_strict_flag: two values parse in order to normalized tuples; try_parse_from with 10.0.0.0/33 is Err and err.to_string() contains \"10.0.0.0/33\"",
        "1.2 impl: add field + doc comment (value_parser = validate::parse_exclusion); in run() validate::check_exclusion_count(exclude.len())? after parse_cidr(addr/addr6) and before net::connect(); call net::xray_up(...) unchanged, then net::replace_exclusions(&handle, &exclude).await? only after it succeeds",
        "1.5 verify: timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 green"
      ]
    },
    {
      "id": "S3-netctl-rules",
      "tasks": [
        "1.3",
        "1.4"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Pref-8997 destination rules to table main in crates/netctl/src/net.rs. Replace semantics in a separate pub fn net::replace_exclusions because adding an 8th parameter to xray_up (net.rs:72-80, 7 params today) trips clippy::too_many_arguments under -D warnings (no clippy.toml, no allow). Rule shape mirrors add_bypass_uid_rule (net.rs:357-382): header.family by addr, header.action ToTable, header.table = RT_TABLE_MAIN as u8, header.dst_len = prefix, RuleAttribute::Destination(ip) (netlink-packet-route 0.30.0 rule/attribute.rs:45), RuleAttribute::Priority(RULE_PREF_EXCLUDE). Teardown via is_xray_rule list (net.rs:397-412). v2ray-rs-netctl is a binary crate with private modules: items added in x1/x2 stay uncalled until x3 wires them, so x1/x2 verify with cargo test only and the clippy gate runs in x3; no #[allow(dead_code)] placeholders.",
      "contract": {
        "states": [
          "no-8997-rules",
          "8997-rules-v4",
          "8997-rules-v6"
        ],
        "transitions": [
          {
            "input": "xray-up with N valid v4 exclusions, none installed",
            "state": "8997-rules-v4",
            "effect": "set",
            "evidence": "spec 'Bring xray TUN routes up'; one rule per CIDR"
          },
          {
            "input": "xray-up with v6 exclusions, /proc/sys/net/ipv6 present",
            "state": "8997-rules-v6",
            "effect": "set",
            "evidence": "host_has_ipv6 net.rs:137-139"
          },
          {
            "input": "xray-up with v6 exclusion, host has IPv6 disabled",
            "state": "8997-rules-v6",
            "effect": "no-op",
            "evidence": "design.md 'IPv6 exclusions ... skipped'; exclusion_installable returns false"
          },
          {
            "input": "xray-up with list M after prior list",
            "state": "8997-rules-v4",
            "effect": "forced",
            "evidence": "spec 'Exclusions replaced on re-run': delete all pref-8997 both families, then add M"
          },
          {
            "input": "xray-up with no --exclude after prior list",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "replace runs with empty list"
          },
          {
            "input": "two values equal after normalization (10.1.2.3/8, 10.0.0.0/8)",
            "state": "8997-rules-v4",
            "effect": "no-op",
            "evidence": "EEXIST tolerated, is_exists net.rs:456-458; one rule"
          },
          {
            "input": "--capture-dns with --exclude 10.15.12.100/32; unmarked udp to :53",
            "state": "8997-rules-v4",
            "effect": "set",
            "evidence": "spec 'Excluded resolver skips DNS capture'; 8997 < 8999 (net.rs:37)"
          },
          {
            "input": "xray-down",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "del_xray_rules net.rs:386-395 with 8997 added to is_xray_rule"
          },
          {
            "input": "recover --xray",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "recover_xray -> xray_down net.rs:170-174"
          },
          {
            "input": "recover --singbox",
            "state": "no-8997-rules",
            "effect": "clear",
            "evidence": "recover_singbox -> xray_down net.rs:178-179"
          },
          {
            "input": "non-EEXIST netlink error on add",
            "state": "8997-rules-v4",
            "effect": "forced",
            "evidence": "returns Err -> xray-up exits 1 -> ProcessManager stops child with TunHelper, manager.rs:667-673"
          }
        ],
        "forbidden": [
          "pref-8997 rule with any table other than main (254)",
          "pref-8997 rule with dst_len 0",
          "pref-8997 rule carrying fwmark, suppress_prefixlength, uidrange or port match",
          "more than 256 pref-8997 rules installed by one call",
          "pref-8997 rule surviving xray-down or recover",
          "rule from an earlier xray-up surviving a later xray-up whose list omits it",
          "IPv6 rule add attempted when host_has_ipv6() is false"
        ],
        "seeding": [
          "exclusion_installable: call the pure fn directly with (IpAddr, bool)",
          "no-8997-rules / 8997-rules-v4 / 8997-rules-v6: only via the netctl binary inside a namespace (netctl_in, privileged.rs:77-81); never `ip rule add pref 8997` by hand",
          "main default via physical-iface stand-in: MAIN_IFACE tuntap + addr + link up + `route add default dev MAIN_IFACE`, pattern privileged.rs:540-552"
        ],
        "budgets": [
          "pref 8997",
          "table main = 254 (RT_TABLE_MAIN net.rs:26)",
          "<= 256 rules per family per call",
          "helper call bounded by HELPER_TIMEOUT 10s (process tun.rs:15)",
          "privileged run: sudo -E timeout 10m, --test-threads=1"
        ],
        "names": [
          "const RULE_PREF_EXCLUDE: u32 = 8997;",
          "pub async fn replace_exclusions(handle: &Handle, exclude: &[(IpAddr, u8)]) -> Result<(), String>",
          "fn exclusion_installable(ip: IpAddr, host_has_ipv6: bool) -> bool  (true for v4; host_has_ipv6 for v6)",
          "async fn del_rules_with_priority(handle: &Handle, priority: u32)  (both IpVersion, errors ignored like del_xray_rules)",
          "add error: format!(\"add exclusion rule {ip}/{prefix} (pref {RULE_PREF_EXCLUDE}): {e}\")",
          "unit test: exclusion_installable_skips_v6_without_host_ipv6",
          "privileged tests: exclusions_route_outside_tunnel_ahead_of_dns_capture, exclusions_replaced_on_reup_and_cleared_on_teardown; namespaces NS_EXCL = \"nctl-excl-ns\", NS_EXCL_REUP = \"nctl-exclreup-ns\"",
          "assert_xray_state_cleared pref list (privileged.rs:392) gains \"8997:\""
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.3 unit tests first in net.rs mod tests: exclusion_installable_skips_v6_without_host_ipv6 (table over v4/v6 x host true/false); xray_rule_prefs_cover_exclusions (XRAY_RULE_PREFS contains RULE_PREF_EXCLUDE)",
        "1.3 impl: RULE_PREF_EXCLUDE = 8997, XRAY_RULE_PREFS const used by is_xray_rule, exclusion_installable, del_rules_with_priority, add_exclusion_rule, replace_exclusions (delete all 8997 both families, then add each installable entry in order; only ever table main, dst_len ≥ 1); update RULE_PREF docs; xray_up signature unchanged",
        "1.4 privileged tests (compile under clippy --all-features, file gated by #![cfg(feature = \"privileged-tests\")] privileged.rs:6): (a) up --capture-dns with --exclude 198.51.100.7/32 --exclude 192.0.2.53/32 and MAIN_IFACE default: two lines starting \"8997:\" containing \"lookup main\"; `ip route get 198.51.100.7` shows dev MAIN_IFACE; `ip route get 192.0.2.53 ipproto udp dport 53` shows dev MAIN_IFACE; 8997 line precedes 8999 line; (b) up with two, re-up with one -> exactly one 8997 line for the kept prefix; up without --exclude -> none; xray-down -> none; up again then recover --xray -> none; up again then recover --singbox -> no 8997 line (or, if recover --singbox does not touch xray rules at HEAD, assert and note that behavior unchanged); run the sudo command"
      ]
    },
    {
      "id": "S4-process-ui-plumbing",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. TunRuntime.exclude_routes (process tun.rs:31-45) emitted by xray_up_args (tun.rs:229-248) and filled by build_tun_runtime (ui connection.rs:822-850) from effective_settings (connection.rs:302). This seam fixes every TunRuntime literal: process tun.rs:432, 447 (481 uses ..xray_rt); manager.rs:1623, 1844, 1880, 1957, 2104, 2138, 2345, 2385; ui connection.rs:834 (prod), 2272 (test). Both tasks in one seam so the ui prod literal gets the real value, not a placeholder.",
      "contract": {
        "states": [
          "no-exclude-args",
          "exclude-args"
        ],
        "transitions": [
          {
            "input": "TunRuntime.exclude_routes empty",
            "state": "no-exclude-args",
            "effect": "no-op",
            "evidence": "spec scenario 'No exclusions'"
          },
          {
            "input": "exclude_routes [\"10.15.12.100/32\", \"91.230.107.224/32\"]",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "spec 'Excluded routes reach the helper'; pairs appended after --strict, in order"
          },
          {
            "input": "build_tun_runtime, backend Xray, tun.enabled, exclude_routes non-empty",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "tasks.md 2.2; clone of settings.tun.exclude_routes"
          },
          {
            "input": "build_tun_runtime, backend SingBox",
            "state": "no-exclude-args",
            "effect": "forced",
            "evidence": "spec: sing-box uses route_exclude_address (singbox.rs:197-199); needs_helper false tun.rs:50-52"
          },
          {
            "input": "build_tun_runtime given effective settings whose exclude_routes differ from persisted",
            "state": "exclude-args",
            "effect": "set",
            "evidence": "tasks.md 2.2; fn reads only its settings argument"
          },
          {
            "input": "tun disabled or backend V2ray",
            "state": "no-exclude-args",
            "effect": "no-op",
            "evidence": "returns None connection.rs:823-829"
          }
        ],
        "forbidden": [
          "sing-box runtime with non-empty exclude_routes",
          "xray_up_args reordering or deduplicating exclude_routes",
          "--exclude emitted without a following value, or joined as one comma string",
          "exclude_routes read from anything but the settings argument (persisted settings)"
        ],
        "seeding": [
          "xray_up_args: construct TunRuntime via xray_rt(strict) fixture (tun.rs:446-457) with struct update ..",
          "build_tun_runtime: tun_settings() (connection.rs:2842-2850) then set backend.backend_type and tun.exclude_routes; 'effective differs' = clone persisted, mutate clone's exclude_routes, pass clone"
        ],
        "budgets": [
          "make test-process / make test-ui: timeout 5m, --test-threads=4"
        ],
        "names": [
          "pub exclude_routes: Vec<String> on TunRuntime (after strict)",
          "argument pair: \"--exclude\", <cidr>",
          "xray_rt fixture gains exclude_routes: vec![\"10.15.12.100/32\".into()] and xray_up_args_include_strict_when_set expects \"--exclude\", \"10.15.12.100/32\" after \"--strict\"; xray_up_args_omit_strict_when_off sets exclude_routes: Vec::new()",
          "tests: xray_up_args_pass_exclusions_in_order (process); xray_runtime_carries_exclude_routes, singbox_runtime_has_no_exclude_routes, runtime_uses_effective_exclude_routes (ui connection.rs mod tests)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1 tests first: extend the two xray_up_args fixtures, add xray_up_args_pass_exclusions_in_order; then add the field, emit pairs in xray_up_args, add exclude_routes: Vec::new() to every literal listed in summary (process + ui test literal); xray_up_args_omit_strict_when_off (uses ..xray_rt) must also set exclude_routes: Vec::new() or its assertion fails",
        "2.2 tests first in connection.rs: three tests named above; then build_tun_runtime sets exclude_routes: if backend == BackendType::Xray { settings.tun.exclude_routes.clone() } else { Vec::new() }"
      ]
    },
    {
      "id": "S5-ui-wording",
      "tasks": [
        "3.1",
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Group descriptions in crates/ui/src/preferences/tun.rs: routes_group (tun.rs:157-160) and domains_group (tun.rs:255-258); refresh in the subscribe_settings closure (tun.rs:723-738) via set_description. Owns the workspace floor 4.1.",
      "contract": {
        "states": [
          "xray-wording",
          "singbox-wording"
        ],
        "transitions": [
          {
            "input": "backend Xray at build or on settings change",
            "state": "xray-wording",
            "effect": "set",
            "evidence": "spec 'xray domain exclusion wording'"
          },
          {
            "input": "backend SingBox at build or on settings change",
            "state": "singbox-wording",
            "effect": "set",
            "evidence": "spec: sing-box text stays"
          },
          {
            "input": "backend V2ray",
            "state": "singbox-wording",
            "effect": "forced",
            "evidence": "groups insensitive for v2ray tun.rs:734-735; default arm"
          },
          {
            "input": "any backend, routes group",
            "state": "xray-wording",
            "effect": "no-op",
            "evidence": "spec 'Route exclusion wording': same text both backends"
          }
        ],
        "forbidden": [
          "routes description varying by backend",
          "xray domains text claiming traffic bypasses/stays off the tunnel",
          "description updated only at build and not on backend change"
        ],
        "seeding": [
          "call excluded_domains_description(BackendType::X) and read EXCLUDED_ROUTES_DESCRIPTION directly; no GTK in tests"
        ],
        "budgets": [
          "make test-ui: timeout 5m, --test-threads=4",
          "4.1: timeout 10m cargo test --workspace -- --test-threads=4"
        ],
        "names": [
          "const EXCLUDED_ROUTES_DESCRIPTION: &str = \"CIDRs routed outside the tunnel; applies on next connect\";",
          "fn excluded_domains_description(backend: BackendType) -> &'static str",
          "xray text: \"Matching traffic still passes through the tunnel and is sent directly by xray; applies on next connect\"",
          "sing-box/v2ray text: \"Domain suffixes that bypass the tunnel; applies on next connect\"",
          "tests: excluded_routes_description_names_next_connect, excluded_domains_description_per_backend"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "precondition: runs in the integration worktree only after both shard chains (netctl x1–x3 and app x4–x5) are merged into it",
        "3.1 tests first in preferences/tun.rs mod tests; then const + fn, use in builders, add domains_group.set_description(Some(excluded_domains_description(backend))) in subscribe_settings closure",
        "4.1 verify: timeout 10m cargo test --workspace -- --test-threads=4",
        "docs: docs/ARCHITECTURE.md netctl paragraph and CLAUDE.md `crates/netctl` section gain `--exclude` on `xray-up` (pref 8997 rules to main, replaced on every xray-up, removed by xray-down/recover); verify each claim against the code before writing",
        "CHANGELOG.md [Unreleased]: Fixed — xray TUN excluded routes now bypass the tunnel device (including DNS capture); Changed — excluded routes with prefix length 0 or lists over 256 entries are refused (a list above 256 must be trimmed in settings.toml)"
      ]
    },
    {
      "id": "S6-core-validation",
      "tasks": [
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. App-side exclusion validation matching the helper: validate_exclude_route in crates/core/src/models/validation.rs, used by TunConfig::validate (crates/core/src/models/tun.rs) and by the excluded-route add dialog in crates/ui/src/preferences/tun.rs.",
      "contract": {
        "states": [
          "route-accepted",
          "route-refused",
          "count-refused"
        ],
        "transitions": [
          {
            "input": "validate_exclude_route(\"10.0.0.0/8\")",
            "state": "route-accepted",
            "effect": "set",
            "evidence": "tasks.md 2.3"
          },
          {
            "input": "validate_exclude_route(\"0.0.0.0/0\") or (\"::/0\")",
            "state": "route-refused",
            "effect": "set",
            "evidence": "spec tun-mode Scenario: Whole-address-space exclusion is refused"
          },
          {
            "input": "validate_exclude_route(\"bogus\") (not a CIDR)",
            "state": "route-refused",
            "effect": "set",
            "evidence": "existing validate_ip_cidr behavior"
          },
          {
            "input": "TunConfig with 257 exclude_routes",
            "state": "count-refused",
            "effect": "set",
            "evidence": "spec tun-mode Scenario: Too many exclusions are refused"
          },
          {
            "input": "TunConfig with 256 valid exclude_routes",
            "state": "route-accepted",
            "effect": "no-op",
            "evidence": "cap is inclusive of 256"
          },
          {
            "input": "add dialog value refused by validate_exclude_route",
            "state": "route-refused",
            "effect": "no-op",
            "evidence": "preferences/tun.rs add dialog keeps the existing silent-ignore on invalid input; nothing is pushed"
          }
        ],
        "forbidden": [
          "a prefix-0 route persisted through the add dialog",
          "address_v4/address_v6 validation changing (they keep validate_ip_cidr)",
          "a new UI control or error surface"
        ],
        "seeding": [
          "plain values: TunConfig { exclude_routes: vec![..], ..TunConfig::default() }; (0..257).map(|i| format!(\"10.{}.{}.0/24\", i / 256, i % 256))"
        ],
        "budgets": [
          "MAX_EXCLUDE_ROUTES = 256 (same number as the helper cap)",
          "prefix length ≥ 1 for both families"
        ],
        "names": [
          "pub fn validate_exclude_route(cidr: &str) -> Result<(), ValidationError>",
          "prefix 0 → ValidationError::InvalidIpCidr(value) (existing variant, names the value)",
          "new variant ValidationError::TooManyExcludedRoutes(usize) with #[error(\"too many excluded routes: {0} (at most {max})\", max = MAX_EXCLUDE_ROUTES)]",
          "pub const MAX_EXCLUDE_ROUTES: usize = 256 in crates/core/src/models/tun.rs, with a comment that the route helper enforces the same cap (MAX_EXCLUSIONS in crates/netctl/src/validate.rs); x1 adds the reverse pointer on MAX_EXCLUSIONS",
          "tests: exclude_route_refuses_whole_address_space, exclude_route_accepts_prefix, tun_config_refuses_more_than_256_exclude_routes"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: validation.rs exclude_route_refuses_whole_address_space (0.0.0.0/0, ::/0 → InvalidIpCidr naming the value), exclude_route_accepts_prefix (10.0.0.0/8, fd00::/64); tun.rs tun_config_refuses_more_than_256_exclude_routes (256 ok, 257 → TooManyExcludedRoutes(257)); existing bad_exclude assertion still InvalidIpCidr",
        "validation.rs: validate_exclude_route = validate_ip_cidr then refuse prefix_len 0 (ipnet::IpNet::prefix_len); add TooManyExcludedRoutes variant; export via models (pub use validation::* already)",
        "tun.rs TunConfig::validate: count check against MAX_EXCLUDE_ROUTES before the per-route loop, loop uses validate_exclude_route",
        "ui preferences/tun.rs excluded-route add dialog: validate_exclude_route instead of validate_ip_cidr (import update); address rows unchanged"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The excluded-routes and excluded-domains groups SHALL describe what each list does on the active backend. Excluded routes SHALL be described as destinations routed outside the tunnel on both backends. For xray, excluded domains SHALL be described as traffic that still enters the tunnel and is sent directly by xray after it recognizes the domain, so they do not keep traffic off the TUN device; for sing-box, the description SHALL describe domain suffixes that bypass the tunnel. Both groups SHALL state that changes apply on the next connect.",
      "tests": [
        "excluded_routes_description_names_next_connect",
        "excluded_domains_description_per_backend"
      ]
    },
    {
      "shall": "- **THEN** the excluded-domains description SHALL state that matching traffic still passes through the tunnel and is sent directly by xray",
      "tests": [
        "excluded_domains_description_per_backend"
      ]
    },
    {
      "shall": "- **THEN** the excluded-routes description SHALL state that the CIDRs are routed outside the tunnel and that changes apply on the next connect",
      "tests": [
        "excluded_routes_description_names_next_connect"
      ]
    },
    {
      "shall": "The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998; with `--capture-dns`, unmarked udp and tcp traffic to port 53 looks up the TUN table (pref 8999), so a resolver on the local subnet is reached through the tunnel rather than the LAN route pref 9001 preserves. With one or more `--exclude <CIDR>` arguments, `xray-up` SHALL install, for each CIDR, a rule sending traffic destined to that prefix to `main` at pref 8997, ahead of DNS capture and the tunnel table, so excluded destinations — including resolvers on port 53 — never enter the TUN device; the pref 8997 rule set SHALL be replaced on every `xray-up`, so exclusions removed since the previous run do not linger. Each `--exclude` value SHALL be validated as an IPv4 or IPv6 CIDR before any netlink call, host bits SHALL be cleared, the number of values SHALL be bounded, and IPv6 exclusions SHALL be skipped when the host has IPv6 disabled. IPv6 equivalents SHALL be installed when an IPv6 address is supplied. With `--strict`, `xray-up` SHALL additionally install an `unreachable` default route with the lowest priority in table 2023 for IPv4 and IPv6, and SHALL install the IPv6 policy rules even when no IPv6 address is supplied — unless the host has IPv6 disabled, in which case the IPv6 fallback route and rules are skipped — so that traffic destined for the tunnel is refused rather than routed through the real default route whenever the TUN device is absent. Re-running `xray-up` against a recreated device of the same name SHALL succeed and leave exactly one copy of each route and rule.",
      "tests": [
        "exclusions_route_outside_tunnel_ahead_of_dns_capture",
        "exclusions_replaced_on_reup_and_cleared_on_teardown",
        "exclusion_refuses_default_prefix",
        "xray_up_refuses_invalid_exclude_naming_it",
        "up_down_is_idempotent_in_namespace",
        "exclusion_clears_host_bits",
        "exclusion_installable_skips_v6_without_host_ipv6",
        "xray_rule_prefs_cover_exclusions"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given, the pref 8999 port-53 rules when `--capture-dns` is given, and one pref 8997 rule per `--exclude` CIDR), each step idempotent",
      "tests": [
        "up_down_is_idempotent_in_namespace",
        "exclusions_route_outside_tunnel_ahead_of_dns_capture"
      ]
    },
    {
      "shall": "- **THEN** they SHALL match only unmarked traffic, so the backend's own resolver queries keep egressing the real interface through the pref 9000 rule",
      "tests": [
        "capture_dns_steers_port_53_into_the_tunnel_table"
      ]
    },
    {
      "shall": "- **THEN** `ip route get 91.230.107.224` for an unmarked packet SHALL resolve through `main` to the physical interface, not the TUN device",
      "tests": [
        "exclusions_route_outside_tunnel_ahead_of_dns_capture"
      ]
    },
    {
      "shall": "- **THEN** unmarked udp traffic to `10.15.12.100:53` SHALL match the pref 8997 rule before the pref 8999 capture rule and SHALL NOT enter the TUN device",
      "tests": [
        "exclusions_route_outside_tunnel_ahead_of_dns_capture"
      ]
    },
    {
      "shall": "- **THEN** exactly one pref 8997 rule SHALL exist, for `10.0.0.0/8`",
      "tests": [
        "exclusions_replaced_on_reup_and_cleared_on_teardown"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL exit with an error naming the value and SHALL make no netlink change",
      "tests": [
        "xray_up_refuses_invalid_exclude_naming_it",
        "exclusion_refuses_default_prefix",
        "exclusion_refuses_out_of_range_prefix",
        "exclusion_count_capped_at_256"
      ]
    },
    {
      "shall": "- **THEN** table 2023 SHALL contain an `unreachable` default route for IPv4 and for IPv6 at a lower priority than the device route, and the IPv6 pref 9000/9001/9002 rules SHALL be present whether or not an IPv6 address was supplied, on any host with IPv6 enabled",
      "tests": [
        "strict_up_installs_fallback_routes_and_v6_rules"
      ]
    },
    {
      "shall": "- **THEN** unmarked traffic to a destination outside the on-link routes SHALL be refused with host-unreachable rather than sent through the real default route, while fwmark-255 and bypass-uid traffic SHALL keep using `main`",
      "tests": [
        "strict_state_refuses_unmarked_traffic_without_device"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL succeed and the resulting routes and rules SHALL match a single fresh `xray-up`",
      "tests": [
        "reup_across_recreated_device_leaves_one_copy",
        "exclusions_replaced_on_reup_and_cleared_on_teardown"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences, including pref 8997) for both address families, remove the table-2023 fallback routes, delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when all are already absent",
      "tests": [
        "down_and_recover_clear_strict_state_both_families",
        "exclusions_replaced_on_reup_and_cleared_on_teardown",
        "xray_rule_prefs_cover_exclusions"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray) including the fallback routes, and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean",
      "tests": [
        "down_and_recover_clear_strict_state_both_families",
        "exclusions_replaced_on_reup_and_cleared_on_teardown"
      ]
    },
    {
      "shall": "For an xray TUN connection, the system SHALL pass every entry of the effective `tun.exclude_routes` to the route helper as an exclusion, built from the same effective settings the config was generated from. The generated xray routing rule sending those CIDRs to `direct` SHALL remain. sing-box connections SHALL keep expressing exclusions through `route_exclude_address` and SHALL NOT invoke the helper for them. Each excluded route SHALL be refused by settings validation when its prefix length is 0, and more than 256 excluded routes SHALL be refused, so the TUN preferences reject such an entry before it is saved and config generation fails with an error naming the value instead of the helper refusing it at connect.",
      "tests": [
        "xray_runtime_carries_exclude_routes",
        "singbox_runtime_has_no_exclude_routes",
        "runtime_uses_effective_exclude_routes",
        "xray_up_args_pass_exclusions_in_order",
        "exclude_route_refuses_whole_address_space",
        "tun_config_refuses_more_than_256_exclude_routes"
      ]
    },
    {
      "shall": "- **THEN** the route helper SHALL be invoked with `--exclude 10.15.12.100/32 --exclude 91.230.107.224/32`",
      "tests": [
        "xray_up_args_pass_exclusions_in_order",
        "xray_runtime_carries_exclude_routes"
      ]
    },
    {
      "shall": "- **THEN** the route helper SHALL be invoked without `--exclude`",
      "tests": [
        "xray_up_args_omit_strict_when_off",
        "singbox_runtime_has_no_exclude_routes"
      ]
    },
    {
      "shall": "- **THEN** the TUN preferences SHALL reject the entry, and settings carrying it SHALL fail validation with an error naming the value",
      "tests": [
        "exclude_route_refuses_whole_address_space"
      ]
    },
    {
      "shall": "- **THEN** settings validation SHALL fail",
      "tests": [
        "tun_config_refuses_more_than_256_exclude_routes"
      ]
    }
  ],
  "testHarness": [
    "parse — crates/netctl/src/main.rs:    fn parse(extra: &[&str]) -> Cli { — Cli from xray-up --iface xtun0 --addr 172.19.0.1/30 + extra args",
    "run — crates/netctl/tests/privileged.rs:fn run(cmd: &str, args: &[&str]) -> bool { — runs a command, success bool",
    "ip_in — crates/netctl/tests/privileged.rs:fn ip_in(ns: &str, args: &[&str]) -> bool { — ip netns exec <ns> ip ..., bool",
    "ip_in_ns — crates/netctl/tests/privileged.rs:fn ip_in_ns(args: &[&str]) -> bool { — ip_in on NS",
    "ip_in_output — crates/netctl/tests/privileged.rs:fn ip_in_output(ns: &str, args: &[&str]) -> String { — stdout of ip in ns",
    "ip_in_ns_output — crates/netctl/tests/privileged.rs:fn ip_in_ns_output(args: &[&str]) -> String { — ip_in_output on NS",
    "ip_in_full — crates/netctl/tests/privileged.rs:fn ip_in_full(ns: &str, args: &[&str]) -> (bool, String) { — success + stdout+stderr (for route get)",
    "netctl_in — crates/netctl/tests/privileged.rs:fn netctl_in(ns: &str, args: &[&str]) -> bool { — runs helper binary BIN inside ns",
    "netctl — crates/netctl/tests/privileged.rs:fn netctl(args: &[&str]) -> bool { — netctl_in on NS",
    "device_exists — crates/netctl/tests/privileged.rs:fn device_exists() -> bool { — IFACE present in NS",
    "NsGuard — crates/netctl/tests/privileged.rs:struct NsGuard(&'static str); — deletes namespace on drop",
    "fallback_lines — crates/netctl/tests/privileged.rs:fn fallback_lines(ns: &str, family: &str) -> Vec<String> { — unreachable default lines of table 2023",
    "assert_xray_state_cleared — crates/netctl/tests/privileged.rs:fn assert_xray_state_cleared(ns: &str, after: &str) { — asserts empty table 2023 and no 8998-9002 rules both families",
    "xray_state — crates/netctl/tests/privileged.rs:fn xray_state(ns: &str) -> Vec<String> { — sorted rules + table-2023 routes both families",
    "consts — crates/netctl/tests/privileged.rs:const IFACE: &str = \"nctltest0\"; — IFACE nctltest0, MAIN_IFACE nctlmain0 (uplink stand-in), ADDR 172.31.255.1/30, ADDR6 fd00:ffff::1/64, NS/NS_DNS/NS_STRICT/NS_CLEAR/NS_REUP/NS_GONE",
    "uplink setup — crates/netctl/tests/privileged.rs:        &[\"route\", \"add\", \"default\", \"dev\", MAIN_IFACE] — MAIN_IFACE tun with 10.99.0.1/24 + main default route (inline in strict_state_refuses_unmarked_traffic_without_device, not a helper)",
    "t — crates/process/src/tun.rs:    fn t(secs: u64) -> std::time::SystemTime { — epoch + secs",
    "xray_rt — crates/process/src/tun.rs:    fn xray_rt(strict: bool) -> TunRuntime { — xray runtime tun0, v4+v6 addr, bypass_uid 967, capture_dns true",
    "write_helper — crates/process/src/tun.rs:    fn write_helper(dir: &Path, body: &str) -> PathBuf { — executable sh script netctl in dir",
    "xray_on_lo — crates/process/src/manager.rs:    fn xray_on_lo(helper: PathBuf) -> TunRuntime { — xray runtime on lo with given helper",
    "tun_settings — crates/ui/src/connection.rs:    fn tun_settings() -> AppSettings { — default settings with tun.enabled",
    "xray_tun_settings — crates/ui/src/connection.rs:    fn xray_tun_settings() -> AppSettings { — tun_settings with backend Xray",
    "v2ray_settings — crates/ui/src/connection.rs:    fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings { — v2ray backend with ports/listen/tun flag",
    "singbox_settings — crates/ui/src/connection.rs:    fn singbox_settings() -> AppSettings { — sing-box settings",
    "strict_route_settings — crates/ui/src/connection.rs:    fn strict_route_settings() -> AppSettings { — strict-route settings",
    "node — crates/ui/src/connection.rs:    fn node(address: &str) -> ProxyNode { — shadowsocks node at address",
    "candidate — crates/ui/src/connection.rs:    fn candidate(address: &str) -> ConnectionCandidate { — connection candidate at address",
    "preferences/tun.rs tests — crates/ui/src/preferences/tun.rs:    fn strict_route_row_sensitive_for_xray() { — only test; no fixtures"
  ],
  "risks": [
    "Privileged namespace tests (1.4) need root and /dev/net/tun; the floor cannot run them unattended. Mitigation: run the sudo command manually and record the result; the file is still compiled by make clippy (--all-features) so it must stay clippy-clean.",
    "Replace is delete-then-add; deletion errors are ignored (mirrors del_xray_rules), so a failed delete could leave a stale 8997 rule until xray-down. Mitigation: re-up test asserts exact count; generator ip->direct rule covers the gap.",
    "Pref 8997 matching in is_xray_rule deletes any foreign rule at that priority (same as existing reserved prefs). Mitigation: documented reserved range 8997-9002.",
    "`ip route get` against a tuntap device with no attached fd (NO-CARRIER) in the namespace: excluded address asserts dev MAIN_IFACE, so it does not depend on the TUN device resolving. Mitigation: assert on MAIN_IFACE only.",
    "docs/ARCHITECTURE.md and CLAUDE.md describe netctl xray-up flags; not in tasks. Mitigation: update the netctl paragraph (pref 8997 --exclude) and CHANGELOG [Unreleased] alongside 1.x.",
    "Privileged namespace tests (task 1.4) are compiled and clippy-checked by the floor but only run manually: sudo -E timeout 10m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=1",
    "Several helper SHALLs (8997 rules to main, route get via the physical interface, 8997 before 8999, replace on re-up, teardown by xray-down/recover) are proven only by the privileged namespace tests, which run manually with sudo; the run result is recorded before archiving.",
    "Settings saved before this change with a prefix-0 excluded route or more than 256 routes now fail validation: connect reports the value, and TUN preference edits are refused until the offending /0 entry is removed; a list above 256 cannot shrink through the UI one entry at a time (edit settings.toml). Rare; noted in CHANGELOG [Unreleased]."
  ],
  "floor": "make fmt && make clippy && make test TEST_TIMEOUT=10m. MANUAL: privileged namespace tests (sudo -E timeout 10m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=1); tasks 4.2/4.3 live checks on xray TUN.",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
