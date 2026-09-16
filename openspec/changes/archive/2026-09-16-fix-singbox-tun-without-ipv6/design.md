## Context

- `backend.log.2` on the affected host: every sing-box 1.14.0 `tun=on` session exits with `set rules: add rule 0/9: address family not supported by protocol`; `/proc/cmdline` has `ipv6.disable=1`, `/proc/sys/net/ipv6` is absent, `ip -6 rule` fails with the same errno.
- sing-tun v0.9.0-beta.4 (the version in sing-box 1.14.0), `tun_linux.go` `rules()`: with `auto_route`, an IPv4 address only (`p4`, not `p6`) and `StrictRoute`, the first rule is `Family: AF_INET6, Type: FR_ACT_UNREACHABLE`. That is rule 0 of 9. `StrictRoute` is read nowhere else on Linux except `redirect_nftables_rules.go`, which only applies with `auto_redirect` (not generated).
- The generated inbound (`crates/core/src/config/singbox.rs:193`) copies `tun.strict_route` verbatim.
- xray already handles this: `crates/netctl/src/net.rs` `v6_rules_needed(v6_requested, strict, host_has_ipv6)` skips IPv6 rules when `/proc/sys/net/ipv6` is absent.
- `connection.rs` builds `effective_settings` per candidate before `writer.write_config`, so a per-connection adjustment does not touch persisted settings.

## Goals / Non-Goals

**Goals:**
- sing-box TUN starts on IPv6-less hosts with IPv4 behavior identical to `strict_route: true` on sing-tun v0.9.

**Non-Goals:**
- Changing the user's persisted `strict_route`, or the preferences UI.
- Making IPv6 tunnel addresses work on IPv6-less hosts; that is impossible.
- Probing inside a separate network namespace: the app and backend share the host namespace.

## Decisions

- **Adjust effective settings in the connection task, not in the generator.** The generator stays a pure function of settings. Doing it where host pins and other per-connection facts are already applied keeps host probing out of `v2ray-rs-core`. Alternative rejected: a `host_has_ipv6` input on `ConfigGenerator` — widens a trait used by three backends for one sing-box fact.
- **Probe `/proc/sys/net/ipv6` existence.** Same signal netctl uses, set by `ipv6.disable=1`. `net.ipv6.conf.all.disable_ipv6=1` keeps the address family, so rule adds succeed and no adjustment is needed there.
- **Reject an IPv6 tunnel address up front.** Checked in `start_connection` before the connection task spawns, for both TUN backends, so it is one clear toast instead of a crash per candidate. Error text: TUN IPv6 address is set but the kernel has IPv6 disabled (`ipv6.disable=1`); clear the IPv6 address in TUN settings.
- **One notice line per connection.** Emitted through the connection's log stream (so it reaches both the logs page and `backend.log`), e.g. `notice: kernel IPv6 is disabled; sing-box strict_route turned off for this session (IPv4 routing unchanged)`.

## Risks / Trade-offs

- [A future sing-tun uses `StrictRoute` for IPv4-only behavior too] → the notice makes the downgrade visible; the unit test pins the current contract and the tun-mode requirement names the verified version.
- [Host probe in unit tests] → the decision is a pure function of `(backend, host_has_ipv6)`; tests call it with explicit inputs, and the connection task receives the probe result as `ConnectionRequest.host_has_ipv6`.
- [The 2.2 test relies on sing-box refusing an XHTTP node at generation] → `fix-node-transport-emission` keeps sing-box behavior unchanged, so the route holds.
- [`STRICT_ROUTE_NOTICE` starts with `notice:` and is written under the `notice` stream tag] → `backend.log` reads `notice notice: …`; accepted, matches how manager notices read on the logs page.

## Implementation plan

Tier standard, mode existing-service-strict, lenses `spec` + `quality` (UI and process wiring, no auth/IO/perf triggers). Rust stack: every seam carries `NO-RED-WAIVER:` / `NO-TESTER-WAIVER:`; tests are each coder's first task and chunks close on their verify command. Chunks are serial in one worktree, because `crates/ui` consumes the `crates/process` export and c2 and c3 both edit `crates/ui/src/app.rs`.

**c1-probe** — task 1.1, coder `zpatcher`.
- `crates/process/src/tun.rs`: add `pub fn host_has_ipv6() -> bool { Path::new("/proc/sys/net/ipv6").exists() }` next to `fn device_path(iface: &str) -> String {`.
- `crates/process/src/lib.rs`: add it to the `pub use tun::{` list.
- No unit test, since it would only re-read the path it checks.
- Verify: `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-process --all-targets -- -D warnings`.

**c2-strict-route** — tasks 2.1 and 2.2, prev c1-probe, coder `rust-coder`.
- `crates/ui/src/connection.rs`: add `fn singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool` (`backend != BackendType::SingBox || host_has_ipv6`).
- Add field `pub host_has_ipv6: bool` on `ConnectionRequest`.
- Before `for candidate in candidates {`, compute `drop_strict_route = settings.tun.enabled && settings.tun.strict_route && !singbox_strict_route_allowed(..)`. When it is set, emit `STRICT_ROUTE_NOTICE` exactly once, to `backend.log` (`append_line("notice", …)`) and as `AppMsg::ProcessLogLine`.
- Per candidate, before `write_config`, set `effective_settings.tun.strict_route = false`.
- `crates/ui/src/app.rs`: the `ConnectionRequest` literal gets `host_has_ipv6: v2ray_rs_process::host_has_ipv6()`.
- `STRICT_ROUTE_NOTICE` = `notice: kernel IPv6 is disabled; sing-box strict_route turned off for this session (IPv4 routing unchanged)`.
- Tests:
  - `strict_route_allowed_unless_singbox_lacks_ipv6`.
  - `singbox_tun_without_ipv6_turns_strict_route_off_once`. It uses two candidates: a VLESS XHTTP node that sing-box refuses at generation, then Shadowsocks, which stops at `HostProbe { getcap: /bin/true }`. It asserts compact `"strict_route":false`, one notice in both messages and `backend.log`, and input settings still `strict_route: true`.
  - `singbox_tun_with_ipv6_keeps_strict_route`.
- Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`.

**c3-preflight** — task 3.1, prev c2-strict-route, coder `rust-coder`.
- `crates/ui/src/app.rs`: add `fn tun_ipv6_unavailable(tun: &TunConfig, backend: BackendType, host_has_ipv6: bool) -> bool` beside `missing_geodata`.
- Add `TUN_IPV6_DISABLED` = `TUN IPv6 address is set but the kernel has IPv6 disabled (ipv6.disable=1); clear the IPv6 address in TUN settings`.
- `start_connection` probes once after the binary check. It shows a toast and returns `Err` before rule load, the generation bump and `connection::spawn`, and reuses the local for the request.
- Tests:
  - `tun_ipv6_unavailable_table`, 14 rows.
  - `tun_ipv6_disabled_error_names_kernel_state_and_setting`.
- Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 tun_ipv6 && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`.

**Floor (4.1):** `timeout 10m cargo test --workspace -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets -- -D warnings`.

**Manual (4.2):** a live check on a host booted with `ipv6.disable=1`. It cannot run in an automated run and stays open for the user.

Plan review: zarchitect, 2 rounds, pass. Round 1 raised one blocker: c2 could not compile in parallel with c1. It was fixed by making the chunks serial and adding the `app.rs` request site to c2.

## Plan appendix

