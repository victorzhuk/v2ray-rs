## Context

- Path resolution: `runtime_dir` = `$XDG_RUNTIME_DIR/<qualifier>` else `data_dir/runtime`; `state_dir` = `$XDG_STATE_HOME/<qualifier>` else `data_dir/state` (`crates/core/src/persistence/mod.rs:84-92`), as required by `openspec/specs/runtime-profiles/spec.md:71-93`. Files under them: `backend.pid`, `generated/`, `v2ray-rs.lock` (runtime); `latency_snapshot.json`, `logs/`, `instance.json`, `tun_session.json` (state) (`mod.rs:218-243`).
- `docs/ARCHITECTURE.md:420-436` shows only the XDG form; the Logging section (heading `:455`) places logs under `$XDG_STATE_HOME/v2ray-rs/logs/`. On the reference host `XDG_STATE_HOME` is unset and logs live in `~/.local/share/v2ray-rs/state/logs`, while `XDG_RUNTIME_DIR` is set (`/run/user/1000/v2ray-rs/generated/xray.json` in `v2ray-rs.log`).
- `docs/ARCHITECTURE.md:149-150` says `ProbeRunner` verifies the binary before a connect. Its only user is `measure_real_delay` (`crates/subscription/src/real_delay.rs:121`); `:85` of the doc already says so correctly.
- `docs/ARCHITECTURE.md:133-135`: "Each helper call is bounded at 10s". `HELPER_TIMEOUT` and `DEVICE_TIMEOUT` are 10s (`crates/process/src/tun.rs:13-15`); `recover_tun_session` (`crates/ui/src/app.rs:3682`, called at startup `:314` and on release `:943`) also runs at `HELPER_TIMEOUT` = 10s — there is no `RECOVER_TIMEOUT` constant. The doc names no per-call bound.
- File-only settings: no widget reads or writes `tun.address_v6` (used only in `crates/ui/src/connection.rs:383`), `auto_update_geodata` / `geodata_update_interval_secs` (only `crates/ui/src/geodata_service.rs:28-29`), or `backend.config_output_dir` (only copied through in `preferences/network.rs:501`, `preferences/mod.rs:246`; honored by `crates/core/src/config/writer.rs:21`).
- Applicability: `connIdle` is emitted only by the v2ray-family generator (`crates/core/src/config/v2ray.rs:96-98`), nothing in `singbox.rs`; the row has no note (`crates/ui/src/preferences/network.rs:127-139`), while the WebSocket ping row says "xray only" (`:143-156`). xray honors a detour only as `direct` under TUN (`v2ray.rs:783-787`); the dialog shows the detour combo for both sing-box and xray without a note (`crates/ui/src/preferences/dns.rs:1198`, `:1258-1264`). `dns_hijack` is compared only against `Hijack` in both generators and the capture decision (`v2ray.rs:147`, `:515`; `singbox.rs:652`; `connection.rs:391-393`), so `native` and `disabled` generate the same config; the combo has no note (`crates/ui/src/preferences/tun.rs:108-116`).
- `CLAUDE.md:42` lists `latency_snapshot.json` and `tun_session.json` right after the `~/.local/share/v2ray-rs/` sentence without naming `state_dir`; it states no log path. Other CLAUDE.md drift (≈0.6.0 era) is out of scope.
- `CHANGELOG.md:13-18` uses `<state_dir>/logs/` and `:361`, `:370` use `state_dir/…`: correct, no edit.

## Goals / Non-Goals

**Goals:**
- A reader can find every on-disk file on a host with or without the XDG variables, and every preference row that a backend ignores says so.

**Non-Goals:**
- Changing `DnsHijackMode` semantics or removing `native`; that is a behavior decision for a separate change.
- Adding UI for file-only settings.
- A full CLAUDE.md reconciliation.
- Changing timeouts (`complete-tun-preflight`).

## Decisions

- **Show both forms in the layout.** Headings become `$XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/)`, and the same for runtime, with one sentence on the rule and the profile qualifier. Alternative rejected: documenting only `<state_dir>` — readers need a literal path to look in.
- **Timeouts per call.** List each call with the constant that bounds it, values re-read from the code at edit time (verified at planning: all three at 10s — `DEVICE_TIMEOUT` device wait, `HELPER_TIMEOUT` for `xray-up`/`xray-down` and `recover`).
- **Notes as row subtitles.** Idle timeout and DNS hijack get `subtitle` text like the existing WebSocket ping row; the detour row's subtitle depends on the backend passed to the dialog. Alternative rejected: making rows insensitive — the idle timeout and detour values still matter after a backend switch, and hiding them would drop saved values from view.
- **Spec deltas only for UI notes.** Doc edits carry no requirement; the notes are observable UI and get ADDED requirements. A new `network-preferences-ui` capability exists because no spec covers the Network page.

## Risks / Trade-offs

- [Note text drifts from generator behavior] → each note is backed by a generator test that already pins the behavior (`connIdle` absent for sing-box, hijack rule only for `Hijack`, xray `dns-direct` tag only under TUN); the tasks cite them.

## Plan appendix

