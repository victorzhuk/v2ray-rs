//! Privileged idempotency test for the route helper. Requires root and is gated
//! behind the `privileged-tests` feature, so it never runs under `cargo test`/
//! `-short`. Everything runs inside a throwaway network namespace, so the split
//! routes the helper installs never touch the host routing table.
//! Run with: `sudo -E cargo test -p v2ray-rs-netctl --features privileged-tests`.
#![cfg(feature = "privileged-tests")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_v2ray-rs-netctl");
const NS: &str = "nctl-test-ns";
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

/// Runs `ip <args>` inside the test network namespace.
fn ip_in_ns(args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", NS, "ip"];
    full.extend_from_slice(args);
    run("ip", &full)
}

/// Runs `ip <args>` inside the namespace and returns its stdout.
fn ip_in_ns_output(args: &[&str]) -> String {
    let mut full = vec!["netns", "exec", NS, "ip"];
    full.extend_from_slice(args);
    Command::new("ip")
        .args(&full)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Runs the netctl binary inside the test namespace, so its netlink socket
/// operates on the namespace's network stack rather than the host's.
fn netctl(args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", NS, BIN];
    full.extend_from_slice(args);
    run("ip", &full)
}

/// Whether the test device currently exists inside the namespace.
fn device_exists() -> bool {
    ip_in_ns(&["link", "show", IFACE])
}

/// Deletes the namespace (and everything in it) on drop, so a panicking
/// assertion can never leak the namespace or the test device.
struct NsGuard;
impl Drop for NsGuard {
    fn drop(&mut self) {
        let _ = run("ip", &["netns", "del", NS]);
    }
}

#[test]
fn up_down_is_idempotent_in_namespace() {
    let _ = run("ip", &["netns", "del", NS]); // best-effort pre-clean from a prior run
    if !run("ip", &["netns", "add", NS]) {
        eprintln!("skipping: cannot create a network namespace (needs root + netns support)");
        return;
    }
    let _guard = NsGuard; // from here, any return or panic deletes the namespace

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
