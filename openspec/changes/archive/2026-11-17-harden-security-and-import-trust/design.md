# Design

## Context

See `proposal.md` for motivation. Five trust boundaries currently allow unintended network disclosure or unvalidated input execution:
1. `crates/subscription/src/real_delay.rs` constructs `node_proxy_client` with `reqwest::Proxy::http(...)` and leaves `reqwest` environment-proxy lookup enabled, so `https://` requests (`EGRESS_ECHO_URL`, `ServiceCheck` URLs) bypass the per-node loopback proxy inbound and dial directly from the host.
2. `crates/netctl/src/net.rs` gates IPv6 port-53 DNS capture (`pref 8999`) on `v6.is_some()` instead of `v6_rules`, while `crates/netctl/src/main.rs` allows `--force-legacy` for any caller able to execute the `0750` `cap_net_admin+ep` helper.
3. `crates/subscription/src/json_import.rs` enables imported DNS/routing profiles automatically on remote URLs, accepts `allowInsecure` TLS outbounds, maps `"protocol": "dns"` outbounds to `Direct`, strips unsupported predicates from multi-condition Xray rules (turning scoped rules like `udp + port 443 -> block` into broad `udp -> block` or empty catch-alls), and skips domain/CIDR/DNS validation on imported profiles.
4. `crates/core/src/geodata.rs` interpolates rule-set tags into paths/URLs without validating `[a-z0-9_-]`, and `crates/subscription/src/fetch.rs` follows HTTP redirects without checking private/local targets.
5. `crates/run/src/main.rs` uses a small environment denylist and does not read back `getresuid`/`getresgid`, `pkg/archlinux/{,bin/}PKGBUILD` puts `4750` into the archive payload before `systemd-sysusers` runs, and `crates/process/src/privilege.rs` never creates `v2ray-rs-bypass` during `pkexec` grant.

## Goals / Non-Goals

**Goals:**
- Guarantee zero direct-egress leaks on `https://` Real Delay egress and geo-availability probes.
- Fail closed on IPv6 port-53 LAN DNS under strict xray TUN and block non-root `--force-legacy` invocations.
- Treat remote subscription JSON profiles as untrusted until user opt-in, keep `RoutingRule` single-condition without silent rule widening, and enforce `core` validation across all imported profile fields.
- Harden `v2ray-rs-run`, `pkexec` grant, Arch packaging, and installer scripts without adding dependencies.

**Non-Goals:**
- Adding multi-condition conjunctive `RoutingRule` variants (explicitly rejected for KISS/YAGNI; multi-axis Xray rules are dropped whole with a warning).
- Performing DNS pre-resolution inside `reqwest` redirect hooks (blocking literal private/loopback/link-local/ULA/unspecified IPs and `localhost` hostnames covers redirect-to-metadata/loopback SSRF without introducing DNS rebinding complexity in the fetcher).

## Decisions

### 1. Proxy all schemes and disable ambient env proxies in `node_proxy_client`
- **Choice:** Replace `reqwest::Proxy::http(...)` with `reqwest::Proxy::all(format!("http://127.0.0.1:{inbound_port}"))` and chain `.no_proxy()` on `reqwest::Client::builder()`. In `RealDelaySession`, run `detect_egress` only for indices where the delay probe returned `Some(_)`.
- **Rationale:** `Proxy::http` matches only `http://` targets in `reqwest`, leaving `https://` targets direct. `.no_proxy()` disables `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` env inspection so `NO_PROXY=*` cannot bypass the loopback inbound.
- **Alternative considered:** Changing echo URLs to `http://`. Rejected because `ServiceCheck` targets (`notebooklm.google.com`, `gemini.google.com`) require `https://`.

### 2. Align IPv6 DNS capture with `v6_rules` and gate `--force-legacy` on `geteuid() == 0`
- **Choice:** In `netctl::net::xray_up`, install `Inet6` port-53 capture rules whenever `v6_rules && cfg.capture_dns` (matching `pref 9000..9002`), and in `netctl::main`, check `nix::unistd::geteuid().is_root()` (or `libc::geteuid() == 0`) when `--force-legacy` is passed.
- **Rationale:** `v2ray-rs-netctl` carries `cap_net_admin+ep` (not setuid root), so desktop invocations run with the unprivileged user's `euid != 0`, whereas `sudo v2ray-rs-netctl --force-legacy recover ...` runs with `euid == 0`. Checking `euid == 0` cleanly separates operator recovery from unprivileged `v2ray-rs` group processes.

