//! Privileged idempotency test for the route helper. Requires root and is gated
//! behind the `privileged-tests` feature, so it never runs under `cargo test`/
//! `-short`. Everything runs inside a throwaway network namespace, so the split
//! routes the helper installs never touch the host routing table.
//! Run with: `sudo -E cargo test -p v2ray-rs-netctl --features privileged-tests`.
#![cfg(feature = "privileged-tests")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_v2ray-rs-netctl");
const NS: &str = "nctl-test-ns";
/// Second namespace so the DNS-capture test can run alongside the idempotency
/// one without the two fighting over the same rules.
const NS_DNS: &str = "nctl-dns-ns";
const NS_STRICT: &str = "nctl-strict-ns";
const NS_CLEAR: &str = "nctl-clear-ns";
const NS_REUP: &str = "nctl-reup-ns";
const NS_GONE: &str = "nctl-gone-ns";
const NS_EXCL: &str = "nctl-excl-ns";
const NS_EXCL_REUP: &str = "nctl-exclreup-ns";
const IFACE: &str = "nctltest0";
/// Stand-in for the physical uplink that owns the `main` default route.
const MAIN_IFACE: &str = "nctlmain0";
const ADDR: &str = "172.31.255.1/30";
const ADDR6: &str = "fd00:ffff::1/64";

fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Runs `ip <args>` inside the given network namespace.
fn ip_in(ns: &str, args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", ns, "ip"];
    full.extend_from_slice(args);
    run("ip", &full)
}

fn ip_in_ns(args: &[&str]) -> bool {
    ip_in(NS, args)
}

/// Runs `ip <args>` inside the given namespace and returns its stdout.
fn ip_in_output(ns: &str, args: &[&str]) -> String {
    let mut full = vec!["netns", "exec", ns, "ip"];
    full.extend_from_slice(args);
    Command::new("ip")
        .args(&full)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn ip_in_ns_output(args: &[&str]) -> String {
    ip_in_output(NS, args)
}

/// Runs `ip <args>` inside the given namespace and returns whether it succeeded
/// together with its stdout and stderr, since `ip route get` reports routing
/// errors only on stderr.
fn ip_in_full(ns: &str, args: &[&str]) -> (bool, String) {
    let mut full = vec!["netns", "exec", ns, "ip"];
    full.extend_from_slice(args);
    match Command::new("ip").args(&full).output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), text)
        }
        Err(e) => (false, e.to_string()),
    }
}

/// Runs the netctl binary inside the given namespace, so its netlink socket
/// operates on that namespace's network stack rather than the host's.
fn netctl_in(ns: &str, args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", ns, BIN, "--force-legacy"];
    full.extend_from_slice(args);
    run("ip", &full)
}

fn netctl(args: &[&str]) -> bool {
    netctl_in(NS, args)
}

/// Whether the test device currently exists inside the namespace.
fn device_exists() -> bool {
    ip_in_ns(&["link", "show", IFACE])
}

/// Deletes the namespace (and everything in it) on drop, so a panicking
/// assertion can never leak the namespace or the test device.
struct NsGuard<'a>(&'a str);
impl Drop for NsGuard<'_> {
    fn drop(&mut self) {
        let _ = run("ip", &["netns", "del", self.0]);
    }
}