```json
{
  "v": 2,
  "change": "docs-sync-state-paths",
  "baseSha": "aa8dec00ff0847ab6a8b2948843d7d3d4887a335",
  "generatedAt": "2026-09-17T12:54:12.792Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "chunks": [
    {
      "id": "docs-a",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "arch-doc-prose",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "## On-disk layout + Logging",
          "anchor": "$XDG_RUNTIME_DIR/v2ray-rs/",
          "change": "Layout + Logging headings show both forms: $XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/) and $XDG_RUNTIME_DIR/v2ray-rs/ (else ~/.local/share/v2ray-rs/runtime/), one sentence on the rule (crates/core/src/persistence/mod.rs:84-92); logs under <state_dir>/logs"
        },
        {
          "task": "1.2",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "process section ProbeRunner sentence",
          "anchor": "generates a minimal backend",
          "change": "Replace pre-connect-verify claim: ProbeRunner is the Real Delay probe runner, sole user measure_real_delay (crates/subscription/src/real_delay.rs:121)"
        }
      ],
      "contract": {
        "states": [
          "layout headings read $XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/) and $XDG_RUNTIME_DIR/v2ray-rs/ (else ~/.local/share/v2ray-rs/runtime/) with the rule in one sentence",
          "ProbeRunner described as the Real Delay probe runner, not a pre-connect binary verifier",
          "helper timeouts listed per call: device wait DEVICE_TIMEOUT, xray-up/xray-down and recover HELPER_TIMEOUT, all 10s (recover_tun_session app.rs:3682 uses HELPER_TIMEOUT; no RECOVER_TIMEOUT exists)",
          "File-only settings subsection lists tun.address_v6, auto_update_geodata, geodata_update_interval_secs, backend.config_output_dir with TOML keys",
          "TUN/DNS prose carries applicability notes for idle timeout, DNS detour, DNS hijack modes"
        ],
        "transitions": [
          {
            "input": "edit on-disk layout + Logging sections (ARCHITECTURE.md:404-440)",
            "state": "both XDG and data_dir fallback forms shown; logs under <state_dir>/logs",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99 (runtime_dir = $XDG_RUNTIME_DIR/<qualifier> else data_dir/runtime; state_dir = $XDG_STATE_HOME/<qualifier> else data_dir/state); file list mod.rs:218-243 per design.md; openspec/specs/runtime-profiles/spec.md:71-93"
          },
          {
            "input": "edit ProbeRunner sentence (ARCHITECTURE.md:149-150)",
            "state": "states its only user is measure_real_delay, matching ARCHITECTURE.md:85",
            "effect": "set",
            "evidence": "crates/subscription/src/real_delay.rs:121 is the sole caller (design.md Context; tasks.md 1.2 rg check)"
          },
          {
            "input": "edit helper timeout sentence (ARCHITECTURE.md:133-135)",
            "state": "per-call values, recover value read from crates/ui/src/app.rs:37 at edit time",
            "effect": "set",
            "evidence": "HELPER_TIMEOUT/DEVICE_TIMEOUT 10s at crates/process/src/tun.rs:13-15; RECOVER_TIMEOUT 5s at crates/ui/src/app.rs:37 and :2239; complete-tun-preflight owns aligning it"
          },
          {
            "input": "add File-only settings subsection",
            "state": "four settings listed with TOML keys and no-widget claim",
            "effect": "set",
            "evidence": "tun.address_v6 used only at crates/ui/src/connection.rs:383; auto_update_geodata / geodata_update_interval_secs only at crates/ui/src/geodata_service.rs:28-29; backend.config_output_dir copied at crates/ui/src/preferences/network.rs:501 and preferences/mod.rs:246, honored at crates/core/src/config/writer.rs:21"
          },
          {
            "input": "add applicability notes next to TUN/DNS prose",
            "state": "idle timeout v2ray+xray only; detour xray direct-only-under-TUN; hijack native==disabled",
            "effect": "set",
            "evidence": "connIdle emitted at crates/core/src/config/v2ray.rs:96-98 (pinned by test_policy_conn_idle_from_settings, v2ray.rs:1046-1053), absent in singbox.rs; xray detour-only-direct-under-TUN v2ray.rs:783-787; hijack compared only against Hijack in v2ray.rs:147 and :515 and singbox.rs:652"
          }
        ],
        "forbidden": [
          "documenting only the XDG form (rejected alternative in design.md Decisions)",
          "stating a per-call bound without the constant that enforces it, or inventing a RECOVER_TIMEOUT constant (none exists; values re-read at edit time)",
          "listing a setting as file-only that has a widget, or omitting one of the four",
          "any non-doc file touched by this seam"
        ],
        "seeding": [
          "none: prose; review reads the rendered section against AppPaths helpers (tasks.md 1.1)"
        ],
        "budgets": [
          "none: no runtime behavior"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1 per seam contract",
        "1.2 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make check",
      "coder": "rust-coder"
    },
    {
      "id": "docs-b",
      "taskIds": [
        "1.3",
        "1.4"
      ],
      "prev": "docs-a",
      "sharedPkg": "docs/ARCHITECTURE.md",
      "parallel": false,
      "seam": "arch-doc-prose",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.3",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "process section helper timeout sentence",
          "anchor": "bounded at 10s",
          "change": "List per-call bounds with their constants: device wait DEVICE_TIMEOUT, xray-up/xray-down and recover HELPER_TIMEOUT (all 10s; recover_tun_session uses HELPER_TIMEOUT per crates/ui/src/app.rs:3682 — no RECOVER_TIMEOUT exists)"
        },
        {
          "task": "1.4",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "## On-disk layout",
          "anchor": "uses the qualifier",
          "change": "New \"File-only settings\" subsection: tun.address_v6, auto_update_geodata, geodata_update_interval_secs, backend.config_output_dir with TOML keys, no-widget note"
        }
      ],
      "contract": {
        "states": [
          "layout headings read $XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/) and $XDG_RUNTIME_DIR/v2ray-rs/ (else ~/.local/share/v2ray-rs/runtime/) with the rule in one sentence",
          "ProbeRunner described as the Real Delay probe runner, not a pre-connect binary verifier",
          "helper timeouts listed per call: device wait DEVICE_TIMEOUT, xray-up/xray-down and recover HELPER_TIMEOUT, all 10s (recover_tun_session app.rs:3682 uses HELPER_TIMEOUT; no RECOVER_TIMEOUT exists)",
          "File-only settings subsection lists tun.address_v6, auto_update_geodata, geodata_update_interval_secs, backend.config_output_dir with TOML keys",
          "TUN/DNS prose carries applicability notes for idle timeout, DNS detour, DNS hijack modes"
        ],
        "transitions": [
          {
            "input": "edit on-disk layout + Logging sections (ARCHITECTURE.md:404-440)",
            "state": "both XDG and data_dir fallback forms shown; logs under <state_dir>/logs",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99 (runtime_dir = $XDG_RUNTIME_DIR/<qualifier> else data_dir/runtime; state_dir = $XDG_STATE_HOME/<qualifier> else data_dir/state); file list mod.rs:218-243 per design.md; openspec/specs/runtime-profiles/spec.md:71-93"
          },
          {
            "input": "edit ProbeRunner sentence (ARCHITECTURE.md:149-150)",
            "state": "states its only user is measure_real_delay, matching ARCHITECTURE.md:85",
            "effect": "set",
            "evidence": "crates/subscription/src/real_delay.rs:121 is the sole caller (design.md Context; tasks.md 1.2 rg check)"
          },
          {
            "input": "edit helper timeout sentence (ARCHITECTURE.md:133-135)",
            "state": "per-call values, recover value read from crates/ui/src/app.rs:37 at edit time",
            "effect": "set",
            "evidence": "HELPER_TIMEOUT/DEVICE_TIMEOUT 10s at crates/process/src/tun.rs:13-15; RECOVER_TIMEOUT 5s at crates/ui/src/app.rs:37 and :2239; complete-tun-preflight owns aligning it"
          },
          {
            "input": "add File-only settings subsection",
            "state": "four settings listed with TOML keys and no-widget claim",
            "effect": "set",
            "evidence": "tun.address_v6 used only at crates/ui/src/connection.rs:383; auto_update_geodata / geodata_update_interval_secs only at crates/ui/src/geodata_service.rs:28-29; backend.config_output_dir copied at crates/ui/src/preferences/network.rs:501 and preferences/mod.rs:246, honored at crates/core/src/config/writer.rs:21"
          },
          {
            "input": "add applicability notes next to TUN/DNS prose",
            "state": "idle timeout v2ray+xray only; detour xray direct-only-under-TUN; hijack native==disabled",
            "effect": "set",
            "evidence": "connIdle emitted at crates/core/src/config/v2ray.rs:96-98 (pinned by test_policy_conn_idle_from_settings, v2ray.rs:1046-1053), absent in singbox.rs; xray detour-only-direct-under-TUN v2ray.rs:783-787; hijack compared only against Hijack in v2ray.rs:147 and :515 and singbox.rs:652"
          }
        ],
        "forbidden": [
          "documenting only the XDG form (rejected alternative in design.md Decisions)",
          "stating a per-call bound without the constant that enforces it, or inventing a RECOVER_TIMEOUT constant (none exists; values re-read at edit time)",
          "listing a setting as file-only that has a widget, or omitting one of the four",
          "any non-doc file touched by this seam"
        ],
        "seeding": [
          "none: prose; review reads the rendered section against AppPaths helpers (tasks.md 1.1)"
        ],
        "budgets": [
          "none: no runtime behavior"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.3 per seam contract",
        "1.4 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make check",
      "coder": "rust-coder"
    },
    {
      "id": "docs-c",
      "taskIds": [
        "1.5"
      ],
      "prev": "docs-b",
      "sharedPkg": "docs/ARCHITECTURE.md",
      "parallel": false,
      "seam": "arch-doc-prose",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.5",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "core section TUN/DNS prose",
          "anchor": "Direct detour",
          "change": "Per-backend applicability notes next to TUN/DNS prose: idle timeout v2ray+xray only; detour xray direct-only-under-TUN, sing-box proxy/direct, v2ray none; hijack native==disabled"
        }
      ],
      "contract": {
        "states": [
          "layout headings read $XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/) and $XDG_RUNTIME_DIR/v2ray-rs/ (else ~/.local/share/v2ray-rs/runtime/) with the rule in one sentence",
          "ProbeRunner described as the Real Delay probe runner, not a pre-connect binary verifier",
          "helper timeouts listed per call: device wait DEVICE_TIMEOUT, xray-up/xray-down and recover HELPER_TIMEOUT, all 10s (recover_tun_session app.rs:3682 uses HELPER_TIMEOUT; no RECOVER_TIMEOUT exists)",
          "File-only settings subsection lists tun.address_v6, auto_update_geodata, geodata_update_interval_secs, backend.config_output_dir with TOML keys",
          "TUN/DNS prose carries applicability notes for idle timeout, DNS detour, DNS hijack modes"
        ],
        "transitions": [
          {
            "input": "edit on-disk layout + Logging sections (ARCHITECTURE.md:404-440)",
            "state": "both XDG and data_dir fallback forms shown; logs under <state_dir>/logs",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99 (runtime_dir = $XDG_RUNTIME_DIR/<qualifier> else data_dir/runtime; state_dir = $XDG_STATE_HOME/<qualifier> else data_dir/state); file list mod.rs:218-243 per design.md; openspec/specs/runtime-profiles/spec.md:71-93"
          },
          {
            "input": "edit ProbeRunner sentence (ARCHITECTURE.md:149-150)",
            "state": "states its only user is measure_real_delay, matching ARCHITECTURE.md:85",
            "effect": "set",
            "evidence": "crates/subscription/src/real_delay.rs:121 is the sole caller (design.md Context; tasks.md 1.2 rg check)"
          },
          {
            "input": "edit helper timeout sentence (ARCHITECTURE.md:133-135)",
            "state": "per-call values, recover value read from crates/ui/src/app.rs:37 at edit time",
            "effect": "set",
            "evidence": "HELPER_TIMEOUT/DEVICE_TIMEOUT 10s at crates/process/src/tun.rs:13-15; RECOVER_TIMEOUT 5s at crates/ui/src/app.rs:37 and :2239; complete-tun-preflight owns aligning it"
          },
          {
            "input": "add File-only settings subsection",
            "state": "four settings listed with TOML keys and no-widget claim",
            "effect": "set",
            "evidence": "tun.address_v6 used only at crates/ui/src/connection.rs:383; auto_update_geodata / geodata_update_interval_secs only at crates/ui/src/geodata_service.rs:28-29; backend.config_output_dir copied at crates/ui/src/preferences/network.rs:501 and preferences/mod.rs:246, honored at crates/core/src/config/writer.rs:21"
          },
          {
            "input": "add applicability notes next to TUN/DNS prose",
            "state": "idle timeout v2ray+xray only; detour xray direct-only-under-TUN; hijack native==disabled",
            "effect": "set",
            "evidence": "connIdle emitted at crates/core/src/config/v2ray.rs:96-98 (pinned by test_policy_conn_idle_from_settings, v2ray.rs:1046-1053), absent in singbox.rs; xray detour-only-direct-under-TUN v2ray.rs:783-787; hijack compared only against Hijack in v2ray.rs:147 and :515 and singbox.rs:652"
          }
        ],
        "forbidden": [
          "documenting only the XDG form (rejected alternative in design.md Decisions)",
          "stating a per-call bound without the constant that enforces it, or inventing a RECOVER_TIMEOUT constant (none exists; values re-read at edit time)",
          "listing a setting as file-only that has a widget, or omitting one of the four",
          "any non-doc file touched by this seam"
        ],
        "seeding": [
          "none: prose; review reads the rendered section against AppPaths helpers (tasks.md 1.1)"
        ],
        "budgets": [
          "none: no runtime behavior"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.5 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make check",
      "coder": "rust-coder"
    },
    {
      "id": "claude-md",
      "taskIds": [
        "2.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "claude-md-persistence",
      "shard": "claudemd",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "CLAUDE.md",
          "symbol": "persistence.rs bullet",
          "anchor": "Also persists `latency_snapshot.json`",
          "change": "Name state_dir ($XDG_STATE_HOME/<qualifier> or data_dir/state) as home of latency_snapshot.json, tun_session.json, logs/"
        }
      ],
      "contract": {
        "states": [
          "persistence bullet names state_dir for latency_snapshot.json, tun_session.json, logs/ with the fallback rule"
        ],
        "transitions": [
          {
            "input": "edit persistence summary bullet (CLAUDE.md:~42)",
            "state": "state_dir named with both forms; file list unchanged otherwise",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99; design.md Context (CLAUDE.md:42 lists the files after the ~/.local/share sentence without naming state_dir)"
          }
        ],
        "forbidden": [
          "any other CLAUDE.md edit (design.md Non-Goal)",
          "contradicting docs/ARCHITECTURE.md layout wording"
        ],
        "seeding": [
          "none: prose"
        ],
        "budgets": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make check",
      "coder": "rust-coder"
    },
    {
      "id": "net-idle-note",
      "taskIds": [
        "3.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "network-idle-note",
      "shard": "netnote",
      "pkgDirs": [
        "crates/ui/src/preferences/"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/network.rs",
          "symbol": "idle_timeout_row",
          "anchor": "Streams idle longer than this are closed by the backend",
          "change": "Subtitle notes v2ray and xray only, sing-box ignores it; ws_heartbeat_row untouched (already xray-only)"
        }
      ],
      "contract": {
        "states": [
          "idle_timeout_row subtitle states it applies to v2ray and xray and sing-box ignores it",
          "ws_heartbeat_row subtitle unchanged (already 'xray only. 0 disables. ...')"
        ],
        "transitions": [
          {
            "input": "Network page rendered (any backend)",
            "state": "idle row subtitle names v2ray+xray applicability",
            "effect": "set",
            "evidence": "network-preferences-ui spec.md ADDED 'Network settings state backend applicability' / 'Idle timeout note' scenario; connIdle emitted for V2ray and Xray at crates/core/src/config/v2ray.rs:96-98, pinned by test_policy_conn_idle_from_settings (v2ray.rs:1046-1053); no connIdle in singbox.rs"
          },
          {
            "input": "Network page rendered",
            "state": "WebSocket ping row states xray only",
            "effect": "no-op",
            "evidence": "network-preferences-ui spec.md 'WebSocket ping note' scenario already met by subtitle at network.rs:147 ('xray only. 0 disables. Keeps NATs...')"
          },
          {
            "input": "backend switched",
            "state": "idle row stays live and sensitive",
            "effect": "no-op",
            "evidence": "design.md Decisions: 'Notes as row subtitles' — values still matter after a backend switch; rejected making rows insensitive"
          }
        ],
        "forbidden": [
          "making idle_timeout_row insensitive or hidden (drops saved values from view)",
          "altering ws_heartbeat_row subtitle or its .sensitive(backend_type == BackendType::Xray) gate",
          "subtitle claiming sing-box honors the idle timeout"
        ],
        "seeding": [
          "subtitle set at row construction following the ws_heartbeat_row subtitle pattern (network.rs:143-156); no automated UI assertion — coder cites the row source in the chunk report (preseal+guard)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.1 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui",
      "coder": "rust-coder"
    },
    {
      "id": "tun-hijack-note",
      "taskIds": [
        "3.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "tun-hijack-note",
      "shard": "tunnote",
      "pkgDirs": [
        "crates/ui/src/preferences/"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.2",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "hijack_row",
          "anchor": "title(\"DNS hijack\")",
          "change": "Add .subtitle: hijack routes captured DNS to backend resolver; native and disabled behave the same (no core edits)"
        }
      ],
      "contract": {
        "states": [
          "hijack_row subtitle explains hijack routes captured DNS to the backend resolver while native and disabled both leave DNS as ordinary traffic"
        ],
        "transitions": [
          {
            "input": "TUN page rendered (sing-box or xray)",
            "state": "subtitle states native and disabled currently behave the same",
            "effect": "set",
            "evidence": "tun-preferences-ui spec.md ADDED 'DNS hijack mode effect is explained' / 'Hijack row note' scenario; generators compare only Hijack (v2ray.rs:147, :515; singbox.rs:652; capture decision crates/ui/src/connection.rs:391-393), pinned by test_xray_tun_native_and_disabled_skip_hijack (v2ray.rs:2651, loops Native and Disabled) and test_singbox_no_hijack_dns_rule_when_hijack_mode_native (singbox.rs:1099)"
          },
          {
            "input": "user selects native vs disabled",
            "state": "generated config identical",
            "effect": "no-op",
            "evidence": "code path untouched by this change; regression floor = pinned core tests above"
          }
        ],
        "forbidden": [
          "changing DnsHijackMode, the combo model entries, hijack_to_index, or capture logic",
          "removing the native entry (design.md Non-Goal)",
          "subtitle implying native changes routing or blocks DNS"
        ],
        "seeding": [
          "subtitle set at hijack_row construction (tun.rs:108-116 pattern); automated coverage stays on the core generators via tasks.md 3.2 narrowed run (timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.2 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-core hijack -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "dns-detour-note",
      "taskIds": [
        "3.3"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "dns-detour-note",
      "shard": "dnsnote",
      "pkgDirs": [
        "crates/ui/src/preferences/"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/preferences/dns.rs",
          "symbol": "build_dns_server_dialog / detour_combo + mod tests",
          "anchor": "let detour_combo = adw::ComboRow::builder()",
          "change": "pub(crate) fn detour_note(backend: BackendType) -> Option<&'static str> + unit tests in existing #[cfg(test)] mod tests; dialog sets subtitle iff Some; .visible(honors_detour) untouched"
        }
      ],
      "contract": {
        "states": [
          "detour_note(BackendType::SingBox) = Some(sing-box note: proxy sends through the proxy, direct dials directly)",
          "detour_note(BackendType::Xray) = Some(xray note: only direct has an effect, and only while TUN is enabled)",
          "detour_note(BackendType::V2ray) = None (detour_combo hidden, no subtitle)",
          "dialog applies the subtitle iff Some"
        ],
        "transitions": [
          {
            "input": "BackendType::SingBox",
            "state": "Some(...)",
            "effect": "set",
            "evidence": "dns-preferences-ui spec.md ADDED 'Detour applicability is explained per backend' / sing-box scenario; sing-box emits `detour` on the server both ways (comment at dns.rs:1196-1198)"
          },
          {
            "input": "BackendType::Xray",
            "state": "Some(...)",
            "effect": "set",
            "evidence": "dns-preferences-ui spec.md xray scenario; xray honors a detour only as direct under TUN (crates/core/src/config/v2ray.rs:783-787), pinned by test_dns_direct_rule_precedes_the_internal_and_hijack_rules (v2ray.rs:2754, requires tun.enabled)"
          },
          {
            "input": "BackendType::V2ray",
            "state": "None -> row hidden, no subtitle",
            "effect": "clear",
            "evidence": "honors_detour = matches!(backend, SingBox | Xray) at dns.rs:1197-1198 excludes v2ray; v2ray has neither mechanism (dns.rs:1196-1198 comment)"
          }
        ],
        "forbidden": [
          "BackendType::V2ray renders detour_combo or gets a subtitle",
          "xray note claiming `proxy` detour works",
          "detour_note allocating (signature is Option<&'static str>; no String, no format!)",
          "changing honors_detour or detour_combo visibility logic",
          "unit tests instantiating the dialog, GTK, or DnsRenderCtx instead of calling the pure fn"
        ],
        "seeding": [
          "direct call detour_note(BackendType::...) inside the existing #[cfg(test)] mod tests in crates/ui/src/preferences/dns.rs (:1799); no dialog, no GTK init, no async"
        ],
        "budgets": [
          "narrowed run cargo test -p v2ray-rs-ui detour_note -- --test-threads=4 under 30s (pure fn)",
          "no new dependencies, no I/O in tests"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.3 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "cargo test -p v2ray-rs-ui detour_note -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "changelog",
      "taskIds": [
        "3.4"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "changelog-entry",
      "shard": "changelog",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.4",
          "file": "CHANGELOG.md",
          "symbol": "## [Unreleased] ### Added",
          "anchor": "## [Unreleased]",
          "change": "One bullet under ### Added: backend-applicability notes on Network/TUN/DNS preference rows"
        }
      ],
      "contract": {
        "states": [
          "[Unreleased] ### Added carries exactly one new bullet describing the backend-applicability notes on Network/TUN/DNS preference rows"
        ],
        "transitions": [
          {
            "input": "preference notes land",
            "state": "one Added bullet",
            "effect": "set",
            "evidence": "proposal.md Impact (CHANGELOG.md: one [Unreleased] line for the preference notes); tasks.md 3.4; existing Added bullets CHANGELOG.md:9-12"
          }
        ],
        "forbidden": [
          "new version heading or link refs",
          "bullets for the docs/ARCHITECTURE.md and CLAUDE.md edits"
        ],
        "seeding": [
          "none: prose"
        ],
        "budgets": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.4 per seam contract"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make check",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "arch-doc-prose",
      "tasks": [
        "1.1",
        "1.2",
        "1.3",
        "1.4",
        "1.5"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. Prose-only edits to docs/ARCHITECTURE.md: layout/Logging fallback paths (:404-440), ProbeRunner role (:149-150), per-call helper timeouts (:133-135), new File-only settings subsection, per-backend applicability notes. No code change, no automated test; verification = rendered-section read against AppPaths path helpers and the generator lines cited per row.",
      "contract": {
        "states": [
          "layout headings read $XDG_STATE_HOME/v2ray-rs/ (else ~/.local/share/v2ray-rs/state/) and $XDG_RUNTIME_DIR/v2ray-rs/ (else ~/.local/share/v2ray-rs/runtime/) with the rule in one sentence",
          "ProbeRunner described as the Real Delay probe runner, not a pre-connect binary verifier",
          "helper timeouts listed per call: device wait DEVICE_TIMEOUT, xray-up/xray-down and recover HELPER_TIMEOUT, all 10s (recover_tun_session app.rs:3682 uses HELPER_TIMEOUT; no RECOVER_TIMEOUT exists)",
          "File-only settings subsection lists tun.address_v6, auto_update_geodata, geodata_update_interval_secs, backend.config_output_dir with TOML keys",
          "TUN/DNS prose carries applicability notes for idle timeout, DNS detour, DNS hijack modes"
        ],
        "transitions": [
          {
            "input": "edit on-disk layout + Logging sections (ARCHITECTURE.md:404-440)",
            "state": "both XDG and data_dir fallback forms shown; logs under <state_dir>/logs",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99 (runtime_dir = $XDG_RUNTIME_DIR/<qualifier> else data_dir/runtime; state_dir = $XDG_STATE_HOME/<qualifier> else data_dir/state); file list mod.rs:218-243 per design.md; openspec/specs/runtime-profiles/spec.md:71-93"
          },
          {
            "input": "edit ProbeRunner sentence (ARCHITECTURE.md:149-150)",
            "state": "states its only user is measure_real_delay, matching ARCHITECTURE.md:85",
            "effect": "set",
            "evidence": "crates/subscription/src/real_delay.rs:121 is the sole caller (design.md Context; tasks.md 1.2 rg check)"
          },
          {
            "input": "edit helper timeout sentence (ARCHITECTURE.md:133-135)",
            "state": "per-call values, recover value read from crates/ui/src/app.rs:37 at edit time",
            "effect": "set",
            "evidence": "HELPER_TIMEOUT/DEVICE_TIMEOUT 10s at crates/process/src/tun.rs:13-15; RECOVER_TIMEOUT 5s at crates/ui/src/app.rs:37 and :2239; complete-tun-preflight owns aligning it"
          },
          {
            "input": "add File-only settings subsection",
            "state": "four settings listed with TOML keys and no-widget claim",
            "effect": "set",
            "evidence": "tun.address_v6 used only at crates/ui/src/connection.rs:383; auto_update_geodata / geodata_update_interval_secs only at crates/ui/src/geodata_service.rs:28-29; backend.config_output_dir copied at crates/ui/src/preferences/network.rs:501 and preferences/mod.rs:246, honored at crates/core/src/config/writer.rs:21"
          },
          {
            "input": "add applicability notes next to TUN/DNS prose",
            "state": "idle timeout v2ray+xray only; detour xray direct-only-under-TUN; hijack native==disabled",
            "effect": "set",
            "evidence": "connIdle emitted at crates/core/src/config/v2ray.rs:96-98 (pinned by test_policy_conn_idle_from_settings, v2ray.rs:1046-1053), absent in singbox.rs; xray detour-only-direct-under-TUN v2ray.rs:783-787; hijack compared only against Hijack in v2ray.rs:147 and :515 and singbox.rs:652"
          }
        ],
        "forbidden": [
          "documenting only the XDG form (rejected alternative in design.md Decisions)",
          "stating a per-call bound without the constant that enforces it, or inventing a RECOVER_TIMEOUT constant (none exists; values re-read at edit time)",
          "listing a setting as file-only that has a widget, or omitting one of the four",
          "any non-doc file touched by this seam"
        ],
        "seeding": [
          "none: prose; review reads the rendered section against AppPaths helpers (tasks.md 1.1)"
        ],
        "budgets": [
          "none: no runtime behavior"
        ]
      },
      "codeTasks": [
        "1.1 rewrite layout + Logging sections with both path forms",
        "1.2 replace ProbeRunner sentence with Real Delay role",
        "1.3 rewrite timeout sentence per call, reading RECOVER_TIMEOUT at crates/ui/src/app.rs:37 at edit time",
        "1.4 add File-only settings subsection (4 settings + TOML keys)",
        "1.5 add per-backend applicability notes, cross-checked against the generator lines cited in the transitions"
      ],
      "entry": []
    },
    {
      "id": "claude-md-persistence",
      "tasks": [
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. One-bullet edit to the CLAUDE.md persistence summary (~:42): name state_dir ($XDG_STATE_HOME/<qualifier> else data_dir/state) as the home of latency_snapshot.json, tun_session.json, and logs/. No other CLAUDE.md edits (design.md Non-Goal: no full reconciliation).",
      "contract": {
        "states": [
          "persistence bullet names state_dir for latency_snapshot.json, tun_session.json, logs/ with the fallback rule"
        ],
        "transitions": [
          {
            "input": "edit persistence summary bullet (CLAUDE.md:~42)",
            "state": "state_dir named with both forms; file list unchanged otherwise",
            "effect": "set",
            "evidence": "crates/core/src/persistence/mod.rs:88-99; design.md Context (CLAUDE.md:42 lists the files after the ~/.local/share sentence without naming state_dir)"
          }
        ],
        "forbidden": [
          "any other CLAUDE.md edit (design.md Non-Goal)",
          "contradicting docs/ARCHITECTURE.md layout wording"
        ],
        "seeding": [
          "none: prose"
        ],
        "budgets": [
          "none"
        ]
      },
      "codeTasks": [
        "2.1 amend the persistence bullet only"
      ],
      "entry": []
    },
    {
      "id": "network-idle-note",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. Subtitle edit on idle_timeout_row (crates/ui/src/preferences/network.rs:127-139) naming v2ray and xray, mirroring the ws_heartbeat_row subtitle pattern at :147. Sites: no test file — row text is not asserted headless; regression floor is the existing suite. The ADDED spec's WebSocket ping scenario is already satisfied by network.rs:147 and is verify-only here.",
      "contract": {
        "states": [
          "idle_timeout_row subtitle states it applies to v2ray and xray and sing-box ignores it",
          "ws_heartbeat_row subtitle unchanged (already 'xray only. 0 disables. ...')"
        ],
        "transitions": [
          {
            "input": "Network page rendered (any backend)",
            "state": "idle row subtitle names v2ray+xray applicability",
            "effect": "set",
            "evidence": "network-preferences-ui spec.md ADDED 'Network settings state backend applicability' / 'Idle timeout note' scenario; connIdle emitted for V2ray and Xray at crates/core/src/config/v2ray.rs:96-98, pinned by test_policy_conn_idle_from_settings (v2ray.rs:1046-1053); no connIdle in singbox.rs"
          },
          {
            "input": "Network page rendered",
            "state": "WebSocket ping row states xray only",
            "effect": "no-op",
            "evidence": "network-preferences-ui spec.md 'WebSocket ping note' scenario already met by subtitle at network.rs:147 ('xray only. 0 disables. Keeps NATs...')"
          },
          {
            "input": "backend switched",
            "state": "idle row stays live and sensitive",
            "effect": "no-op",
            "evidence": "design.md Decisions: 'Notes as row subtitles' — values still matter after a backend switch; rejected making rows insensitive"
          }
        ],
        "forbidden": [
          "making idle_timeout_row insensitive or hidden (drops saved values from view)",
          "altering ws_heartbeat_row subtitle or its .sensitive(backend_type == BackendType::Xray) gate",
          "subtitle claiming sing-box honors the idle timeout"
        ],
        "seeding": [
          "subtitle set at row construction following the ws_heartbeat_row subtitle pattern (network.rs:143-156); no automated UI assertion — coder cites the row source in the chunk report (preseal+guard)"
        ]
      },
      "codeTasks": [
        "3.1 set idle_timeout_row subtitle; confirm ws_heartbeat_row already satisfies the WS ping scenario (no edit)"
      ],
      "entry": []
    },
    {
      "id": "tun-hijack-note",
      "tasks": [
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. Subtitle edit on hijack_row (crates/ui/src/preferences/tun.rs:108-116) stating native and disabled behave the same. Sites: no new test file; the coder may NOT touch crates/core — the pinned generator tests there are the read-only regression floor this note is backed by: test_singbox_tun_hijack_dns_rule_when_dns_enabled_and_hijack_mode (singbox.rs:1003), test_singbox_no_hijack_dns_rule_when_hijack_mode_native (singbox.rs:1099), test_xray_tun_hijack_emits_dns_out_and_udp53_rule (v2ray.rs:2630), test_xray_tun_native_and_disabled_skip_hijack (v2ray.rs:2651), test_xray_tun_exclusion_precedes_dns_hijack (v2ray.rs:2369).",
      "contract": {
        "states": [
          "hijack_row subtitle explains hijack routes captured DNS to the backend resolver while native and disabled both leave DNS as ordinary traffic"
        ],
        "transitions": [
          {
            "input": "TUN page rendered (sing-box or xray)",
            "state": "subtitle states native and disabled currently behave the same",
            "effect": "set",
            "evidence": "tun-preferences-ui spec.md ADDED 'DNS hijack mode effect is explained' / 'Hijack row note' scenario; generators compare only Hijack (v2ray.rs:147, :515; singbox.rs:652; capture decision crates/ui/src/connection.rs:391-393), pinned by test_xray_tun_native_and_disabled_skip_hijack (v2ray.rs:2651, loops Native and Disabled) and test_singbox_no_hijack_dns_rule_when_hijack_mode_native (singbox.rs:1099)"
          },
          {
            "input": "user selects native vs disabled",
            "state": "generated config identical",
            "effect": "no-op",
            "evidence": "code path untouched by this change; regression floor = pinned core tests above"
          }
        ],
        "forbidden": [
          "changing DnsHijackMode, the combo model entries, hijack_to_index, or capture logic",
          "removing the native entry (design.md Non-Goal)",
          "subtitle implying native changes routing or blocks DNS"
        ],
        "seeding": [
          "subtitle set at hijack_row construction (tun.rs:108-116 pattern); automated coverage stays on the core generators via tasks.md 3.2 narrowed run (timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4)"
        ]
      },
      "codeTasks": [
        "3.2 set hijack_row subtitle; run the pinned core hijack tests unchanged as floor"
      ],
      "entry": []
    },
    {
      "id": "dns-detour-note",
      "tasks": [
        "3.3"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. The only new automated assertions in this change: pure fn detour_note(backend) -> Option<&'static str> in crates/ui/src/preferences/dns.rs plus unit tests in that file's existing #[cfg(test)] mod tests (:1799). Sites (complete): the coder may add/change tests ONLY in crates/ui/src/preferences/dns.rs tests module; crates/core pinned tests named by task 3.2 are floor, read-only. detour_note unit tests are the FIRST codeTasks of this seam; v2ray returns None and the row stays hidden via the existing honors_detour gate (dns.rs:1197-1198, detour_combo .visible(honors_detour) at :1258-1264).",
      "contract": {
        "states": [
          "detour_note(BackendType::SingBox) = Some(sing-box note: proxy sends through the proxy, direct dials directly)",
          "detour_note(BackendType::Xray) = Some(xray note: only direct has an effect, and only while TUN is enabled)",
          "detour_note(BackendType::V2ray) = None (detour_combo hidden, no subtitle)",
          "dialog applies the subtitle iff Some"
        ],
        "transitions": [
          {
            "input": "BackendType::SingBox",
            "state": "Some(...)",
            "effect": "set",
            "evidence": "dns-preferences-ui spec.md ADDED 'Detour applicability is explained per backend' / sing-box scenario; sing-box emits `detour` on the server both ways (comment at dns.rs:1196-1198)"
          },
          {
            "input": "BackendType::Xray",
            "state": "Some(...)",
            "effect": "set",
            "evidence": "dns-preferences-ui spec.md xray scenario; xray honors a detour only as direct under TUN (crates/core/src/config/v2ray.rs:783-787), pinned by test_dns_direct_rule_precedes_the_internal_and_hijack_rules (v2ray.rs:2754, requires tun.enabled)"
          },
          {
            "input": "BackendType::V2ray",
            "state": "None -> row hidden, no subtitle",
            "effect": "clear",
            "evidence": "honors_detour = matches!(backend, SingBox | Xray) at dns.rs:1197-1198 excludes v2ray; v2ray has neither mechanism (dns.rs:1196-1198 comment)"
          }
        ],
        "forbidden": [
          "BackendType::V2ray renders detour_combo or gets a subtitle",
          "xray note claiming `proxy` detour works",
          "detour_note allocating (signature is Option<&'static str>; no String, no format!)",
          "changing honors_detour or detour_combo visibility logic",
          "unit tests instantiating the dialog, GTK, or DnsRenderCtx instead of calling the pure fn"
        ],
        "seeding": [
          "direct call detour_note(BackendType::...) inside the existing #[cfg(test)] mod tests in crates/ui/src/preferences/dns.rs (:1799); no dialog, no GTK init, no async"
        ],
        "budgets": [
          "narrowed run cargo test -p v2ray-rs-ui detour_note -- --test-threads=4 under 30s (pure fn)",
          "no new dependencies, no I/O in tests"
        ]
      },
      "codeTasks": [
        "write unit tests first in dns.rs tests module: detour_note_singbox (Some, names proxy+direct), detour_note_xray (Some, names direct-only + TUN), detour_note_v2ray_none (None)",
        "implement pub(crate) fn detour_note(backend: BackendType) -> Option<&'static str> returning the two notes above and None",
        "wire the dialog: if let Some(note) = detour_note(backend) { detour_combo.set_subtitle(note) } in show_dns_server_dialog (dns.rs:1258-1264), leaving .visible(honors_detour) untouched"
      ],
      "entry": []
    },
    {
      "id": "changelog-entry",
      "tasks": [
        "3.4"
      ],
      "summary": "NO-RED-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. NO-TESTER-WAIVER: Rust: rust-coder writes tests+code; closure by waiver with preseal+guard. One bullet under the existing [Unreleased] ### Added (CHANGELOG.md:8-12) for the three preference notes. Keep-a-Changelog shape already in file; no new version heading; doc-only edits get no entry (proposal Impact).",
      "contract": {
        "states": [
          "[Unreleased] ### Added carries exactly one new bullet describing the backend-applicability notes on Network/TUN/DNS preference rows"
        ],
        "transitions": [
          {
            "input": "preference notes land",
            "state": "one Added bullet",
            "effect": "set",
            "evidence": "proposal.md Impact (CHANGELOG.md: one [Unreleased] line for the preference notes); tasks.md 3.4; existing Added bullets CHANGELOG.md:9-12"
          }
        ],
        "forbidden": [
          "new version heading or link refs",
          "bullets for the docs/ARCHITECTURE.md and CLAUDE.md edits"
        ],
        "seeding": [
          "none: prose"
        ],
        "budgets": [
          "none"
        ]
      },
      "codeTasks": [
        "3.4 append one bullet to [Unreleased] ### Added"
      ],
      "entry": []
    }
  ],
  "requirements": [
    {
      "shall": "The Network preferences page SHALL state, on each setting that only some backends honor, which backends apply it.",
      "tests": [
        "test_policy_conn_idle_from_settings (backing evidence; row text itself is UI-verified)"
      ]
    },
    {
      "shall": "The idle connection timeout SHALL be noted as applying to v2ray and xray only.",
      "tests": [
        "test_policy_conn_idle_from_settings (connIdle v2ray/xray only; row subtitle UI-verified)"
      ]
    },
    {
      "shall": "The WebSocket ping interval SHALL be noted as applying to xray only.",
      "tests": [
        "already satisfied: network.rs ws_heartbeat_row subtitle (verify-only, no automated test)"
      ]
    },
    {
      "shall": "The TUN page SHALL explain the DNS hijack modes next to the selector: `hijack` routes DNS queries captured by the tunnel to the backend's resolver, while `native` and `disabled` both leave DNS queries to be carried as ordinary traffic without being answered by the backend's resolver.",
      "tests": [
        "test_xray_tun_native_and_disabled_skip_hijack",
        "test_singbox_no_hijack_dns_rule_when_hijack_mode_native"
      ]
    },
    {
      "shall": "The detour control in the DNS server dialog SHALL state its effect for the selected backend.",
      "tests": [
        "detour_note_singbox",
        "detour_note_xray",
        "detour_note_v2ray_none"
      ]
    },
    {
      "shall": "For xray it SHALL state that only `direct` has an effect and only while TUN is enabled.",
      "tests": [
        "detour_note_xray",
        "test_dns_direct_rule_precedes_the_internal_and_hijack_rules"
      ]
    },
    {
      "shall": "For sing-box it SHALL state that `proxy` sends the server through the proxy and `direct` dials it directly.",
      "tests": [
        "detour_note_singbox"
      ]
    }
  ],
  "testHarness": [
    "detour_note_singbox / detour_note_xray / detour_note_v2ray_none (new, crates/ui/src/preferences/dns.rs tests module)",
    "test_policy_conn_idle_from_settings (v2ray.rs:1046) — backs the idle note",
    "test_singbox_tun_hijack_dns_rule_when_dns_enabled_and_hijack_mode (singbox.rs:1003)",
    "test_singbox_no_hijack_dns_rule_when_hijack_mode_native (singbox.rs:1099)",
    "test_xray_tun_hijack_emits_dns_out_and_udp53_rule (v2ray.rs:2630)",
    "test_xray_tun_native_and_disabled_skip_hijack (v2ray.rs:2651) — backs the hijack note",
    "test_xray_tun_exclusion_precedes_dns_hijack (v2ray.rs:2369)",
    "test_dns_direct_rule_precedes_the_internal_and_hijack_rules (v2ray.rs:2754) — backs the xray detour note"
  ],
  "floor": "make lint && timeout 10m cargo test --workspace -- --test-threads=4",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1
  }
}
```