```json
{
  "v": 2,
  "change": "fix-singbox-tun-without-ipv6",
  "baseSha": "42304a4823de1c97f61085da8ae2c8812fc495b3",
  "generatedAt": "2026-09-15T20:16:38.055805+00:00",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 1.8,
  "chunks": [
    {
      "id": "c1-probe",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "host-ipv6-probe",
      "shard": "",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "host_has_ipv6 (new pub fn)",
          "anchor": "fn device_path(iface: &str) -> String {",
          "change": "add `pub fn host_has_ipv6() -> bool { Path::new(\"/proc/sys/net/ipv6\").exists() }` next to the other host-path probes (device_path/wait_for_device); `Path` already imported (`use std::path::{Path, PathBuf};`). Doc comment mirrors netctl: absent when kernel boots with `ipv6.disable=1`."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/lib.rs",
          "symbol": "pub use tun::{...}",
          "anchor": "    helpers_stale, relocated_helper_path, relocation_required, run_helper, run_path,",
          "change": "add `host_has_ipv6` to the alphabetically ordered tun re-export list"
        }
      ],
      "contract": {
        "states": [
          "ipv6-present",
          "ipv6-absent"
        ],
        "transitions": [
          {
            "input": "/proc/sys/net/ipv6 exists (IPv6 kernel, including net.ipv6.conf.all.disable_ipv6=1)",
            "state": "ipv6-present",
            "effect": "set",
            "evidence": "crates/netctl/src/net.rs:135-139; design.md Decisions 'Probe /proc/sys/net/ipv6 existence'"
          },
          {
            "input": "/proc/sys/net/ipv6 absent (booted ipv6.disable=1)",
            "state": "ipv6-absent",
            "effect": "set",
            "evidence": "design.md Context backend.log evidence; net.rs:135-136"
          }
        ],
        "forbidden": [
          "reading any other signal (disable_ipv6 sysctl value, ip -6 output)",
          "v2ray-rs-netctl depending on v2ray-rs-process for this probe"
        ],
        "seeding": [
          "not seeded in tests; production-only call from start_connection"
        ],
        "budgets": [
          "1 stat syscall per Connect click; 0 calls per candidate"
        ],
        "decision": "crates/process/src/tun.rs: `pub fn host_has_ipv6() -> bool { Path::new(\"/proc/sys/net/ipv6\").exists() }` with doc comment mirroring net.rs:135-136; add `host_has_ipv6` to the `pub use tun::{...}` list in crates/process/src/lib.rs:20-23. netctl keeps its private copy."
      },
      "redTasks": [],
      "codeTasks": [
        "1.1"
      ],
      "redTests": [],
      "verify": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-process --all-targets -- -D warnings",
      "coder": "zpatcher"
    },
    {
      "id": "c2-strict-route",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "c1-probe",
      "sharedPkg": "crates/ui consumes the crates/process export host_has_ipv6",
      "parallel": false,
      "seam": "singbox-strict-route",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "singbox_strict_route_allowed (new fn)",
          "anchor": "fn relays(state: &ProcessState) -> bool {",
          "change": "add pure `fn singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool` (`backend != BackendType::SingBox || host_has_ipv6`) beside build_tun_runtime/relays; BackendType already imported at `AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,`"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "mod tests",
          "anchor": "    fn forwarder_relays_only_nonterminal_states() {",
          "change": "add #[test] cases: SingBox+false→false, SingBox+true→true, Xray+false→true (table style like netctl `v6_rules_follow_address_strict_and_host_support`)"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with task, before candidate loop",
          "anchor": "        for candidate in candidates {",
          "change": "before `for candidate in candidates {` (after backend_log is opened): `let drop_strict_route = settings.tun.enabled && settings.tun.strict_route && !singbox_strict_route_allowed(settings.backend.backend_type, host_has_ipv6);` and, when true, emit the notice exactly once here: `if let Some(log) = &backend_log { log.append_line(\"notice\", STRICT_ROUTE_NOTICE); }` then `sender.emit(AppMsg::ProcessLogLine(generation, STRICT_ROUTE_NOTICE.into()));`. No flag, no emission inside the loop."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with task, per candidate",
          "anchor": "            pin_node_addresses(&mut effective_settings, &nodes).await;",
          "change": "right after resolve_effective_config / pin_node_addresses and before `writer.write_config(&nodes, &effective_rules, &effective_settings)`: `if drop_strict_route { effective_settings.tun.strict_route = false; }`. Nothing else in the loop."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "ConnectionRequest (test seam)",
          "anchor": "pub struct ConnectionRequest",
          "change": "new field `pub host_has_ipv6: bool` on `ConnectionRequest` (crates/ui/src/connection.rs:35-52), destructured in spawn_with (75-88). Production: start_connection computes `let host_has_ipv6 = v2ray_rs_process::host_has_ipv6();` once and uses it for the preflight and the request. Tests set the bool; no test reads /proc."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "mod tests (new tokio test)",
          "anchor": "    async fn last_candidate_failure_reports_one_error() {",
          "change": "new #[tokio::test(flavor = \"multi_thread\")] tests singbox_tun_without_ipv6_turns_strict_route_off_once and singbox_tun_with_ipv6_keeps_strict_route, seeded exactly as: allowed/denied: call singbox_strict_route_allowed directly with literal inputs | strict-forced-off + notice-once: stub `r#\"[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30\"#`; settings = tun_settings() with backend_type SingBox, interface_name \"v2rstest2\" (TunConfig::default strict_route=true, tun.rs:74); candidates: [VLESS node address \"203.0.113.1\" with TransportSettings::Xhttp(XhttpSettings{ path: \"/x\".into(), host: None, mode: \"auto\".into() }), tls None -> sing-box UnsupportedTransport (singbox.rs:325-330) -> 'config generation failed' + continue], [candidate(\"203.0.113.2\") shadowsocks -> config written -> caps gate]; request.host_has_ipv6 = false; spawn_with(req, tx, |mgr| mgr.with_host_probe(HostProbe{ getcap: PathBuf::from(\"/bin/true\"), helper: PathBuf::from(\"/bin/true\") })) -> TunCapabilityMissing host-level Error ends the loop (manager.rs:350-379, connection.rs:240-252). IP literals avoid pin_node_addresses DNS lookups. | strict-kept + notice-none: same stub/probe, single candidate(\"203.0.113.2\"), request.host_has_ipv6 = true | persisted check: keep `let persisted = settings.clone()` before building the request; assert persisted.tun.strict_route after the terminal state (proves the input copy is untouched). VLESS fixture: a complete VlessConfig literal (valid uuid literal, encryption/flow/tls/remark None, every required field spelled) so the refusal is UnsupportedTransport, not validation. Assert compact `\"strict_route\":false` / `\"strict_route\":true` (writer uses serde_json::to_string) or parse JSON. Tolerate TunGrantRequired alongside the Error terminal. Persisted check: assert the request's input settings had strict_route true while the generated config has false (the effective copy was changed, the input was not)."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "start_connection ConnectionRequest literal",
          "anchor": "        let handle = crate::connection::spawn(",
          "change": "add `host_has_ipv6: v2ray_rs_process::host_has_ipv6(),` to the ConnectionRequest literal (c3 later hoists the probe into a local shared with the preflight)."
        }
      ],
      "contract": {
        "states": [
          "allowed",
          "denied",
          "strict-forced-off",
          "strict-kept",
          "notice-once",
          "notice-none"
        ],
        "transitions": [
          {
            "input": "singbox_strict_route_allowed(BackendType::SingBox, false)",
            "state": "denied",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec Scenario 'sing-box strict route on an IPv6-less host'"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::SingBox, true)",
            "state": "allowed",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec Scenario 'on a host with IPv6'"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::Xray, false)",
            "state": "allowed",
            "effect": "set",
            "evidence": "tasks.md 2.1 (netctl handles xray, net.rs:105-131)"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::V2ray, false)",
            "state": "allowed",
            "effect": "set",
            "evidence": "predicate body `backend != SingBox || host_has_ipv6`; v2ray has no TUN runtime (connection.rs:397-400)"
          },
          {
            "input": "sing-box, tun.enabled, tun.strict_route=true, host_has_ipv6=false; any candidate reaching write_config",
            "state": "strict-forced-off",
            "effect": "forced",
            "evidence": "spec Requirement 'SHALL set strict_route: false ... regardless of the persisted setting'; generated sing-box.json contains `\"strict_route\":false` (compact serde_json::to_string, crates/core/src/config/writer.rs:84)"
          },
          {
            "input": "same inputs, connection with 2 candidates (1st fails config generation, 2nd fails host-level)",
            "state": "notice-once",
            "effect": "set",
            "evidence": "tasks.md 2.2 'notice appears once across two failed candidates'; emission sits before `for candidate` (connection.rs:136)"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=true, host_has_ipv6=true",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "spec Scenario 'on a host with IPv6': `\"strict_route\":true`, no notice"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=true, host_has_ipv6=true",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "spec Scenario 'no notice SHALL be logged'"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=false, host_has_ipv6=false",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "nothing to turn off; drop_strict_route requires settings.tun.strict_route"
          },
          {
            "input": "sing-box, tun.enabled=false, host_has_ipv6=false",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "no tun inbound generated (singbox.rs:179 only under tun); drop_strict_route requires settings.tun.enabled"
          },
          {
            "input": "xray, tun.enabled, strict_route=true, host_has_ipv6=false",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "xray strict handled in netctl v6_rules_needed (net.rs:105,131); allowed=true"
          },
          {
            "input": "request settings after the connection task ran",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "spec 'persisted strict_route setting SHALL NOT change'; task mutates only effective_settings clone"
          }
        ],
        "forbidden": [
          "notice emitted inside the candidate loop (count would scale with candidates)",
          "notice emitted when effective strict_route was already false or TUN is off",
          "mutating `settings` (request copy) or self.settings instead of effective_settings",
          "reading /proc from ConnectionRequest consumers or tests (probe is only called in start_connection)",
          "changing ConfigGenerator, build_tun_inbound or TunConfig (generator stays pure)",
          "a test fixture reaching strict-forced-off with host_has_ipv6=true"
        ],
        "seeding": [
          "allowed/denied: call singbox_strict_route_allowed directly with literal inputs",
          "strict-forced-off + notice-once: stub `r#\"[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30\"#`; settings = tun_settings() with backend_type SingBox, interface_name \"v2rstest2\" (TunConfig::default strict_route=true, tun.rs:74); candidates: [VLESS node address \"203.0.113.1\" with TransportSettings::Xhttp(XhttpSettings{ path: \"/x\".into(), host: None, mode: \"auto\".into() }), tls None -> sing-box UnsupportedTransport (singbox.rs:325-330) -> 'config generation failed' + continue], [candidate(\"203.0.113.2\") shadowsocks -> config written -> caps gate]; request.host_has_ipv6 = false; spawn_with(req, tx, |mgr| mgr.with_host_probe(HostProbe{ getcap: PathBuf::from(\"/bin/true\"), helper: PathBuf::from(\"/bin/true\") })) -> TunCapabilityMissing host-level Error ends the loop (manager.rs:350-379, connection.rs:240-252). IP literals avoid pin_node_addresses DNS lookups.",
          "strict-kept + notice-none: same stub/probe, single candidate(\"203.0.113.2\"), request.host_has_ipv6 = true",
          "persisted check: keep `let persisted = settings.clone()` before building the request; assert persisted.tun.strict_route after the terminal state (proves the input copy is untouched)"
        ],
        "budgets": [
          "notice lines per connection: exactly 1 when drop_strict_route, else 0 — counted both as AppMsg::ProcessLogLine(GENERATION, STRICT_ROUTE_NOTICE) messages until the channel closes and as `contents.matches(STRICT_ROUTE_NOTICE).count()` in logs_dir()/backend.log",
          "probe calls inside the connection task: 0",
          "test wait per message: existing RECV_TIMEOUT 20s"
        ],
        "names": [
          "singbox_strict_route_allowed",
          "STRICT_ROUTE_NOTICE",
          "ConnectionRequest.host_has_ipv6",
          "drop_strict_route (local)",
          "tests::request (helper)",
          "generated file: stub.paths.generated_dir().join(\"sing-box.json\")"
        ],
        "refusals": [
          "none: this seam never refuses; the IPv4-less failure modes belong to seam ipv6-address-preflight"
        ],
        "decisions": [
          "`fn singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool { backend != BackendType::SingBox || host_has_ipv6 }` in crates/ui/src/connection.rs (BackendType = v2ray_rs_core::models::BackendType, the type of AppSettings.backend.backend_type, crates/core/src/models/settings.rs:15). Two args per tasks.md 2.1: 'allowed' is a backend+host fact; whether strict_route is on and TUN is enabled is the call-site condition that also gates the notice, so folding strict_route into the predicate would conflate 'allowed' with 'needs change'. design.md Risks names the three inputs of the whole decision, not the helper signature; no conflict.",
          "new field `pub host_has_ipv6: bool` on `ConnectionRequest` (crates/ui/src/connection.rs:35-52), destructured in spawn_with (75-88). Production: start_connection computes `let host_has_ipv6 = v2ray_rs_process::host_has_ipv6();` once and uses it for the preflight and the request. Tests set the bool; no test reads /proc.",
          "computed once before `for candidate in candidates` (after backend_log is opened, connection.rs:126-134): `let drop_strict_route = settings.tun.enabled && settings.tun.strict_route && !singbox_strict_route_allowed(settings.backend.backend_type, host_has_ipv6);` If true, emit the notice exactly once there: `if let Some(log) = &backend_log { log.append_line(\"notice\", STRICT_ROUTE_NOTICE); }` then `sender.emit(AppMsg::ProcessLogLine(generation, STRICT_ROUTE_NOTICE.into()));`. Inside the loop, right after resolve_effective_config (connection.rs:149-154): `if drop_strict_route { effective_settings.tun.strict_route = false; }`. The notice is outside the loop, so candidate count cannot change it. Line shape mirrors ProcessManager::push_notice (crates/process/src/manager.rs:657-665: stream tag \"notice\", content prefixed like unreadable_version manager.rs:933-935). Logs page order is safe: start_connection emits LogsMsg::Clear (app.rs:567) before connection::spawn.",
          "`const STRICT_ROUTE_NOTICE: &str = \"notice: kernel IPv6 is disabled; sing-box strict_route turned off for this session (IPv4 routing unchanged)\";` in crates/ui/src/connection.rs",
          "untouched by construction: the task owns a clone (`settings: self.settings.clone()`, app.rs:569) and only mutates the per-candidate effective_settings clone; nothing in connection.rs persists settings."
        ],
        "testRoute": "Use the design seeding route verbatim (XHTTP VLESS candidate fails sing-box generation, shadowsocks candidate hits HostProbe getcap=/bin/true caps gate). Verified: the pending fix-node-transport-emission change states sing-box behavior SHALL NOT change, so the XHTTP refusal (singbox.rs UnsupportedTransport) stays."
      },
      "redTasks": [],
      "codeTasks": [
        "2.1",
        "2.2"
      ],
      "redTests": [],
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "c3-preflight",
      "taskIds": [
        "3.1"
      ],
      "prev": "c2-strict-route",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "ipv6-address-preflight",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "start_connection",
          "anchor": "                self.show_toast(\"No backend binary configured — check Preferences\");",
          "change": "right after the binary_path match, before `let rules = match self.store.load_routing_rules() {`: `let host_has_ipv6 = v2ray_rs_process::host_has_ipv6(); if tun_ipv6_unavailable(&self.settings.tun, self.settings.backend.backend_type, host_has_ipv6) { self.show_toast(TUN_IPV6_DISABLED); return Err(TUN_IPV6_DISABLED.into()); }`; replace the ConnectionRequest field value from c2 with the `host_has_ipv6` local."
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "new pure predicate fn",
          "anchor": "fn missing_geodata(",
          "change": "add `fn tun_ipv6_unavailable(tun: &TunConfig, backend: BackendType, host_has_ipv6: bool) -> bool { tun.enabled && matches!(backend, BackendType::SingBox | BackendType::Xray) && tun.address_v6.is_some() && !host_has_ipv6 }` and `const TUN_IPV6_DISABLED: &str` beside missing_geodata"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "mod tests",
          "anchor": "    fn missing_geodata_singbox_is_never_gated() {",
          "change": "add table test over backend × v6-address × host-IPv6 (and tun enabled)"
        }
      ],
      "contract": {
        "states": [
          "rejected",
          "passed"
        ],
        "transitions": [
          {
            "input": "tun.enabled, backend SingBox, address_v6 Some(\"fd00::1/126\"), host_has_ipv6=false",
            "state": "rejected",
            "effect": "set",
            "evidence": "spec Scenario 'IPv6 tunnel address on an IPv6-less host'; tasks.md 3.1"
          },
          {
            "input": "tun.enabled, backend Xray, address_v6 Some, host_has_ipv6=false",
            "state": "rejected",
            "effect": "set",
            "evidence": "spec Scenario (either backend)"
          },
          {
            "input": "tun.enabled, backend V2ray, address_v6 Some, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec names sing-box or xray only; v2ray builds no TUN runtime (connection.rs:397-400)"
          },
          {
            "input": "tun.enabled, SingBox|Xray, address_v6 Some, host_has_ipv6=true",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec Requirement conditions on 'such a host'"
          },
          {
            "input": "tun.enabled, SingBox|Xray, address_v6 None, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec Scenario 'strict route on an IPv6-less host' has no IPv6 address and must connect"
          },
          {
            "input": "tun.enabled=false, SingBox|Xray, address_v6 Some, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "requirement scope 'TUN connection'; no tun inbound/runtime when disabled (connection.rs:394-396)"
          }
        ],
        "forbidden": [
          "rejection after connection::spawn or after apply_state(ProcessState::Starting) / connection_generation bump",
          "per-candidate rejection inside the connection task",
          "clearing or editing self.settings.tun.address_v6 automatically"
        ],
        "seeding": [
          "rejected/passed: call tun_ipv6_unavailable with a TunConfig built as `TunConfig { enabled, address_v6, ..TunConfig::default() }` and literal backend/bool; start_connection itself is GTK-bound and not unit-tested"
        ],
        "budgets": [
          "table test covers all 3 backends x 2 address_v6 values x 2 host values with tun enabled (12 rows) plus tun disabled rows for SingBox and Xray (2 rows) = 14 asserted rows",
          "toasts per rejected Connect: 1",
          "candidates tried when rejected: 0"
        ],
        "names": [
          "tun_ipv6_unavailable",
          "TUN_IPV6_DISABLED"
        ],
        "refusals": [
          "App::start_connection (crates/ui/src/app.rs:482) refuses synchronously on the Connect message, before connection::spawn, with toast and Err text TUN_IPV6_DISABLED = \"TUN IPv6 address is set but the kernel has IPv6 disabled (ipv6.disable=1); clear the IPv6 address in TUN settings\""
        ],
        "decisions": [
          "`fn tun_ipv6_unavailable(tun: &TunConfig, backend: BackendType, host_has_ipv6: bool) -> bool { tun.enabled && matches!(backend, BackendType::SingBox | BackendType::Xray) && tun.address_v6.is_some() && !host_has_ipv6 }` in crates/ui/src/app.rs next to missing_geodata (app.rs:1630), same explicit-input style. `is_some()` matches what the generator emits (TunConfig::addresses, crates/core/src/models/tun.rs:84-90).",
          "start_connection (app.rs:482): after the binary_path match (489-495), before load_routing_rules (497). `let host_has_ipv6 = v2ray_rs_process::host_has_ipv6(); if tun_ipv6_unavailable(&self.settings.tun, self.settings.backend.backend_type, host_has_ipv6) { self.show_toast(TUN_IPV6_DISABLED); return Err(TUN_IPV6_DISABLED.into()); }` — before connection_generation bump, runtime_snapshot, apply_state(Starting) and connection::spawn, so no candidate is tried.",
          "`const TUN_IPV6_DISABLED: &str = \"TUN IPv6 address is set but the kernel has IPv6 disabled (ipv6.disable=1); clear the IPv6 address in TUN settings\";` in crates/ui/src/app.rs",
          "start_connection passes the same probed bool into ConnectionRequest.host_has_ipv6 (field added by c2-strict-route)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.1"
      ],
      "redTests": [],
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 tun_ipv6 && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "host-ipv6-probe",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, no test-writer agent; tests are the first codeTask. NO-TESTER-WAIVER: rust stack; chunk closes on its verify command. Export `v2ray_rs_process::host_has_ipv6() -> bool` (existence of /proc/sys/net/ipv6) from crates/process/src/tun.rs, reference crates/netctl/src/net.rs:135-139. No dedicated unit test: the body is a single filesystem existence read, and a test would re-read the same /proc path it asserts on; callers take the result as an injected bool. Verify: process crate tests green.",
      "contract": {
        "states": [
          "ipv6-present",
          "ipv6-absent"
        ],
        "transitions": [
          {
            "input": "/proc/sys/net/ipv6 exists (IPv6 kernel, including net.ipv6.conf.all.disable_ipv6=1)",
            "state": "ipv6-present",
            "effect": "set",
            "evidence": "crates/netctl/src/net.rs:135-139; design.md Decisions 'Probe /proc/sys/net/ipv6 existence'"
          },
          {
            "input": "/proc/sys/net/ipv6 absent (booted ipv6.disable=1)",
            "state": "ipv6-absent",
            "effect": "set",
            "evidence": "design.md Context backend.log evidence; net.rs:135-136"
          }
        ],
        "forbidden": [
          "reading any other signal (disable_ipv6 sysctl value, ip -6 output)",
          "v2ray-rs-netctl depending on v2ray-rs-process for this probe"
        ],
        "seeding": [
          "not seeded in tests; production-only call from start_connection"
        ],
        "budgets": [
          "1 stat syscall per Connect click; 0 calls per candidate"
        ]
      },
      "codeTasks": [
        "1.1"
      ]
    },
    {
      "id": "singbox-strict-route",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, no test-writer agent; tests are the first codeTask. NO-TESTER-WAIVER: rust stack; chunk closes on its verify command. Pure `singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool` plus `ConnectionRequest.host_has_ipv6: bool`; the connection task computes `drop_strict_route` once before the candidate loop, emits STRICT_ROUTE_NOTICE once to backend.log (stream tag \"notice\") and AppMsg::ProcessLogLine, and forces effective_settings.tun.strict_route = false per candidate before write_config. Test helper change: extract the ConnectionRequest literal in tests `connect_with` (connection.rs:532-558) into `fn request(stub: &Stub, settings: AppSettings, candidates: Vec<ConnectionCandidate>) -> ConnectionRequest` with `host_has_ipv6: true`; new tests set `host_has_ipv6 = false` and call spawn_with directly.",
      "contract": {
        "states": [
          "allowed",
          "denied",
          "strict-forced-off",
          "strict-kept",
          "notice-once",
          "notice-none"
        ],
        "transitions": [
          {
            "input": "singbox_strict_route_allowed(BackendType::SingBox, false)",
            "state": "denied",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec Scenario 'sing-box strict route on an IPv6-less host'"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::SingBox, true)",
            "state": "allowed",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec Scenario 'on a host with IPv6'"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::Xray, false)",
            "state": "allowed",
            "effect": "set",
            "evidence": "tasks.md 2.1 (netctl handles xray, net.rs:105-131)"
          },
          {
            "input": "singbox_strict_route_allowed(BackendType::V2ray, false)",
            "state": "allowed",
            "effect": "set",
            "evidence": "predicate body `backend != SingBox || host_has_ipv6`; v2ray has no TUN runtime (connection.rs:397-400)"
          },
          {
            "input": "sing-box, tun.enabled, tun.strict_route=true, host_has_ipv6=false; any candidate reaching write_config",
            "state": "strict-forced-off",
            "effect": "forced",
            "evidence": "spec Requirement 'SHALL set strict_route: false ... regardless of the persisted setting'; generated sing-box.json contains `\"strict_route\":false` (compact serde_json::to_string, crates/core/src/config/writer.rs:84)"
          },
          {
            "input": "same inputs, connection with 2 candidates (1st fails config generation, 2nd fails host-level)",
            "state": "notice-once",
            "effect": "set",
            "evidence": "tasks.md 2.2 'notice appears once across two failed candidates'; emission sits before `for candidate` (connection.rs:136)"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=true, host_has_ipv6=true",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "spec Scenario 'on a host with IPv6': `\"strict_route\":true`, no notice"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=true, host_has_ipv6=true",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "spec Scenario 'no notice SHALL be logged'"
          },
          {
            "input": "sing-box, tun.enabled, strict_route=false, host_has_ipv6=false",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "nothing to turn off; drop_strict_route requires settings.tun.strict_route"
          },
          {
            "input": "sing-box, tun.enabled=false, host_has_ipv6=false",
            "state": "notice-none",
            "effect": "no-op",
            "evidence": "no tun inbound generated (singbox.rs:179 only under tun); drop_strict_route requires settings.tun.enabled"
          },
          {
            "input": "xray, tun.enabled, strict_route=true, host_has_ipv6=false",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "xray strict handled in netctl v6_rules_needed (net.rs:105,131); allowed=true"
          },
          {
            "input": "request settings after the connection task ran",
            "state": "strict-kept",
            "effect": "no-op",
            "evidence": "spec 'persisted strict_route setting SHALL NOT change'; task mutates only effective_settings clone"
          }
        ],
        "forbidden": [
          "notice emitted inside the candidate loop (count would scale with candidates)",
          "notice emitted when effective strict_route was already false or TUN is off",
          "mutating `settings` (request copy) or self.settings instead of effective_settings",
          "reading /proc from ConnectionRequest consumers or tests (probe is only called in start_connection)",
          "changing ConfigGenerator, build_tun_inbound or TunConfig (generator stays pure)",
          "a test fixture reaching strict-forced-off with host_has_ipv6=true"
        ],
        "seeding": [
          "allowed/denied: call singbox_strict_route_allowed directly with literal inputs",
          "strict-forced-off + notice-once: stub `r#\"[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30\"#`; settings = tun_settings() with backend_type SingBox, interface_name \"v2rstest2\" (TunConfig::default strict_route=true, tun.rs:74); candidates: [VLESS node address \"203.0.113.1\" with TransportSettings::Xhttp(XhttpSettings{ path: \"/x\".into(), host: None, mode: \"auto\".into() }), tls None -> sing-box UnsupportedTransport (singbox.rs:325-330) -> 'config generation failed' + continue], [candidate(\"203.0.113.2\") shadowsocks -> config written -> caps gate]; request.host_has_ipv6 = false; spawn_with(req, tx, |mgr| mgr.with_host_probe(HostProbe{ getcap: PathBuf::from(\"/bin/true\"), helper: PathBuf::from(\"/bin/true\") })) -> TunCapabilityMissing host-level Error ends the loop (manager.rs:350-379, connection.rs:240-252). IP literals avoid pin_node_addresses DNS lookups.",
          "strict-kept + notice-none: same stub/probe, single candidate(\"203.0.113.2\"), request.host_has_ipv6 = true",
          "persisted check: keep `let persisted = settings.clone()` before building the request; assert persisted.tun.strict_route after the terminal state (proves the input copy is untouched)"
        ],
        "budgets": [
          "notice lines per connection: exactly 1 when drop_strict_route, else 0 — counted both as AppMsg::ProcessLogLine(GENERATION, STRICT_ROUTE_NOTICE) messages until the channel closes and as `contents.matches(STRICT_ROUTE_NOTICE).count()` in logs_dir()/backend.log",
          "probe calls inside the connection task: 0",
          "test wait per message: existing RECV_TIMEOUT 20s"
        ],
        "names": [
          "singbox_strict_route_allowed",
          "STRICT_ROUTE_NOTICE",
          "ConnectionRequest.host_has_ipv6",
          "drop_strict_route (local)",
          "tests::request (helper)",
          "generated file: stub.paths.generated_dir().join(\"sing-box.json\")"
        ],
        "refusals": [
          "none: this seam never refuses; the IPv4-less failure modes belong to seam ipv6-address-preflight"
        ]
      },
      "codeTasks": [
        "2.1",
        "2.2"
      ]
    },
    {
      "id": "ipv6-address-preflight",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, no test-writer agent; tests are the first codeTask. NO-TESTER-WAIVER: rust stack; chunk closes on its verify command. Pure `tun_ipv6_unavailable(tun: &TunConfig, backend: BackendType, host_has_ipv6: bool) -> bool` in crates/ui/src/app.rs beside missing_geodata (1630); start_connection calls v2ray_rs_process::host_has_ipv6() once after the binary_path check and, when the predicate holds, shows toast TUN_IPV6_DISABLED and returns Err(TUN_IPV6_DISABLED.into()) before routing-rule load, generation bump, apply_state(Starting) and connection::spawn; otherwise passes host_has_ipv6 into ConnectionRequest.",
      "contract": {
        "states": [
          "rejected",
          "passed"
        ],
        "transitions": [
          {
            "input": "tun.enabled, backend SingBox, address_v6 Some(\"fd00::1/126\"), host_has_ipv6=false",
            "state": "rejected",
            "effect": "set",
            "evidence": "spec Scenario 'IPv6 tunnel address on an IPv6-less host'; tasks.md 3.1"
          },
          {
            "input": "tun.enabled, backend Xray, address_v6 Some, host_has_ipv6=false",
            "state": "rejected",
            "effect": "set",
            "evidence": "spec Scenario (either backend)"
          },
          {
            "input": "tun.enabled, backend V2ray, address_v6 Some, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec names sing-box or xray only; v2ray builds no TUN runtime (connection.rs:397-400)"
          },
          {
            "input": "tun.enabled, SingBox|Xray, address_v6 Some, host_has_ipv6=true",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec Requirement conditions on 'such a host'"
          },
          {
            "input": "tun.enabled, SingBox|Xray, address_v6 None, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "spec Scenario 'strict route on an IPv6-less host' has no IPv6 address and must connect"
          },
          {
            "input": "tun.enabled=false, SingBox|Xray, address_v6 Some, host_has_ipv6=false",
            "state": "passed",
            "effect": "no-op",
            "evidence": "requirement scope 'TUN connection'; no tun inbound/runtime when disabled (connection.rs:394-396)"
          }
        ],
        "forbidden": [
          "rejection after connection::spawn or after apply_state(ProcessState::Starting) / connection_generation bump",
          "per-candidate rejection inside the connection task",
          "clearing or editing self.settings.tun.address_v6 automatically"
        ],
        "seeding": [
          "rejected/passed: call tun_ipv6_unavailable with a TunConfig built as `TunConfig { enabled, address_v6, ..TunConfig::default() }` and literal backend/bool; start_connection itself is GTK-bound and not unit-tested"
        ],
        "budgets": [
          "table test covers all 3 backends x 2 address_v6 values x 2 host values with tun enabled (12 rows) plus tun disabled rows for SingBox and Xray (2 rows) = 14 asserted rows",
          "toasts per rejected Connect: 1",
          "candidates tried when rejected: 0"
        ],
        "names": [
          "tun_ipv6_unavailable",
          "TUN_IPV6_DISABLED"
        ],
        "refusals": [
          "App::start_connection (crates/ui/src/app.rs:482) refuses synchronously on the Connect message, before connection::spawn, with toast and Err text TUN_IPV6_DISABLED = \"TUN IPv6 address is set but the kernel has IPv6 disabled (ipv6.disable=1); clear the IPv6 address in TUN settings\""
        ]
      },
      "codeTasks": [
        "3.1"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The system SHALL start TUN connections on a host whose kernel has no IPv6 support (booted with `ipv6.disable=1`).",
      "tests": [
        "connection::tests::singbox_tun_without_ipv6_turns_strict_route_off_once",
        "manual 4.2"
      ]
    },
    {
      "shall": "For sing-box, the connection's generated config SHALL set `strict_route: false` on such a host regardless of the persisted setting, because sing-box's strict route adds an IPv6 policy rule the kernel rejects, and SHALL write one notice line to the process log stream stating that kernel IPv6 is disabled and strict route was turned off for the session.",
      "tests": [
        "connection::tests::singbox_strict_route_denied_only_for_singbox_without_ipv6",
        "connection::tests::singbox_tun_without_ipv6_turns_strict_route_off_once",
        "connection::tests::singbox_tun_with_ipv6_keeps_strict_route"
      ]
    },
    {
      "shall": "The persisted `strict_route` setting SHALL NOT change.",
      "tests": [
        "connection::tests::singbox_tun_without_ipv6_turns_strict_route_off_once"
      ]
    },
    {
      "shall": "When an IPv6 tunnel address is configured on such a host, a TUN connection for either backend SHALL fail before any backend is spawned, with an error stating that the kernel has IPv6 disabled and that the IPv6 tunnel address must be cleared.",
      "tests": [
        "app::tests::tun_ipv6_unavailable_only_for_tun_backends_without_host_ipv6",
        "app::tests::tun_ipv6_disabled_error_names_kernel_state_and_setting"
      ]
    },
    {
      "shall": "- **THEN** the generated tun inbound SHALL contain `\"strict_route\": false`, the process log SHALL contain one notice naming disabled kernel IPv6, and the persisted settings SHALL still have `strict_route` on",
      "tests": [
        "connection::tests::singbox_tun_without_ipv6_turns_strict_route_off_once"
      ]
    },
    {
      "shall": "- **THEN** the generated tun inbound SHALL contain `\"strict_route\": true` and no notice SHALL be logged",
      "tests": [
        "connection::tests::singbox_tun_with_ipv6_keeps_strict_route"
      ]
    },
    {
      "shall": "- **THEN** Connect SHALL fail without spawning a backend or trying any candidate, and the user SHALL see an error naming disabled kernel IPv6 and the IPv6 tunnel address setting",
      "tests": [
        "app::tests::tun_ipv6_unavailable_only_for_tun_backends_without_host_ipv6",
        "app::tests::tun_ipv6_disabled_error_names_kernel_state_and_setting"
      ]
    }
  ],
  "testHarness": [
    "Stub / stub(script) — crates/ui/src/connection.rs:`fn stub(script: &str) -> Stub {` — TempDir + AppPaths::for_profile_in(AppProfile::Test, ..) with ensure_dirs, writes an executable /bin/sh backend script",
    "executable(dir, name) — crates/ui/src/connection.rs:`fn executable(dir: &std::path::Path, name: &str) -> PathBuf {` — exit-0 script (note ETXTBSY comment: prefer /bin/true for getcap when possible; a cap-reporting getcap must be a script)",
    "candidate(address) / node(address) — crates/ui/src/connection.rs:`fn candidate(address: &str) -> ConnectionCandidate {` — manual Shadowsocks candidate",
    "connect / connect_with — crates/ui/src/connection.rs:`fn connect_with(` — builds ConnectionRequest (ConfigWriter over stub.paths, GENERATION=7) and calls spawn_with with a ProcessManager hook; returns (ConnectionHandle, relm4::Receiver<AppMsg>)",
    "singbox_settings() / tun_settings() — crates/ui/src/connection.rs:`fn singbox_settings() -> AppSettings {` / `fn tun_settings() -> AppSettings {`",
    "next_state / assert_nothing_after_terminal — crates/ui/src/connection.rs:`async fn next_state(` — drains AppMsg until ProcessStateConnection",
    "Log-line assertion precedent — crates/ui/src/connection.rs:`async fn log_lines_carry_connection_generation() {` matches `AppMsg::ProcessLogLine(generation, line)`",
    "Generated-config assertion precedent — crates/ui/src/connection.rs:`let config = std::fs::read_to_string(stub.paths.generated_dir().join(\"xray.json\"))` (sing-box file name is `sing-box.json`, crates/core/src/config/writer.rs:`BackendType::SingBox => \"sing-box.json\",`)",
    "backend.log assertion precedent — crates/ui/src/connection.rs:`async fn live_connect_writes_backend_diagnostics() {` reads stub.paths.logs_dir().join(\"backend.log\") and counts matches",
    "Fast non-host-level candidate failure — crates/ui/src/connection.rs:`[ \"$1\" = check ] && { grep -q 203.0.113.3 \"$3\" && exit 1; exit 0; }` → \"config rejected\"; config check runs after TUN gates (crates/process/src/manager.rs:`if let Err(e) = self.check_config().await {`)",
    "HostProbe — crates/process/src/manager.rs:`pub struct HostProbe {` (getcap, helper; cfg test/test-utils) applied via `pub fn with_host_probe(mut self, probe: HostProbe) -> Self {` — skips mount gate, forces CAP gate; cap-granting getcap script precedent crates/process/src/privilege.rs:`echo \\\"$0 cap_net_admin+ep\\\"`",
    "app.rs pure-predicate tests — crates/ui/src/app.rs:`    fn missing_geodata_singbox_is_never_gated() {` inside `mod tests` (with `#[allow(clippy::items_after_test_module)]`)"
  ],
  "targetedTests": [
    "v2ray_rs_ui connection::tests::singbox_strict_route_denied_only_for_singbox_without_ipv6 (unit: SingBox/false -> false, SingBox/true -> true, Xray/false -> true, V2ray/false -> true)",
    "v2ray_rs_ui connection::tests::singbox_tun_without_ipv6_turns_strict_route_off_once (async multi_thread: two candidates, generated sing-box.json contains \"strict_route\":false and 203.0.113.2, ProcessLogLine STRICT_ROUTE_NOTICE count == 1, backend.log STRICT_ROUTE_NOTICE count == 1, persisted.tun.strict_route == true, Error terminal, assert_nothing_after_terminal)",
    "v2ray_rs_ui connection::tests::singbox_tun_with_ipv6_keeps_strict_route (async multi_thread: single candidate, host_has_ipv6=true, sing-box.json contains \"strict_route\":true, zero STRICT_ROUTE_NOTICE lines in messages and backend.log)",
    "v2ray_rs_ui app::tests::tun_ipv6_unavailable_only_for_tun_backends_without_host_ipv6 (14-row table)",
    "v2ray_rs_ui app::tests::tun_ipv6_disabled_error_names_kernel_state_and_setting (asserts TUN_IPV6_DISABLED contains \"kernel has IPv6 disabled\" and \"clear the IPv6 address in TUN settings\")"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets -- -D warnings",
  "manual": [
    "4.2 live check on a host booted with ipv6.disable=1 — left unchecked, handed to the user"
  ],
  "risks": [
    "Test 2.2 relies on sing-box UnsupportedTransport for XHTTP; the pending fix-node-transport-emission spec states sing-box behavior SHALL NOT change, so the route holds.",
    "Test uses HostProbe getcap=/bin/true so the TUN caps gate yields host-level TunCapabilityMissing on candidate 2; if run as root the gate still runs because host_pinned() forces it (manager.rs:350-351) -> deterministic.",
    "A future sing-tun reading StrictRoute for IPv4 behavior -> notice makes the downgrade visible; unit test pins the current contract (design.md Risks).",
    "Task 4.2 is manual-only (needs ipv6.disable=1 host) -> left unchecked with a user hand-off; automated run ends at 4.1.",
    "docs/ARCHITECTURE.md TUN section may deserve one line about IPv6-less hosts -> not in tasks; flag to the user rather than edit unasked.",
    "tasks.md 1.1 names no test; seam host-ipv6-probe deliberately adds none (tautological /proc read) -> coverage comes from injected-bool tests and 4.2.",
    "STRICT_ROUTE_NOTICE keeps the `notice:` prefix (design.md text, matches manager content like `warning: xray ...` on the logs page, which shows content only); backend.log reads `notice notice: ...` — cosmetic, accepted."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
