//! Privileged idempotency tests. Require root and are gated behind the
//! `privileged-tests` feature, so they never run under `cargo test`/`-short`.
//! Run with: `sudo -E cargo test -p v2ray-rs-netctl --features privileged-tests`.
#![cfg(feature = "privileged-tests")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_v2ray-rs-netctl");
const IFACE: &str = "nctltest0";
const ADDR: &str = "172.31.255.1/30";

fn netctl(args: &[&str]) -> bool {
    Command::new(BIN)
        .args(args)
        .status()
        .expect("spawn netctl")
        .success()
}

fn ip(args: &[&str]) {
    let _ = Command::new("ip").args(args).status();
}

#[test]
fn up_down_is_idempotent_in_namespace() {
    // A dummy device stands in for the TUN device xray would create.
    ip(&["link", "del", IFACE]);
    let created = Command::new("ip")
        .args(["link", "add", IFACE, "type", "dummy"])
        .status()
        .expect("ip link add")
        .success();
    assert!(
        created,
        "needs root / CAP_NET_ADMIN to create the test device"
    );

    // Running up twice must succeed (address EEXIST + route present are ignored).
    assert!(netctl(&["xray-up", "--iface", IFACE, "--addr", ADDR]));
    assert!(netctl(&["xray-up", "--iface", IFACE, "--addr", ADDR]));

    // Down deletes the device; a second down is a clean no-op.
    assert!(netctl(&["xray-down", "--iface", IFACE]));
    assert!(netctl(&["xray-down", "--iface", IFACE]));

    // Recover is a no-op once the device is gone.
    assert!(netctl(&["recover", "--xray", "--iface", IFACE]));

    ip(&["link", "del", IFACE]);
}
