#![cfg(feature = "privileged-tests")]
//! Real sing-box TUN routing acceptance.
//!
//! Each test re-executes this binary inside an unprivileged user+network
//! namespace (`unshare -rn`), builds a synthetic docker0-shaped topology,
//! generates the production sing-box config through
//! `v2ray_rs_core::config::ConfigWriter::write_config` with prefixes discovered
//! by the production `v2ray_rs_process::connected_subnets`, and asserts on real
//! kernel route selection and real packet delivery. A missing prerequisite
//! fails loudly; nothing skips as success.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendLogLevel, BackendType, ProxyNode, RoutingRule, RuleAction, RuleMatch,
    ShadowsocksConfig,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::AppProfile;

const CHILD_ENV: &str = "V2RAY_RS_NETNS_CHILD";
const SING_BOX: &str = "/usr/bin/sing-box";
const TUN_IFACE: &str = "tun0";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CHILD_TIMEOUT: Duration = Duration::from_secs(180);

fn is_child() -> bool {
    std::env::var(CHILD_ENV).as_deref() == Ok("1")
}

/// Parent side: re-execute this binary inside `unshare -rn` for one test.
fn run_namespaced(test: &'static str, expected_evidence: &[&str]) {
    assert!(
        !is_child(),
        "namespace child must never recurse into run_namespaced"
    );
    let exe = std::env::current_exe().expect("current test binary");
    let mut child = Command::new("unshare")
        .arg("-rn")
        .arg("--")
        .arg(&exe)
        .arg("--exact")
        .arg(test)
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn unshare -rn; unshare is required for this acceptance test");

    let deadline = Instant::now() + CHILD_TIMEOUT;
    let output = loop {
        match child.try_wait().expect("wait on namespace child") {
            Some(_status) => break child.wait_with_output().expect("collect namespace child"),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("namespace child for {test} exceeded {:?}", CHILD_TIMEOUT);
            }
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "namespace child for {test} failed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    for marker in expected_evidence {
        assert!(
            stdout.contains(&format!("EVIDENCE {marker}")),
            "missing evidence {marker:?} for {test}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        );
    }
    println!("== {test}: namespace child output ==\n{stdout}");
}

// ---------------------------------------------------------------------------
// Child-side helpers
// ---------------------------------------------------------------------------

fn prereq(msg: &str) -> ! {
    panic!("missing prerequisite: {msg}");
}

/// Fails loudly naming the missing prerequisite.
fn check_prerequisites() {
    if !Path::new("/dev/net/tun").exists() {
        prereq("/dev/net/tun is missing; the kernel TUN device is required");
    }
    for tool in [
        "/usr/bin/ip",
        "/usr/bin/unshare",
        "/usr/bin/nsenter",
        "/usr/bin/python3",
    ] {
        let ok = Path::new(tool).exists();
        if !ok {
            prereq(&format!("required tool {tool:?} is unavailable"));
        }
    }
    if !Path::new(SING_BOX).exists() {
        prereq(&format!("{SING_BOX} is missing"));
    }
}

/// Runs `ip` and panics with stderr on failure.
fn ip(args: &[&str]) {
    let out = Command::new("ip")
        .args(args)
        .output()
        .unwrap_or_else(|e| prereq(&format!("ip {:?} could not run: {e}", args)));
    assert!(
        out.status.success(),
        "ip {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// True when the interface exists in the current (namespace) network stack.
fn iface_exists(iface: &str) -> bool {
    Command::new("/usr/bin/ip")
        .args(["link", "show", iface])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn route_dev(dst: &str) -> String {
    let out = Command::new("ip")
        .args(["route", "get", dst])
        .output()
        .expect("ip route get");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "ip route get {dst} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let words: Vec<&str> = text.split_whitespace().collect();
    let dev = words
        .iter()
        .position(|w| *w == "dev")
        .and_then(|i| words.get(i + 1))
        .copied()
        .unwrap_or_else(|| prereq(&format!("no device in `ip route get {dst}`: {text}")));
    dev.to_string()
}

fn evidence(key: &str, value: &str) {
    println!("EVIDENCE {key}={value}");
}

fn assert_route(dst: &str, want_dev: &str) {
    let dev = route_dev(dst);
    assert_eq!(
        dev, want_dev,
        "ip route get {dst} selected {dev}, expected {want_dev}"
    );
    evidence(&format!("route_{dst}"), &dev);
}

/// TCP echo probe with a hard timeout.
fn tcp_echo(addr: &str, port: u16, payload: &str, marker: &str) -> Result<(), String> {
    let run = || -> Result<(), String> {
        let addrs: Vec<SocketAddr> = (addr, port)
            .to_socket_addrs()
            .map_err(|e| format!("resolve: {e}"))?
            .collect();
        let peer = *addrs.first().ok_or("no resolved address")?;
        let mut stream = TcpStream::connect_timeout(&peer, REQUEST_TIMEOUT)
            .map_err(|e| format!("TCP connect failed within timeout: {e}"))?;
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .and_then(|_| stream.set_write_timeout(Some(REQUEST_TIMEOUT)))
            .map_err(|e| format!("set socket timeouts: {e}"))?;
        stream
            .write_all(payload.as_bytes())
            .map_err(|e| format!("TCP send failed: {e}"))?;
        let mut buf = [0u8; 256];
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("TCP read failed: {e}"))?;
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        if !text.starts_with("reply:") {
            return Err(format!("unexpected reply: {text:?}"));
        }
        Ok(())
    };
    run()
        .map(|()| evidence(marker, "ok"))
        .map_err(|e| format!("{addr}:{port} {e}"))
}

/// UDP DNS-shaped probe: the local fixture echoes any datagram it receives.
fn udp_dns_fixture(server: &str, marker: &str) {
    let socket = UdpSocket::bind("0.0.0.0:0").expect("bind UDP client");
    socket
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .expect("set read timeout");
    let peer: SocketAddr = format!("{server}:53")
        .parse()
        .expect("parse resolver address");
    socket
        .send_to(b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01", peer)
        .unwrap_or_else(|e| panic!("UDP send to {peer} failed: {e}"));
    let mut buf = [0u8; 512];
    let (n, from) = socket
        .recv_from(&mut buf)
        .unwrap_or_else(|e| panic!("UDP reply from {server}:53 failed within timeout: {e}"));
    assert_eq!(
        from, peer,
        "reply for {server}:53 arrived from unexpected source {from}"
    );
    assert!(
        buf[..n].starts_with(b"reply:"),
        "unexpected UDP payload from {from}: {:?}",
        String::from_utf8_lossy(&buf[..n])
    );
    evidence(marker, "ok");
}

const FIXTURE_PY: &str = r#"
import os, socket, subprocess, sys, threading, time

def ip(*a):
    r = subprocess.run(["ip", *a], capture_output=True, text=True)
    if r.returncode != 0:
        print("FIXTURE-ERR", a, r.stderr, flush=True)
        sys.exit(1)

ip("link", "set", "lo", "up")
ip("link", "add", "docker0", "type", "veth", "peer", "name", "dpeer")
ip("link", "add", "uplink0", "type", "veth", "peer", "name", "upeer")
ip("link", "set", "docker0", "netns", str(os.getppid()))
ip("link", "set", "uplink0", "netns", str(os.getppid()))
ip("addr", "add", "172.17.0.2/16", "dev", "dpeer")
ip("addr", "add", "192.168.100.2/24", "dev", "upeer")
ip("addr", "add", "10.0.0.5/8", "dev", "upeer")
ip("link", "set", "dpeer", "up")
ip("link", "set", "upeer", "up")

def tcp_serve(host, port):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((host, port))
    s.listen(8)
    while True:
        c, _ = s.accept()
        def handle(c):
            c.settimeout(5)
            try:
                d = c.recv(4096)
                if d:
                    c.sendall(b"reply:" + d)
            except Exception:
                pass
            c.close()
        threading.Thread(target=handle, args=(c,), daemon=True).start()

def udp53():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("172.17.0.2", 53))
    while True:
        d, a = s.recvfrom(4096)
        s.sendto(b"reply:" + d, a)

threading.Thread(target=tcp_serve, args=("172.17.0.2", 9001), daemon=True).start()
threading.Thread(target=tcp_serve, args=("10.0.0.5", 9002), daemon=True).start()
threading.Thread(target=udp53, daemon=True).start()
print("FIXTURE-READY", flush=True)
time.sleep(300)
"#;

struct Fixture {
    child: Child,
}

impl Fixture {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("v2ray-rs-sbroute-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        let script = dir.join("fixture.py");
        std::fs::write(&script, FIXTURE_PY).expect("write fixture script");

        let mut child = Command::new("unshare")
            .arg("-n")
            .arg("python3")
            .arg("-u")
            .arg(&script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| prereq(&format!("could not spawn fixture namespace: {e}")));

        let mut stdout = child.stdout.take().expect("fixture stdout");
        let mut line = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                prereq("fixture namespace never became ready within 10s");
            }
            let mut byte = [0u8; 1];
            match stdout.read(&mut byte) {
                Ok(0) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    prereq("fixture namespace exited before readiness");
                }
                Ok(_) => {
                    line.push(byte[0] as char);
                    if line.contains("FIXTURE-READY") {
                        break;
                    }
                    if line.contains("FIXTURE-ERR") {
                        let _ = child.kill();
                        let _ = child.wait();
                        prereq(&format!("fixture setup failed: {line}"));
                    }
                }
            }
        }
        Self { child }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Parent-side topology, built after the fixture moved its veth ends over.
fn build_topology(extra_dummies: &[(&str, &str)]) {
    ip(&["link", "set", "lo", "up"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    for iface in ["docker0", "uplink0"] {
        loop {
            if iface_exists(iface) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture never delivered interface {iface}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    ip(&["addr", "add", "172.17.0.1/16", "dev", "docker0"]);
    ip(&["link", "set", "docker0", "up"]);
    ip(&["addr", "add", "192.168.100.1/24", "dev", "uplink0"]);
    ip(&["link", "set", "uplink0", "up"]);
    // Default uplink via the fixture peer (a real gateway so sing-box's
    // auto_detect_interface can bind its outbound dials) and a static
    // (non-connected) route to 10/8: discovery collects interface prefixes
    // only, so 10.0.0.5 must stay TUN-captured.
    ip(&[
        "route",
        "add",
        "default",
        "via",
        "192.168.100.2",
        "dev",
        "uplink0",
    ]);
    ip(&["route", "add", "10.0.0.0/8", "dev", "uplink0"]);
    // A second bridge and a LAN prefix, plain dummies for discovery.
    ip(&["link", "add", "brlan0", "type", "dummy"]);
    ip(&["addr", "add", "192.168.50.1/24", "dev", "brlan0"]);
    ip(&["link", "set", "brlan0", "up"]);
    for (name, cidr) in extra_dummies {
        ip(&["link", "add", name, "type", "dummy"]);
        ip(&["addr", "add", cidr, "dev", name]);
        ip(&["link", "set", name, "up"]);
    }
}

fn ss_node() -> ProxyNode {
    ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: "ss.example.com".into(),
        port: 8388,
        method: "aes-256-gcm".into(),
        password: "secret".into(),
        remark: Some("Test SS".into()),
    })
}

struct Harness {
    _fixture: Fixture,
    config_path: PathBuf,
    _paths_dir: tempfile::TempDir,
    singbox: Child,
    singbox_log: PathBuf,
}

/// Routes 10/8 (the non-connected TUN-captured destination) to the direct
/// outbound: the synthetic proxy node is unreachable by design.
fn direct_rules() -> Vec<RoutingRule> {
    vec![RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::IpCidr {
            cidr: "10.0.0.0/8".parse().expect("parse 10.0.0.0/8"),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    }]
}

impl Harness {
    /// Builds topology + fixture, generates the real config with `automatic`
    /// discovery (or an empty snapshot for the positive control), starts
    /// sing-box and waits for the TUN device.
    fn start(automatic: bool, extra_dummies: &[(&str, &str)]) -> Self {
        check_prerequisites();
        let fixture = Fixture::start();
        build_topology(extra_dummies);

        let paths_dir = tempfile::tempdir().expect("tempdir for AppPaths");
        let paths = AppPaths::for_profile_in(AppProfile::Test, paths_dir.path());
        let mut settings = AppSettings::default();
        settings.backend.backend_type = BackendType::SingBox;
        settings.tun.enabled = true;
        settings.tun.interface_name = TUN_IFACE.to_string();
        // This kernel provides no IPv6 inside the namespace: sing-tun's
        // strict_route adds IPv6 ip-rules that fail with "address family not
        // supported by protocol". The defect under test is automatic
        // route exclusion, not strict_route, so the fixture disables it.
        settings.tun.strict_route = false;
        settings.logging.backend_level = BackendLogLevel::Debug;
        settings.logging.connection_log = true;

        let connected = if automatic {
            let snap = v2ray_rs_process::connected_subnets(TUN_IFACE)
                .unwrap_or_else(|e| prereq(&format!("connected_subnets failed: {e}")));
            assert!(
                snap.iter().any(|n| n.to_string() == "172.17.0.0/16"),
                "discovery snapshot missing docker0 prefix 172.17.0.0/16: {snap:?}"
            );
            snap
        } else {
            Vec::new()
        };

        let writer = ConfigWriter::new(&settings, &paths);
        let config_path = writer
            .write_config(&[ss_node()], &direct_rules(), &settings, &connected)
            .unwrap_or_else(|e| prereq(&format!("write_config failed: {e}")));

        let singbox_log = paths_dir.path().join("singbox.log");
        let log_file = std::fs::File::create(&singbox_log).expect("create singbox log");
        let err_file = log_file.try_clone().expect("clone singbox log file");
        let mut singbox = Command::new(SING_BOX)
            .args(["run", "-c", &config_path.to_string_lossy()])
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap_or_else(|e| prereq(&format!("could not start {SING_BOX}: {e}")));

        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if iface_exists(TUN_IFACE) {
                std::thread::sleep(Duration::from_millis(500)); // let rules settle
                break;
            }
            if let Some(status) = singbox.try_wait().expect("poll sing-box") {
                let log = std::fs::read_to_string(&singbox_log).unwrap_or_default();
                panic!(
                    "sing-box exited early ({status}) without creating {TUN_IFACE} within \
                     {STARTUP_TIMEOUT:?}; log:\n{log}"
                );
            }
            assert!(
                Instant::now() < deadline,
                "sing-box did not create {TUN_IFACE} within {STARTUP_TIMEOUT:?}; log:\n{}",
                std::fs::read_to_string(&singbox_log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(200));
        }

        Self {
            _fixture: fixture,
            config_path,
            _paths_dir: paths_dir,
            singbox,
            singbox_log,
        }
    }

    fn log_tail(&self) -> String {
        std::fs::read_to_string(&self.singbox_log).unwrap_or_else(|e| format!("<{e}>"))
    }

    fn tun_inbound_route_exclude_address(&self) -> Vec<String> {
        let json = std::fs::read_to_string(&self.config_path).expect("read generated config");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse generated config");
        let tun = value["inbounds"]
            .as_array()
            .expect("inbounds array")
            .iter()
            .find(|i| i["type"] == "tun")
            .unwrap_or_else(|| prereq("generated config has no tun inbound"))
            .clone();
        tun["route_exclude_address"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.singbox.kill();
        let _ = self.singbox.wait();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. Attached bridge subnet resolves via the bridge and carries real traffic.
#[test]
fn attached_bridge_is_not_captured() {
    if is_child() {
        let h = Harness::start(true, &[]);
        assert_route("172.17.0.2", "docker0");
        if let Err(e) = tcp_echo("172.17.0.2", 9001, "bridge-ping", "tcp_bridge") {
            panic!("bridge echo failed: {e}; sing-box log:\n{}", h.log_tail());
        }
        return;
    }
    run_namespaced(
        "attached_bridge_is_not_captured",
        &["route_172.17.0.2=docker0", "tcp_bridge=ok"],
    );
}

/// 2. Positive control: without the automatic entries the bridge destination
///    is captured by the TUN (the original defect).
#[test]
fn positive_control_without_automatic_exclusions() {
    if is_child() {
        let _h = Harness::start(false, &[]);
        let dev = route_dev("172.17.0.2");
        let rules = Command::new("/usr/bin/ip")
            .args(["rule", "list"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if dev != TUN_IFACE {
            panic!(
                "positive control could not reproduce the original defect: without automatic \
                 exclusions, `ip route get 172.17.0.2` selected dev {dev}, not the TUN. \
                 Missing prerequisite: sing-box 1.14.3's auto_route adds rule \
                 `lookup 2022 suppress_prefixlength 0`, which already exempts every main-table \
                 connected prefix, so the capture defect only manifests with strict_route=true \
                 (requires kernel IPv6; this namespace has none, sing-tun aborts on \
                 `add rule: address family not supported`) or with sing-box <= 1.13.14 \
                 (not installed; only 1.14.3 present). Rules in effect:\n{rules}"
            );
        }
        evidence("route_172.17.0.2", &dev);
        return;
    }
    run_namespaced(
        "positive_control_without_automatic_exclusions",
        &["route_172.17.0.2=tun0"],
    );
}

/// 3. A non-connected private destination stays captured by the TUN, and a
///    packet delivered through the tunnel reaches a controlled endpoint.
#[test]
fn non_connected_private_destination_is_captured() {
    if is_child() {
        let h = Harness::start(true, &[]);
        assert_route("10.0.0.5", TUN_IFACE);
        if let Err(e) = tcp_echo("10.0.0.5", 9002, "tun-ping", "tcp_10_0_0_5") {
            panic!(
                "tunnel delivery failed: {e}\nip rule list:\n{}\nip route show table all:\n{}\n\
                 sing-box log:\n{}",
                Command::new("/usr/bin/ip")
                    .args(["rule", "list"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default(),
                Command::new("/usr/bin/ip")
                    .args(["route", "show", "table", "all"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default(),
                h.log_tail()
            );
        }
        return;
    }
    run_namespaced(
        "non_connected_private_destination_is_captured",
        &["route_10.0.0.5=tun0", "tcp_10_0_0_5=ok"],
    );
}

/// 4. A broader attached prefix containing the TUN subnet is retained in the
///    exclusions while the TUN's own /30 still resolves to the TUN.
#[test]
fn tun_subnet_inside_attached_bridge_stays_on_tun() {
    if is_child() {
        let h = Harness::start(true, &[("brwide0", "172.19.0.5/16")]);
        let exclusions = h.tun_inbound_route_exclude_address();
        assert!(
            exclusions.iter().any(|e| e == "172.19.0.0/16"),
            "generated exclusions missing 172.19.0.0/16: {exclusions:?}"
        );
        evidence("exclude_contains_172.19.0.0/16", "yes");
        assert_route("172.19.0.2", TUN_IFACE); // inside the TUN's own /30
        assert_route("172.19.0.100", "brwide0"); // elsewhere in the /16
        return;
    }
    run_namespaced(
        "tun_subnet_inside_attached_bridge_stays_on_tun",
        &[
            "exclude_contains_172.19.0.0/16=yes",
            "route_172.19.0.2=tun0",
            "route_172.19.0.100=brwide0",
        ],
    );
}

/// 5. DNS/port-53 to a directly attached resolver bypasses the TUN and is
///    served by the local fixture.
#[test]
fn directly_attached_dns_bypasses_tun() {
    if is_child() {
        let _h = Harness::start(true, &[]);
        assert_route("172.17.0.2", "docker0");
        udp_dns_fixture("172.17.0.2", "udp53_bridge");
        return;
    }
    run_namespaced(
        "directly_attached_dns_bypasses_tun",
        &["route_172.17.0.2=docker0", "udp53_bridge=ok"],
    );
}
