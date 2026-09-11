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
const IFACE: &str = "nctltest0";
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

/// Runs the netctl binary inside the given namespace, so its netlink socket
/// operates on that namespace's network stack rather than the host's.
fn netctl_in(ns: &str, args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", ns, BIN];
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
struct NsGuard(&'static str);
impl Drop for NsGuard {
    fn drop(&mut self) {
        let _ = run("ip", &["netns", "del", self.0]);
    }
}

#[test]
fn up_down_is_idempotent_in_namespace() {
    let _ = run("ip", &["netns", "del", NS]); // best-effort pre-clean from a prior run
    if !run("ip", &["netns", "add", NS]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        return;
    }
    let _guard = NsGuard(NS); // from here, any return or panic deletes the namespace

    // A `tun` device stands in for the one xray creates (the `dummy` driver is
    // not universally installed; the feature requires `tun` regardless).
    if !ip_in_ns(&["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
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
        return;
    }
    let _guard = NsGuard(NS_DNS);

    if !ip_in(NS_DNS, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
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
        return;
    }
    let _guard = NsGuard(NS_STRICT);

    if !ip_in(NS_STRICT, &["tuntap", "add", "dev", IFACE, "mode", "tun"]) {
        eprintln!("skipping: cannot create a tun device (needs /dev/net/tun)");
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

    // No --addr6: the IPv6 table holds only the fallback, never a device route.
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
    assert!(
        !v6_rules.contains("8999:"),
        "IPv6 dns capture rule present without --addr6: {v6_rules}"
    );
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
        for pref in ["8998:", "8999:", "9000:", "9001:", "9002:"] {
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