#[test]
fn up_down_is_idempotent_in_namespace() {
    let _ = run("ip", &["netns", "del", NS]); // best-effort pre-clean from a prior run
    if !run("ip", &["netns", "add", NS]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS); // from here, any return or panic deletes the namespace

    // A `tun` device stands in for the one xray creates (the `dummy` driver is
    // not universally installed; the feature requires `tun` regardless).
    if !ip_in_ns(&["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }

    // up twice: an already-assigned address (EEXIST) and existing routes/rules
    // are ignored, so the second call must also succeed.
    assert!(netctl(&["xray-up", "--iface", IFACE, "--addr", ADDR]));
    assert!(netctl(&["xray-up", "--iface", IFACE, "--addr", ADDR]));

    // The fwmark bypass + capture rules and the tunnel-table route must be in
    // place: this is what keeps xray's own `direct` traffic out of the tunnel.
    let rules = ip_in_ns_output(&["rule", "show"]);
    assert!(
        rules.contains("9000:") && rules.contains("fwmark 0xff"),
        "fwmark bypass rule missing: {rules}"
    );
    assert!(
        rules.contains("9001:") && rules.contains("9002:"),
        "capture policy rules missing: {rules}"
    );
    assert!(
        !rules.contains("8998:"),
        "bypass-uid rule present without --bypass-uid: {rules}"
    );
    let tun_table = ip_in_ns_output(&["route", "show", "table", "2023"]);
    assert!(
        tun_table.contains(IFACE),
        "tun default route missing from table 2023: {tun_table}"
    );

    // Without --strict nothing fails closed: no fallback route, no IPv6 rules.
    let tun_table_v6 = ip_in_ns_output(&["-6", "route", "show", "table", "2023"]);
    assert!(
        !tun_table.contains("unreachable") && !tun_table_v6.contains("unreachable"),
        "fallback route present without --strict: {tun_table} / {tun_table_v6}"
    );
    let v6_rules = ip_in_ns_output(&["-6", "rule", "show"]);
    assert!(
        !v6_rules.contains("9002:"),
        "IPv6 capture rule present without --strict or --addr6: {v6_rules}"
    );

    // down deletes the live device; a second down is a clean no-op.
    assert!(netctl(&["xray-down", "--iface", IFACE]));
    assert!(!device_exists(), "xray-down must remove the device");
    assert!(netctl(&["xray-down", "--iface", IFACE]));

    // down also tears down the policy rules (they are not device-scoped, so the
    // device deletion alone would leave them behind).
    let rules_after = ip_in_ns_output(&["rule", "show"]);
    assert!(
        !rules_after.contains("9000:")
            && !rules_after.contains("9001:")
            && !rules_after.contains("9002:"),
        "policy rules leaked after xray-down: {rules_after}"
    );

    // recover must remove a *live* leftover device, not merely succeed once it is
    // already gone: recreate it, bring it up, then recover and confirm removal.
    assert!(ip_in_ns(&["tuntap", "add", "dev", IFACE, "mode", "tun"]));
    assert!(netctl(&["xray-up", "--iface", IFACE, "--addr", ADDR]));
    assert!(netctl(&["recover", "--xray", "--iface", IFACE]));
    assert!(!device_exists(), "recover must remove the leftover device");

    // recover is a clean no-op once the device is already gone.
    assert!(netctl(&["recover", "--xray", "--iface", IFACE]));

    // `--bypass-uid` installs a pref-8998 uidrange rule per family, which is
    // torn down alongside the capture rules on down/recover.
    assert!(ip_in_ns(&["tuntap", "add", "dev", IFACE, "mode", "tun"]));
    assert!(netctl(&[
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--addr6",
        ADDR6,
        "--bypass-uid",
        "999990",
    ]));

    let v4_rules = ip_in_ns_output(&["rule", "show"]);
    assert!(
        v4_rules.contains("8998:") && v4_rules.contains("uidrange 999990-999990"),
        "bypass-uid rule missing for IPv4: {v4_rules}"
    );
    let v6_rules = ip_in_ns_output(&["-6", "rule", "show"]);
    assert!(
        v6_rules.contains("8998:") && v6_rules.contains("uidrange 999990-999990"),
        "bypass-uid rule missing for IPv6: {v6_rules}"
    );

    assert!(netctl(&["xray-down", "--iface", IFACE]));
    let v4_after = ip_in_ns_output(&["rule", "show"]);
    assert!(
        !v4_after.contains("8998:"),
        "bypass-uid rule leaked after xray-down (v4): {v4_after}"
    );
    let v6_after = ip_in_ns_output(&["-6", "rule", "show"]);
    assert!(
        !v6_after.contains("8998:"),
        "bypass-uid rule leaked after xray-down (v6): {v6_after}"
    );

    // recover --xray must also clear the bypass rule from a live device.
    assert!(ip_in_ns(&["tuntap", "add", "dev", IFACE, "mode", "tun"]));
    assert!(netctl(&[
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--bypass-uid",
        "999990",
    ]));
    assert!(netctl(&["recover", "--xray", "--iface", IFACE]));
    let recovered = ip_in_ns_output(&["rule", "show"]);
    assert!(
        !recovered.contains("8998:"),
        "bypass-uid rule leaked after recover --xray: {recovered}"
    );
}

#[test]
fn capture_dns_steers_port_53_into_the_tunnel_table() {
    let _ = run("ip", &["netns", "del", NS_DNS]);
    if !run("ip", &["netns", "add", NS_DNS]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_DNS);

    if !ip_in(NS_DNS, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }

    assert!(netctl_in(
        NS_DNS,
        &["xray-up", "--iface", IFACE, "--addr", ADDR, "--capture-dns"]
    ));

    // The pair must sit ahead of the fwmark bypass so unmarked queries reach the
    // tunnel, and must carry `fwmark 0/0xff` so the proxy's own marked queries
    // fall through to it instead of looping back into the tunnel.
    let rules = ip_in_output(NS_DNS, &["rule", "show"]);
    for proto in ["udp", "tcp"] {
        assert!(
            rules.contains(&format!(
                "fwmark 0/0xff ipproto {proto} dport 53 lookup 2023"
            )),
            "dns capture rule missing for {proto}: {rules}"
        );
    }
    assert!(
        rules.find("8999:").unwrap() < rules.find("9000:").unwrap(),
        "dns capture must be evaluated before the fwmark bypass: {rules}"
    );

    // Without the flag the rules must not appear at all.
    assert!(netctl_in(NS_DNS, &["xray-down", "--iface", IFACE]));
    assert!(ip_in(
        NS_DNS,
        &["tuntap", "add", "dev", IFACE, "mode", "tun"]
    ));
    assert!(netctl_in(
        NS_DNS,
        &["xray-up", "--iface", IFACE, "--addr", ADDR]
    ));
    let without = ip_in_output(NS_DNS, &["rule", "show"]);
    assert!(
        !without.contains("8999:"),
        "dns capture rules present without --capture-dns: {without}"
    );

    // And they are torn down with everything else, not left to blackhole DNS.
    assert!(netctl_in(NS_DNS, &["xray-down", "--iface", IFACE]));
    assert!(ip_in(
        NS_DNS,
        &["tuntap", "add", "dev", IFACE, "mode", "tun"]
    ));
    assert!(netctl_in(
        NS_DNS,
        &["xray-up", "--iface", IFACE, "--addr", ADDR, "--capture-dns"]
    ));
    assert!(netctl_in(NS_DNS, &["recover", "--xray", "--iface", IFACE]));
    let after = ip_in_output(NS_DNS, &["rule", "show"]);
    assert!(
        !after.contains("8999:"),
        "dns capture rules leaked after recover --xray: {after}"
    );
}

/// Lines of `ip [-6] route show table 2023` that are the strict fallback route.
fn fallback_lines(ns: &str, family: &str) -> Vec<String> {
    ip_in_output(ns, &[family, "route", "show", "table", "2023"])
        .lines()
        .filter(|l| l.starts_with("unreachable default"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn strict_up_installs_fallback_routes_and_v6_rules() {
    let _ = run("ip", &["netns", "del", NS_STRICT]);
    if !run("ip", &["netns", "add", NS_STRICT]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_STRICT);

    if !ip_in(NS_STRICT, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }

    assert!(netctl_in(
        NS_STRICT,
        &[
            "xray-up",
            "--iface",
            IFACE,
            "--addr",
            ADDR,
            "--bypass-uid",
            "999990",
            "--capture-dns",
            "--strict",
        ]
    ));

    for family in ["-4", "-6"] {
        let fallback = fallback_lines(NS_STRICT, family);
        assert_eq!(
            fallback.len(),
            1,
            "expected one fallback route ({family}): {fallback:?}"
        );
        assert!(
            fallback[0].contains("metric 4294967295"),
            "fallback route must carry the maximum metric ({family}): {fallback:?}"
        );
    }

    // No --addr6: the IPv6 table never gains a device route.
    let v6_table = ip_in_output(NS_STRICT, &["-6", "route", "show", "table", "2023"]);
    assert!(
        !v6_table.contains(IFACE),
        "IPv6 device route present without --addr6: {v6_table}"
    );

    let v6_rules = ip_in_output(NS_STRICT, &["-6", "rule", "show"]);
    for pref in ["8998:", "9000:", "9001:", "9002:"] {
        assert!(
            v6_rules.contains(pref),
            "IPv6 rule {pref} missing under --strict: {v6_rules}"
        );
    }
    for proto in ["ipproto udp dport 53", "ipproto tcp dport 53"] {
        assert!(
            v6_rules.contains(proto),
            "IPv6 dns capture rule missing under --strict --capture-dns ({proto}): {v6_rules}"
        );
    }
    let v4_rules = ip_in_output(NS_STRICT, &["rule", "show"]);
    assert!(
        v4_rules.contains("8999:"),
        "IPv4 dns capture rule missing: {v4_rules}"
    );
}

/// Asserts that no xray route or policy rule is left in either family.
fn assert_xray_state_cleared(ns: &str, after: &str) {
    for family in ["-4", "-6"] {
        let table = ip_in_output(ns, &[family, "route", "show", "table", "2023"]);
        assert!(
            table.trim().is_empty(),
            "table 2023 not empty after {after} ({family}): {table}"
        );
        let rules = ip_in_output(ns, &[family, "rule", "show"]);
        for pref in ["8997:", "8998:", "8999:", "9000:", "9001:", "9002:"] {
            assert!(
                !rules.contains(pref),
                "rule {pref} leaked after {after} ({family}): {rules}"
            );
        }
    }
}

#[test]
fn down_and_recover_clear_strict_state_both_families() {
    let _ = run("ip", &["netns", "del", NS_CLEAR]);
    if !run("ip", &["netns", "add", NS_CLEAR]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_CLEAR);

    let strict_up = [
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--bypass-uid",
        "999990",
        "--capture-dns",
        "--strict",
    ];

    if !ip_in(NS_CLEAR, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    assert!(netctl_in(NS_CLEAR, &strict_up));
    assert!(netctl_in(NS_CLEAR, &["xray-down", "--iface", IFACE]));
    assert!(!ip_in(NS_CLEAR, &["link", "show", IFACE]));
    assert_xray_state_cleared(NS_CLEAR, "xray-down");
    assert!(netctl_in(NS_CLEAR, &["xray-down", "--iface", IFACE]));

    assert!(ip_in(
        NS_CLEAR,
        &["tuntap", "add", "dev", IFACE, "mode", "tun"]
    ));
    assert!(netctl_in(NS_CLEAR, &strict_up));
    assert!(netctl_in(
        NS_CLEAR,
        &["recover", "--xray", "--iface", IFACE]
    ));
    assert!(!ip_in(NS_CLEAR, &["link", "show", IFACE]));
    assert_xray_state_cleared(NS_CLEAR, "recover --xray");
}

/// Sorted policy rules and table-2023 routes of both families, each line
/// prefixed with its family.
fn xray_state(ns: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for family in ["-4", "-6"] {
        for args in [
            vec![family, "rule", "show"],
            vec![family, "route", "show", "table", "2023"],
        ] {
            lines.extend(
                ip_in_output(ns, &args)
                    .lines()
                    .map(|l| format!("{family} {}", l.trim())),
            );
        }
    }
    lines.sort();
    lines
}

#[test]
fn reup_across_recreated_device_leaves_one_copy() {
    let _ = run("ip", &["netns", "del", NS_REUP]);
    if !run("ip", &["netns", "add", NS_REUP]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_REUP);

    let strict_up = [
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--bypass-uid",
        "999990",
        "--strict",
    ];

    if !ip_in(NS_REUP, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    assert!(netctl_in(NS_REUP, &strict_up));
    let fresh = xray_state(NS_REUP);

    // The rules and the fallback route outlive the device, so the second up
    // meets them already present and must not stack a second copy.
    assert!(ip_in(
        NS_REUP,
        &["tuntap", "del", "dev", IFACE, "mode", "tun"]
    ));
    assert!(ip_in(
        NS_REUP,
        &["tuntap", "add", "dev", IFACE, "mode", "tun"]
    ));
    assert!(netctl_in(NS_REUP, &strict_up));
    let reup = xray_state(NS_REUP);
    assert_eq!(reup, fresh, "state after re-up differs from a single up");

    for family in ["-4", "-6"] {
        let rules = ip_in_output(NS_REUP, &[family, "rule", "show"]);
        for pref in ["8998:", "9000:", "9001:", "9002:"] {
            let count = rules.lines().filter(|l| l.starts_with(pref)).count();
            assert_eq!(count, 1, "rule {pref} count ({family}): {rules}");
        }

        let table = ip_in_output(NS_REUP, &[family, "route", "show", "table", "2023"]);
        let mut routes: Vec<&str> = table.lines().map(str::trim).collect();
        assert!(!routes.is_empty(), "table 2023 empty ({family})");
        routes.sort_unstable();
        let total = routes.len();
        routes.dedup();
        assert_eq!(
            routes.len(),
            total,
            "duplicate table 2023 route ({family}): {table}"
        );
    }
}

#[test]
fn strict_state_refuses_unmarked_traffic_without_device() {
    let _ = run("ip", &["netns", "del", NS_GONE]);
    if !run("ip", &["netns", "add", NS_GONE]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_GONE);

    if !ip_in(NS_GONE, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    assert!(ip_in(
        NS_GONE,
        &["tuntap", "add", "dev", MAIN_IFACE, "mode", "tun"]
    ));
    assert!(ip_in(
        NS_GONE,
        &["addr", "add", "10.99.0.1/24", "dev", MAIN_IFACE]
    ));
    assert!(ip_in(NS_GONE, &["link", "set", MAIN_IFACE, "up"]));
    assert!(ip_in(
        NS_GONE,
        &["route", "add", "default", "dev", MAIN_IFACE]
    ));

    assert!(netctl_in(
        NS_GONE,
        &["xray-up", "--iface", IFACE, "--addr", ADDR, "--strict"]
    ));
    assert!(ip_in(NS_GONE, &["link", "del", IFACE]));

    // Unmarked traffic skips main's default route and lands on the fallback.
    let (ok, out) = ip_in_full(NS_GONE, &["-4", "route", "get", "198.51.100.1"]);
    assert!(
        !ok && (out.contains("No route to host") || out.contains("unreachable")),
        "unmarked IPv4 lookup must fail closed: {out}"
    );

    // The proxy's own marked sockets still reach the uplink through main.
    let (ok, out) = ip_in_full(
        NS_GONE,
        &["-4", "route", "get", "198.51.100.1", "mark", "255"],
    );
    assert!(
        ok && out.contains(&format!("dev {MAIN_IFACE}")),
        "marked IPv4 lookup must reach main: {out}"
    );

    let (ok, out) = ip_in_full(NS_GONE, &["-6", "route", "get", "2001:db8::1"]);
    assert!(
        !ok || out.contains("unreachable"),
        "unmarked IPv6 lookup must fail closed: {out}"
    );
}

/// IPv4 policy rules at the exclusion priority.
fn exclusion_lines(ns: &str) -> Vec<String> {
    ip_in_output(ns, &["-4", "rule", "show"])
        .lines()
        .filter(|l| l.starts_with("8997:"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn exclusions_route_outside_tunnel_ahead_of_dns_capture() {
    let _ = run("ip", &["netns", "del", NS_EXCL]);
    if !run("ip", &["netns", "add", NS_EXCL]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_EXCL);

    if !ip_in(NS_EXCL, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    assert!(ip_in(
        NS_EXCL,
        &["tuntap", "add", "dev", MAIN_IFACE, "mode", "tun"]
    ));
    assert!(ip_in(
        NS_EXCL,
        &["addr", "add", "10.99.0.1/24", "dev", MAIN_IFACE]
    ));
    assert!(ip_in(NS_EXCL, &["link", "set", MAIN_IFACE, "up"]));
    assert!(ip_in(
        NS_EXCL,
        &["route", "add", "default", "dev", MAIN_IFACE]
    ));

    assert!(netctl_in(
        NS_EXCL,
        &[
            "xray-up",
            "--iface",
            IFACE,
            "--addr",
            ADDR,
            "--capture-dns",
            "--exclude",
            "198.51.100.7/32",
            "--exclude",
            "192.0.2.53/32",
        ]
    ));

    let excl = exclusion_lines(NS_EXCL);
    assert_eq!(excl.len(), 2, "expected two exclusion rules: {excl:?}");
    assert!(
        excl.iter().all(|l| l.contains("lookup main")),
        "exclusion rules must look up main: {excl:?}"
    );

    let rules = ip_in_output(NS_EXCL, &["-4", "rule", "show"]);
    assert!(
        rules.find("8997:").unwrap() < rules.find("8999:").unwrap(),
        "exclusions must be evaluated before dns capture: {rules}"
    );

    let (ok, out) = ip_in_full(NS_EXCL, &["-4", "route", "get", "198.51.100.7"]);
    assert!(
        ok && out.contains(&format!("dev {MAIN_IFACE}")),
        "excluded address must route via main: {out}"
    );

    // Without the exclusion this lookup would match the dns capture rule and
    // land in the tunnel table.
    let (ok, out) = ip_in_full(
        NS_EXCL,
        &[
            "-4",
            "route",
            "get",
            "192.0.2.53",
            "ipproto",
            "udp",
            "dport",
            "53",
        ],
    );
    assert!(
        ok && out.contains(&format!("dev {MAIN_IFACE}")),
        "excluded resolver must skip dns capture: {out}"
    );
}

#[test]
fn exclusions_replaced_on_reup_and_cleared_on_teardown() {
    let _ = run("ip", &["netns", "del", NS_EXCL_REUP]);
    if !run("ip", &["netns", "add", NS_EXCL_REUP]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }
    let _guard = NsGuard(NS_EXCL_REUP);

    if !ip_in(
        NS_EXCL_REUP,
        &["tuntap", "add", "dev", IFACE, "mode", "tun"],
    ) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
        assert!(
            std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
            "required routing acceptance prerequisite is unavailable"
        );
        return;
    }

    let up_two = [
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--exclude",
        "198.51.100.7/32",
        "--exclude",
        "192.0.2.53/32",
    ];
    let up_one = [
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--exclude",
        "198.51.100.7/32",
    ];
    let up_none = ["xray-up", "--iface", IFACE, "--addr", ADDR];
    let recreate = || {
        assert!(ip_in(
            NS_EXCL_REUP,
            &["tuntap", "add", "dev", IFACE, "mode", "tun"]
        ));
    };

    assert!(netctl_in(NS_EXCL_REUP, &up_two));
    assert_eq!(exclusion_lines(NS_EXCL_REUP).len(), 2);

    assert!(netctl_in(NS_EXCL_REUP, &up_one));
    let excl = exclusion_lines(NS_EXCL_REUP);
    assert!(
        excl.len() == 1 && excl[0].contains("198.51.100.7"),
        "re-up must keep only the listed exclusion: {excl:?}"
    );

    assert!(netctl_in(NS_EXCL_REUP, &up_none));
    let excl = exclusion_lines(NS_EXCL_REUP);
    assert!(
        excl.is_empty(),
        "re-up without --exclude must clear: {excl:?}"
    );

    assert!(netctl_in(NS_EXCL_REUP, &up_two));
    assert!(netctl_in(NS_EXCL_REUP, &["xray-down", "--iface", IFACE]));
    let excl = exclusion_lines(NS_EXCL_REUP);
    assert!(
        excl.is_empty(),
        "exclusions leaked after xray-down: {excl:?}"
    );

    recreate();
    assert!(netctl_in(NS_EXCL_REUP, &up_two));
    assert!(netctl_in(
        NS_EXCL_REUP,
        &["recover", "--xray", "--iface", IFACE]
    ));
    let excl = exclusion_lines(NS_EXCL_REUP);
    assert!(
        excl.is_empty(),
        "exclusions leaked after recover --xray: {excl:?}"
    );

    recreate();
    assert!(netctl_in(NS_EXCL_REUP, &up_two));
    assert!(netctl_in(
        NS_EXCL_REUP,
        &["recover", "--singbox", "--iface", IFACE]
    ));
    let excl = exclusion_lines(NS_EXCL_REUP);
    assert!(
        excl.is_empty(),
        "exclusions leaked after recover --singbox: {excl:?}"
    );
}

fn owned_command(path: &std::path::Path, identity: &str, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .arg("--journal")
        .arg(path)
        .arg("--session-id")
        .arg(identity)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn journaled_ownership_contract() {
    const CHILD: &str = "V2RAY_RS_OWNERSHIP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        if !std::path::Path::new("/dev/net/tun").exists() {
            assert!(
                std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
                "required TUN device is missing"
            );
            return;
        }
        let namespace = format!("nctl-owned-{}", std::process::id());
        if !run("ip", &["netns", "add", &namespace]) {
            assert!(
                std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
                "required network namespace is unavailable"
            );
            return;
        }
        let _guard = NsGuard(&namespace);
        let result = Command::new("timeout")
            .arg("90s")
            .args(["ip", "netns", "exec", &namespace])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "journaled_ownership_contract",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env(
                "V2RAY_RS_OWNERSHIP_PARENT_NS",
                v2ray_rs_tun_session::Epoch::current()
                    .unwrap()
                    .namespace_inode
                    .to_string(),
            )
            .status()
            .unwrap();
        assert!(result.success(), "journaled ownership acceptance failed");
        return;
    }
    let parent: u64 = std::env::var("V2RAY_RS_OWNERSHIP_PARENT_NS")
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(
        parent,
        v2ray_rs_tun_session::Epoch::current()
            .unwrap()
            .namespace_inode
    );
    use v2ray_rs_tun_session::{Ownership, Phase, RESERVATION_NAME, Resource, Session};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.json");
    let mut session = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    let identity = session.ownership.as_ref().unwrap().session_id.to_string();
    v2ray_rs_tun_session::write(&path, &session).unwrap();
    let rules = || {
        Command::new("ip")
            .args(["-json", "rule", "show"])
            .output()
            .unwrap()
            .stdout
    };
    let ok = |args: &[&str]| {
        let result = owned_command(&path, &identity, args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    assert!(run("ip", &["rule", "add", "pref", "9001", "table", "main"]));
    let baseline = rules();
    let original = std::fs::read(&path).unwrap();
    assert!(
        !owned_command(&path, &identity, &["reserve", "--iface", IFACE])
            .status
            .success()
    );
    assert_eq!(baseline, rules());
    assert_eq!(original, std::fs::read(&path).unwrap());
    assert!(run("ip", &["rule", "del", "pref", "9001", "table", "main"]));
    ok(&["reserve", "--iface", IFACE]);

    let other_path = root.path().join("other.json");
    let other = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    let other_id = other.ownership.as_ref().unwrap().session_id.to_string();
    v2ray_rs_tun_session::write(&other_path, &other).unwrap();
    assert!(
        !owned_command(&other_path, &other_id, &["reserve", "--iface", IFACE])
            .status
            .success()
    );
    assert!(run("ip", &["tuntap", "add", "dev", IFACE, "mode", "tun"]));
    let index = std::fs::read_to_string(format!("/sys/class/net/{IFACE}/ifindex")).unwrap();
    let args = [
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--ifindex",
        index.trim(),
        "--strict",
    ];
    ok(&args);
    ok(&args);
    session = v2ray_rs_tun_session::read(&path).unwrap();
    assert_eq!(session.ownership.as_ref().unwrap().phase, Phase::Active);
    assert!(!session.ownership.as_ref().unwrap().resources.is_empty());

    assert!(run(
        "ip",
        &[
            "rule",
            "add",
            "pref",
            "8997",
            "to",
            "192.0.2.0/24",
            "table",
            "main"
        ]
    ));
    session.ownership.as_mut().unwrap().pending = Some(Resource::Rule(Vec::new()));
    v2ray_rs_tun_session::write(&path, &session).unwrap();
    let baseline = rules();
    assert!(
        !owned_command(&path, &identity, &["recover", "--xray", "--iface", IFACE])
            .status
            .success()
    );
    assert_eq!(baseline, rules());
    assert!(
        v2ray_rs_tun_session::read::<String>(&path)
            .unwrap()
            .ownership
            .unwrap()
            .pending
            .is_some()
    );
    assert!(run(
        "ip",
        &[
            "rule",
            "del",
            "pref",
            "8997",
            "to",
            "192.0.2.0/24",
            "table",
            "main"
        ]
    ));
    assert!(run(
        "ip",
        &["link", "set", "dev", IFACE, "alias", "foreign-fixture"]
    ));
    assert!(
        !owned_command(&path, &identity, &["recover", "--xray", "--iface", IFACE])
            .status
            .success()
    );
    let alias = session.ownership.as_ref().unwrap().alias();
    assert!(run("ip", &["link", "set", "dev", IFACE, "alias", &alias]));
    ok(&["recover", "--xray", "--iface", IFACE]);
    ok(&["recover", "--xray", "--iface", IFACE]);
    assert_eq!(
        v2ray_rs_tun_session::read::<String>(&path)
            .unwrap()
            .ownership
            .unwrap()
            .phase,
        Phase::Complete
    );
    assert!(!run("ip", &["link", "show", IFACE]));
    assert!(!run("ip", &["link", "show", RESERVATION_NAME]));

    let replacement = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    v2ray_rs_tun_session::write(&path, &replacement).unwrap();
    assert!(
        !owned_command(&path, &identity, &["reserve", "--iface", IFACE])
            .status
            .success()
    );
    let replacement_id = replacement
        .ownership
        .as_ref()
        .unwrap()
        .session_id
        .to_string();
    assert!(
        owned_command(&path, &replacement_id, &["reserve", "--iface", IFACE])
            .status
            .success()
    );
    assert!(run("ip", &["link", "del", RESERVATION_NAME]));
    assert!(
        !owned_command(
            &path,
            &replacement_id,
            &["recover", "--xray", "--iface", IFACE]
        )
        .status
        .success()
    );
}

#[test]
fn journal_reservation_cli_without_tun() {
    const CHILD: &str = "V2RAY_RS_RESERVATION_CLI_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new("timeout")
            .args(["60s", "unshare", "-rn", "--"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "journal_reservation_cli_without_tun",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env(
                "V2RAY_RS_RESERVATION_PARENT_NS",
                v2ray_rs_tun_session::Epoch::current()
                    .unwrap()
                    .namespace_inode
                    .to_string(),
            )
            .status()
            .unwrap();
        assert!(status.success(), "private namespace CLI acceptance failed");
        return;
    }
    use v2ray_rs_tun_session::{Epoch, Ownership, Phase, RESERVATION_NAME, Session};
    let parent: u64 = std::env::var("V2RAY_RS_RESERVATION_PARENT_NS")
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(parent, Epoch::current().unwrap().namespace_inode);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.json");
    let session = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    let identity = session.ownership.as_ref().unwrap().session_id.to_string();
    v2ray_rs_tun_session::write(&path, &session).unwrap();
    let reserve = ["reserve", "--iface", IFACE];
    let recover = ["recover", "--xray", "--iface", IFACE];
    let success = |identity: &str, args: &[&str]| {
        let result = owned_command(&path, identity, args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    assert!(run("ip", &["rule", "add", "pref", "9001", "table", "main"]));
    let original = std::fs::read(&path).unwrap();
    let failure = owned_command(&path, &identity, &reserve);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("already in use"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(run("ip", &["rule", "del", "pref", "9001", "table", "main"]));
    success(&identity, &reserve);
    success(&identity, &reserve);
    assert_eq!(
        v2ray_rs_tun_session::read::<String>(&path)
            .unwrap()
            .ownership
            .unwrap()
            .phase,
        Phase::Reserved
    );

    let other_path = dir.path().join("other.json");
    let mut other = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    let other_id = other.ownership.as_ref().unwrap().session_id.to_string();
    v2ray_rs_tun_session::write(&other_path, &other).unwrap();
    assert!(
        !owned_command(&other_path, &other_id, &reserve)
            .status
            .success()
    );
    success(&identity, &recover);
    success(&identity, &recover);
    assert!(!run("ip", &["link", "show", RESERVATION_NAME]));

    v2ray_rs_tun_session::write(&path, &other).unwrap();
    success(&other_id, &reserve);
    let current = std::fs::read(&path).unwrap();
    let failure = owned_command(&path, &identity, &recover);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("session identity changed"));
    assert_eq!(std::fs::read(&path).unwrap(), current);
    other = v2ray_rs_tun_session::read(&path).unwrap();
    other.ownership.as_mut().unwrap().epoch.namespace_inode += 1;
    v2ray_rs_tun_session::write(&path, &other).unwrap();
    let invalid = std::fs::read(&path).unwrap();
    assert!(!owned_command(&path, &other_id, &recover).status.success());
    assert_eq!(std::fs::read(&path).unwrap(), invalid);
    other.ownership.as_mut().unwrap().epoch = Epoch::current().unwrap();
    v2ray_rs_tun_session::write(&path, &other).unwrap();
    assert!(run("ip", &["link", "del", RESERVATION_NAME]));
    let failure = owned_command(&path, &other_id, &recover);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("reservation is missing"));
}

/// Journaled `xray-up` reruns converge the optional rules: a changed UID while
/// the earlier rule is still installed replaces rather than stacks, and a rerun
/// without `--capture-dns`/`--bypass-uid` removes the optional rules from the
/// kernel and the journal even when the kernel rule is already gone out-of-band.
#[test]
fn journaled_rerun_without_optional_flags_removes_stale_rules() {
    const CHILD: &str = "V2RAY_RS_RECONCILE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        if !std::path::Path::new("/dev/net/tun").exists() {
            assert!(
                std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
                "required TUN device is missing"
            );
            return;
        }
        let namespace = format!("nctl-reconcile-{}", std::process::id());
        if !run("ip", &["netns", "add", &namespace]) {
            assert!(
                std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_none(),
                "required network namespace is unavailable"
            );
            return;
        }
        let _guard = NsGuard(&namespace);
        let result = Command::new("timeout")
            .arg("90s")
            .args(["ip", "netns", "exec", &namespace])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "journaled_rerun_without_optional_flags_removes_stale_rules",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env(
                "V2RAY_RS_RECONCILE_PARENT_NS",
                v2ray_rs_tun_session::Epoch::current()
                    .unwrap()
                    .namespace_inode
                    .to_string(),
            )
            .status()
            .unwrap();
        assert!(
            result.success(),
            "optional-rule reconcile acceptance failed"
        );
        return;
    }
    let parent: u64 = std::env::var("V2RAY_RS_RECONCILE_PARENT_NS")
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(
        parent,
        v2ray_rs_tun_session::Epoch::current()
            .unwrap()
            .namespace_inode
    );
    use rtnetlink::packet_core::Parseable;
    use rtnetlink::packet_route::rule::{RuleAttribute, RuleMessage, RuleMessageBuffer};
    use v2ray_rs_tun_session::{Ownership, Phase, Resource, Session};

    // The child already runs inside the namespace, so `ip` needs no netns exec.
    let kernel_rules = || -> Vec<String> {
        ["-4", "-6"]
            .iter()
            .flat_map(|family| {
                String::from_utf8_lossy(
                    &Command::new("ip")
                        .args([family, "rule", "show"])
                        .output()
                        .unwrap()
                        .stdout,
                )
                .lines()
                .map(|line| format!("{family} {line}"))
                .collect::<Vec<_>>()
            })
            .collect()
    };
    let journaled_rules = |path: &std::path::Path| -> Vec<(u32, Option<u32>)> {
        v2ray_rs_tun_session::read::<String>(path)
            .unwrap()
            .ownership
            .unwrap()
            .resources
            .iter()
            .filter_map(|resource| {
                let Resource::Rule(bytes) = resource else {
                    return None;
                };
                let rule =
                    RuleMessage::parse(&RuleMessageBuffer::new_checked(bytes).unwrap()).unwrap();
                let priority = rule.attributes.iter().find_map(|a| match a {
                    RuleAttribute::Priority(p) => Some(*p),
                    _ => None,
                });
                let uid = rule.attributes.iter().find_map(|a| match a {
                    RuleAttribute::UidRange(range) if range.start == range.end => Some(range.start),
                    _ => None,
                });
                priority.map(|priority| (priority, uid))
            })
            .collect()
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let session = Session {
        backend: "xray".to_string(),
        iface: IFACE.to_string(),
        ownership: Some(Ownership::new().unwrap()),
    };
    let identity = session.ownership.as_ref().unwrap().session_id.to_string();
    v2ray_rs_tun_session::write(&path, &session).unwrap();
    let owned = |args: &[&str]| {
        let result = owned_command(&path, &identity, args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    assert!(run("ip", &["tuntap", "add", "dev", IFACE, "mode", "tun"]));
    let index = std::fs::read_to_string(format!("/sys/class/net/{IFACE}/ifindex")).unwrap();
    let index = index.trim();

    owned(&["reserve", "--iface", IFACE]);
    owned(&[
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--ifindex",
        index,
        "--capture-dns",
        "--bypass-uid",
        "999990",
    ]);
    let journaled = journaled_rules(&path);
    assert!(
        journaled.contains(&(8998, Some(999990))) && journaled.contains(&(8999, None)),
        "optional rules missing from the journal after the first up: {journaled:?}"
    );
    assert!(
        kernel_rules()
            .iter()
            .any(|line| line.contains("8998:") && line.contains("999990")),
        "999990 bypass-uid rule missing from the kernel after the first up"
    );

    // Changed UID while the earlier rule is still installed: the rerun must
    // replace it, not stack beside it.
    owned(&[
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--ifindex",
        index,
        "--bypass-uid",
        "999991",
    ]);
    let live = kernel_rules();
    assert_eq!(
        live.iter().filter(|line| line.contains("8998:")).count(),
        1,
        "changed bypass UID stacked kernel rules: {live:?}"
    );
    assert!(
        live.iter()
            .any(|line| line.contains("8998:") && line.contains("999991")),
        "999991 bypass-uid rule missing after the changed-UID rerun: {live:?}"
    );
    let journaled = journaled_rules(&path);
    assert_eq!(
        journaled
            .iter()
            .filter(|rule| *rule == &(8998, Some(999991)))
            .count(),
        1,
        "changed bypass UID stacked journal entries: {journaled:?}"
    );

    // The kernel rule is gone while the journal still claims it; the next run
    // must converge the stale record along with the optional rules.
    assert!(run("ip", &["rule", "del", "pref", "8998"]));

    owned(&[
        "xray-up",
        "--iface",
        IFACE,
        "--addr",
        ADDR,
        "--ifindex",
        index,
    ]);
    let live = kernel_rules();
    assert!(
        !live.iter().any(|line| line.contains("8998:")),
        "bypass-uid rule survived re-up without --bypass-uid: {live:?}"
    );
    assert!(
        !live.iter().any(|line| line.contains("8999:")),
        "DNS capture rule survived re-up without --capture-dns: {live:?}"
    );
    assert!(
        live.iter().any(|line| line.contains("9000:")),
        "core policy rules lost during reconcile: {live:?}"
    );
    let journaled = journaled_rules(&path);
    assert!(
        !journaled
            .iter()
            .any(|&(priority, _)| priority == 8998 || priority == 8999),
        "stale optional rules kept in the journal: {journaled:?}"
    );
    assert!(
        journaled.contains(&(9000, None))
            && journaled.contains(&(9001, None))
            && journaled.contains(&(9002, None)),
        "core rules missing from the journal after reconcile: {journaled:?}"
    );

    owned(&["xray-down", "--iface", IFACE, "--ifindex", index]);
    assert_eq!(
        v2ray_rs_tun_session::read::<String>(&path)
            .unwrap()
            .ownership
            .unwrap()
            .phase,
        Phase::Complete
    );
}