### 3. Opt-in remote profiles, strict single-axis Xray rule import, and `allowInsecure` rejection
- **Choice:**
  - `Subscription::new_from_url` sets `use_imported_profile: false`; `Subscription::new_from_file` and pasted JSON bundle import set `use_imported_profile: true`; `#[serde(default)]` (replacing `default_true`) defaults missing JSON fields to `false` while preserving explicit `true`/`false`.
  - Expose an `adw::SwitchRow` / row menu toggle in `SubscriptionsPage` when `subscription.imported_profile.is_some()` so users can opt in per URL subscription, with a toast/banner on import when a remote profile is detected.
  - Reject TLS outbounds/URIs with `allowInsecure == true` (`allowInsecure=1`) during import, recording a warning in `ImportResult` / `JsonImportOutcome` so remaining valid nodes still import.
  - In `json_import.rs`: skip outbounds with `"protocol": "dns"` when building outbound tag maps; ignore `"network": "tcp,udp"` (or `"udp,tcp"`) when combined with a single `domain` or `ip` axis; drop any Xray routing rule that combines >1 real matcher axis (`network: "udp"` + `port`, `domain` + `ip`, `port` alone, `inboundTag`, `source`, `protocol`, `attrs`, `regexp:`) as a whole with a warning; run `RoutingRule::validate`, `DnsConfig::validate`, `DnsRuleMatch::validate`, and `HostOverride` validation on imported items AND inside `ConfigWriter::validate_runtime_inputs`.
- **Rationale:** Preserves the compact single-condition `RoutingRule` model while eliminating silent rule widening, `dns-out -> direct` misrouting, and unvalidated remote DNS/routing injection.

### 4. Strict environment allowlist and `getresuid`/`getresgid` verification in `v2ray-rs-run`, plus safe packaging
- **Choice:**
  - In `crates/run/src/main.rs`, after `setgroups(0, null)`, `setresgid(gid, gid, gid)`, and `setresuid(uid, uid, uid)`, call `getresgid` and `getresuid` and verify all six IDs equal the `v2ray-rs-bypass` passwd entry. Clear all environment variables and retain only `PATH` (sanitized fallback `/usr/local/bin:/usr/bin`), `TERM`, `LANG`, `LC_*`, `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, and `XAUTHORITY`, setting `USER` and `LOGNAME` from `v2ray-rs-bypass` (or keeping `HOME` read-only safe) without passing `LD_*`, `PYTHONPATH`, `GIO_EXTRA_MODULES`, `GTK_MODULES`, or `GCONV_PATH`.
  - In `pkg/archlinux/PKGBUILD` and `pkg/archlinux/bin/PKGBUILD`, install `v2ray-rs-run` as `0755` inside `package()` and apply `chown root:v2ray-rs` + `chmod 4750` in `v2ray-rs.install` (`post_install`/`post_upgrade`) after `systemd-sysusers` runs.
  - In `crates/process/src/privilege.rs`, extend the `pkexec` grant script to create system user `v2ray-rs-bypass` (`useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin v2ray-rs-bypass` when `id -u v2ray-rs-bypass` fails) whenever `v2ray-rs-run` is present.
  - Fix `scripts/install-appimage.sh` helper tarball asset naming to match release tarballs (`v2ray-rs-${tag}-x86_64.tar.gz`) and avoid installing broken unprivileged copies of `v2ray-rs-run` in `scripts/install.sh` `--user` mode.

### 5. Redact headers and tokenized DNS URLs in `config/redact.rs`
- **Choice:** Extend recursive JSON redaction in `crates/core/src/config/redact.rs` to mask values under `headers` maps matching `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, or `x-api-key` (case-insensitive), and redact query strings plus non-`/dns-query` path segments in `https://` / `h3://` DNS server URLs (`server`, `address`, or URL strings under `dns.servers`). In `crates/ui/src/config_preview.rs`, keep `self.reveal_raw` unchanged across `ConfigPreviewMsg::Refresh`.

## Risks / Trade-offs

- **[Risk] Existing URL subscriptions that relied on implicit `use_imported_profile` default** -> Existing `subscriptions.json` files already serialize `"use_imported_profile": true` or `false` explicitly (`#[serde(default)]` only affects newly created URL subscriptions or hand-edited files missing the key); users importing new URL bundles get a clear UI toggle and import notice.
- **[Risk] Dropping `allowInsecure: true` nodes reduces node count on misconfigured providers** -> Surfaced import warnings name every skipped node and reason instead of failing silently at Xray connect time or exposing users to MITM on v2ray/sing-box.
- **[Risk] Strict env allowlist in `v2ray-rs-run` could omit custom app variables** -> Desktop GUI/CLI tools (`curl`, browsers, `ssh`) only need display/session/locale/`PATH` vars; users needing extra env vars can pass `env KEY=VAL cmd` explicitly as arguments to `v2ray-rs-run`.

## Migration Plan

1. Ship `Subscription` default change and validator checks; existing persisted `"use_imported_profile"` booleans remain intact.
2. Update Arch `PKGBUILD`s and `.install` script together so package upgrades re-apply `root:v2ray-rs 4750` cleanly in `post_upgrade`.
3. Rollback requires no data migration because no persisted schema field is removed.
