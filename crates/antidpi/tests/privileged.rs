//! Privileged end-to-end tests for the anti-DPI engine: a real TLS
//! handshake crosses two throwaway network namespaces through a censor
//! emulator (NFQUEUE in namespace B) while the engine runs in namespace A.
//! Everything is torn down on drop, so the host network stack is untouched.
//! Requires root, `ip`, `iptables`, `openssl`, `nft`, and `ethtool`.
//!
//! Run with:
//! `sudo -E cargo test --features privileged-tests -p v2ray-rs-antidpi`
//!
//! A missing prerequisite skips a test. With `V2RAY_RS_REQUIRE_NETNS=1` a
//! missing prerequisite fails the test and names the reason, so a required
//! acceptance run can never report success from a skip.
#![cfg(feature = "privileged-tests")]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_v2ray-rs-antidpi");
const CENSOR: &str = env!("CARGO_BIN_EXE_v2ray-rs-antidpi-censor");
const PROBE: &str = env!("CARGO_BIN_EXE_v2ray-rs-antidpi-probe");

const DOMAIN: &str = "blocked.example.com";
const FAKE_SNI: &str = "fonts.google.com";
const IP_B: &str = "10.205.0.1";

fn ns_exec(ns: &str, cmd: &str, args: &[&str]) -> bool {
    let mut full = vec!["netns", "exec", ns, cmd];
    full.extend_from_slice(args);
    Command::new("ip")
        .args(&full)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ns_output(ns: &str, cmd: &str, args: &[&str]) -> (i32, String) {
    let mut full = vec!["netns", "exec", ns, cmd];
    full.extend_from_slice(args);
    match Command::new("ip").args(&full).output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code().unwrap_or(-1), text)
        }
        Err(e) => (-1, e.to_string()),
    }
}

/// First missing tool in the list, by name.
fn missing_tool_reason(tools: &[&str]) -> Option<String> {
    for tool in tools {
        let ok = Command::new("which")
            .arg(tool)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Some(format!("missing tool: {tool}"));
        }
    }
    None
}

/// Environment gate: root plus the tools the lab needs.
fn unsupported_reason() -> Option<String> {
    if unsafe { libc::geteuid() } != 0 {
        return Some("not root".into());
    }
    missing_tool_reason(&["ip", "iptables", "openssl"]).or_else(netns_unsupported_reason)
}

/// Probes that a private namespace can actually be created: root alone is
/// not enough on kernels or containers without usable network namespaces.
fn netns_unsupported_reason() -> Option<String> {
    let probe = format!(
        "adpi-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let ok = Command::new("ip")
        .args(["netns", "add", &probe])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        let _ = Command::new("ip").args(["netns", "del", &probe]).status();
        None
    } else {
        Some("cannot create a network namespace".into())
    }
}

/// Shared skip/fail gate. Returns true when the test may proceed. Without
/// `V2RAY_RS_REQUIRE_NETNS` a missing prerequisite skips the test; with it
/// set the test panics and names the prerequisite, because a required
/// acceptance run must never report success from a skip.
fn prerequisites_met(reason: Option<String>) -> bool {
    match reason {
        None => true,
        Some(reason) => {
            if std::env::var_os("V2RAY_RS_REQUIRE_NETNS").is_some() {
                panic!("required antidpi acceptance prerequisite unavailable: {reason}");
            }
            eprintln!("skipping: {reason}");
            false
        }
    }
}

/// One lab: two namespaces joined by a veth pair, TLS server + censor in B,
/// everything killed and removed on drop. Veth and namespace names carry the
/// tag so concurrent tests never collide.
struct Lab {
    ns_a: String,
    ns_b: String,
    veth_a: String,
    children: Vec<Child>,
    cert: std::path::PathBuf,
    key: std::path::PathBuf,
}

impl Lab {
    fn new(tag: &str, dir: &std::path::Path) -> Lab {
        let ns_a = format!("adpi-{tag}-a");
        let ns_b = format!("adpi-{tag}-b");
        let veth_a = format!("va-{tag}");
        let veth_b = format!("vb-{tag}");
        Command::new("ip")
            .args(["netns", "add", &ns_a])
            .status()
            .unwrap();
        Command::new("ip")
            .args(["netns", "add", &ns_b])
            .status()
            .unwrap();
        Command::new("ip")
            .args([
                "link", "add", &veth_a, "type", "veth", "peer", "name", &veth_b,
            ])
            .status()
            .unwrap();
        Command::new("ip")
            .args(["link", "set", &veth_a, "netns", &ns_a])
            .status()
            .unwrap();
        Command::new("ip")
            .args(["link", "set", &veth_b, "netns", &ns_b])
            .status()
            .unwrap();
        ns_exec(
            &ns_a,
            "ip",
            &["addr", "add", "10.205.0.2/24", "dev", &veth_a],
        );
        ns_exec(
            &ns_b,
            "ip",
            &["addr", "add", "10.205.0.1/24", "dev", &veth_b],
        );
        ns_exec(&ns_a, "ip", &["link", "set", &veth_a, "up"]);
        ns_exec(&ns_b, "ip", &["link", "set", &veth_b, "up"]);
        ns_exec(&ns_a, "ip", &["link", "set", "lo", "up"]);
        ns_exec(&ns_b, "ip", &["link", "set", "lo", "up"]);
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        let cert_s = cert.to_str().unwrap();
        let key_s = key.to_str().unwrap();
        let ok = Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-keyout", key_s, "-out", cert_s, "-days",
                "2", "-nodes", "-subj", "/CN=test",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "openssl req failed");
        Lab {
            ns_a,
            ns_b,
            veth_a,
            children: Vec::new(),
            cert,
            key,
        }
    }

    fn track(&mut self, child: Child) {
        self.children.push(child);
    }

    fn spawn_in(&self, ns: &str, cmd: &str, args: &[&str], log_name: &str) -> Child {
        let mut full = vec!["netns", "exec", ns, cmd];
        full.extend_from_slice(args);
        let log = std::fs::File::create(self.log_path(log_name)).expect("create log file");
        Command::new("ip")
            .args(&full)
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn in netns")
    }

    fn log_path(&self, name: &str) -> std::path::PathBuf {
        let cert_dir = self.cert.parent().expect("cert dir");
        cert_dir.join(format!("{name}.log"))
    }

    /// Prints a child's stderr log (best-effort, for failure diagnosis).
    fn dump_log(&self, name: &str) {
        if let Ok(text) = std::fs::read_to_string(self.log_path(name)) {
            eprintln!("--- {name} log ---\n{text}");
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
        }
        for c in &mut self.children {
            let _ = c.wait();
        }
        let _ = Command::new("ip")
            .args(["netns", "del", &self.ns_a])
            .status();
        let _ = Command::new("ip")
            .args(["netns", "del", &self.ns_b])
            .status();
        let _ = Command::new("ip")
            .args(["link", "del", &self.veth_a])
            .status();
    }
}

/// Starts the TLS server (openssl s_server) inside namespace B.
fn start_server(lab: &mut Lab) {
    let cert = lab.cert.to_str().unwrap();
    let key = lab.key.to_str().unwrap();
    let child = lab.spawn_in(
        &lab.ns_b,
        "openssl",
        &[
            "s_server", "-accept", "443", "-cert", cert, "-key", key, "-quiet",
        ],
        "server",
    );
    lab.track(child);
}

/// Echo server for the bidirectional traffic test: it returns each
/// received line reversed, one echoed line per sent line.
fn start_echo_server(lab: &mut Lab) {
    let cert = lab.cert.to_str().unwrap();
    let key = lab.key.to_str().unwrap();
    let child = lab.spawn_in(
        &lab.ns_b,
        "openssl",
        &[
            "s_server", "-accept", "443", "-cert", cert, "-key", key, "-rev",
        ],
        "server",
    );
    lab.track(child);
}

/// Starts the censor emulator in B with an INPUT NFQUEUE rule.
fn start_censor(lab: &mut Lab, mode: &str) {
    let ok = ns_exec(
        &lab.ns_b,
        "iptables",
        &[
            "-t",
            "mangle",
            "-A",
            "INPUT",
            "-p",
            "tcp",
            "--dport",
            "443",
            "-j",
            "NFQUEUE",
            "--queue-num",
            "201",
            "--queue-bypass",
        ],
    );
    assert!(ok, "censor queue rule install failed");
    let child = lab.spawn_in(
        &lab.ns_b,
        CENSOR,
        &["--mode", mode, "--block", DOMAIN, "--queue", "201"],
        "censor",
    );
    lab.track(child);
}

/// Installs the engine's OUTPUT queue rules in A: the fwmark guard first
/// (rawsend segments carry it and must never loop back into the queue), then
/// the queue rule. The engine runs with --no-fw, so nothing else is touched.
fn start_engine(lab: &mut Lab, strategies: &[&str]) {
    ns_exec(
        &lab.ns_a,
        "iptables",
        &[
            "-t",
            "mangle",
            "-A",
            "OUTPUT",
            "-m",
            "mark",
            "--mark",
            "0x40000000/0x40000000",
            "-j",
            "RETURN",
        ],
    );
    ns_exec(
        &lab.ns_a,
        "iptables",
        &[
            "-t",
            "mangle",
            "-A",
            "OUTPUT",
            "-p",
            "tcp",
            "--dport",
            "443",
            "-j",
            "NFQUEUE",
            "--queue-num",
            "200",
            "--queue-bypass",
        ],
    );
    let mut argv: Vec<&str> = vec!["run", "--queue", "200", "--no-fw"];
    for s in strategies {
        argv.push("--strategy");
        argv.push(s);
    }
    let child = Command::new("ip")
        .args(["netns", "exec", &lab.ns_a, BIN])
        .args(&argv)
        .env("RUST_LOG", "debug")
        .stdout(Stdio::null())
        .stderr({
            let log = std::fs::File::create(lab.log_path("engine")).expect("engine log");
            Stdio::from(log)
        })
        .spawn()
        .expect("spawn engine");
    lab.track(child);
    std::thread::sleep(Duration::from_millis(400));
}

/// One TLS handshake attempt from A to B with the test SNI. `-groups X25519`
/// keeps the ClientHello small enough to fit a single TCP segment, so the
/// per-packet/latch censor models see a complete record at baseline.
fn handshake(lab: &Lab) -> String {
    let (_, out) = ns_output(
        &lab.ns_a,
        "timeout",
        &[
            "8",
            "openssl",
            "s_client",
            "-connect",
            &format!("{IP_B}:443"),
            "-servername",
            DOMAIN,
            "-groups",
            "X25519",
            "-brief",
        ],
    );
    out
}

fn handshake_ok(out: &str) -> bool {
    out.contains("Connection established") || out.contains("Protocol version")
}

/// Retries the handshake until it succeeds or the deadline passes.
fn handshake_until(lab: &Lab, deadline: Duration) -> String {
    let start = Instant::now();
    loop {
        let last = handshake(lab);
        if handshake_ok(&last) {
            return last;
        }
        if start.elapsed() > deadline {
            return last;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Runs one A/B pair in separate labs: the baseline lab proves the censor
/// blocks, then a fresh lab with the engine started before any handshake
/// proves the strategy defeats it. Separate labs keep the baseline's
/// revived-but-dead client (the censor drops its ClientHello forever, so the
/// single-connection TLS server stays stuck in SSL_accept on it) from
/// poisoning the engine phase.
fn ab_test(tag: &str, censor_mode: &str, strategies: &[&str]) {
    if !prerequisites_met(unsupported_reason()) {
        return;
    }
    // A: baseline — the censor without the engine must block the handshake.
    {
        let dir = tempfile::TempDir::new().unwrap();
        let mut lab = Lab::new(&format!("{tag}-base"), dir.path());
        start_server(&mut lab);
        start_censor(&mut lab, censor_mode);
        std::thread::sleep(Duration::from_millis(400));
        let out = handshake(&lab);
        assert!(!handshake_ok(&out), "baseline must fail: {out}");
    }
    // B: engine on — fresh server, censor, and engine; the handshake must
    // now succeed.
    {
        let dir = tempfile::TempDir::new().unwrap();
        let mut lab = Lab::new(&format!("{tag}-engine"), dir.path());
        start_server(&mut lab);
        start_censor(&mut lab, censor_mode);
        start_engine(&mut lab, strategies);
        let out = handshake_until(&lab, Duration::from_secs(10));
        lab.dump_log("censor");
        lab.dump_log("engine");
        assert!(handshake_ok(&out), "handshake failed: {out}");
    }
}

#[test]
fn multisplit_defeats_per_packet_censor() {
    ab_test("ms", "per-packet", &["multisplit:pos=midsld"]);
}

#[test]
fn disorder_defeats_per_packet_censor() {
    ab_test("md", "per-packet", &["multidisorder:pos=midsld"]);
}

#[test]
fn fake_defeats_latch_censor() {
    // Decoy ClientHello with an allowed SNI, seq shifted +10000 so the
    // server treats it as out-of-window garbage; the censor latches on it.
    ab_test(
        "la",
        "latch",
        &[&format!(
            "fake:blob=tls:tls_mod=sni={FAKE_SNI}:tcp_seq=10000"
        )],
    );
}

#[test]
fn seqovl_defeats_reassemble_censor() {
    // seqovl prepends pattern bytes below the server window edge: the
    // censor's naive reassembly desyncs, the server trims the overlap.
    ab_test("sq", "reassemble", &["multisplit:pos=midsld:seqovl=8"]);
}

/// Single throwaway namespace for the nftables lifecycle check; deleted on
/// drop so the host stack is never touched.
struct NsGuard {
    name: String,
}

impl NsGuard {
    fn new(tag: &str) -> NsGuard {
        let name = format!("adpi-{tag}");
        let ok = Command::new("ip")
            .args(["netns", "add", &name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "ip netns add failed");
        NsGuard { name }
    }
}

impl Drop for NsGuard {
    fn drop(&mut self) {
        let _ = Command::new("ip")
            .args(["netns", "del", &self.name])
            .status();
    }
}

/// Gate for the nftables lifecycle: root plus `ip` and `nft` only.
fn nft_unsupported_reason() -> Option<String> {
    if unsafe { libc::geteuid() } != 0 {
        return Some("not root".into());
    }
    missing_tool_reason(&["ip", "nft"]).or_else(netns_unsupported_reason)
}

/// Gate for the traffic test: root, the lab and nftables tools, and a
/// usable netns. `ethtool` disables the veth offloads to keep skbs small;
/// the bulk client below also writes one sub-MSS line at a time, because
/// an unpaced bulk upload still forms outbound GSO before egress on this
/// kernel.
fn traffic_unsupported_reason() -> Option<String> {
    if unsafe { libc::geteuid() } != 0 {
        return Some("not root".into());
    }
    missing_tool_reason(&["ip", "nft", "openssl", "ethtool"]).or_else(netns_unsupported_reason)
}

/// Bounded wait for a child to exit; returns its status once observed, or
/// None when the limit passes (the caller escalates).
fn wait_exit(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("child status") {
            Some(status) => return Some(status),
            None if start.elapsed() > limit => return None,
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Kills and reaps the engine on every exit path after spawn, so a failing
/// assertion can never leak a live consumer into the namespace teardown.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.0.try_wait().expect("engine status").is_none() {
            let _ = unsafe { libc::kill(self.0.id() as libc::pid_t, libc::SIGKILL) };
        }
        let _ = self.0.wait();
    }
}

/// Full helper lifecycle inside a private namespace, under one stable
/// controller identity: `ip netns exec` replaces itself with the command,
/// so `install-rules`, the managed run, and `remove-rules` are all parented
/// by this test process — the same contract the GUI controller has. The
/// test requires:
/// - the journal to record an `active` install covering BOTH families
///   after `install-rules`, with the `ip` and the `ip6` table readable
///   under the implementation's own name (per-family readbacks that exit
///   zero);
/// - `run --session-id` to reach the ready sentinel (printed only after a
///   successful queue bind) and stay alive while both tables remain
///   readable, so readiness and liveness are asserted positively;
/// - `remove-rules` to leave neither family's table — proven by a
///   successful `nft list tables` readback, not by one failed listing —
///   and to remove the journal file specifically (`NotFound`).
///   Skipped without root, `ip`, `nft`, or a usable netns; fails under
///   `V2RAY_RS_REQUIRE_NETNS=1`.
#[test]
fn nft_table_install_remove_roundtrip_in_netns() {
    if !prerequisites_met(nft_unsupported_reason()) {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    private_dir(dir.path());
    let journal = dir.path().join("antidpi.v1.json");
    let ns = NsGuard::new("nft-rt");
    let table = v2ray_rs_antidpi::fw::table_name();
    let journal_s = journal.to_str().unwrap();
    let helper = |args: &[&str]| {
        Command::new("ip")
            .args(["netns", "exec", &ns.name, BIN])
            .args(args)
            .env("V2RAYRS_ANTIDPI_STATE", journal_s)
            .output()
            .expect("run helper inside netns")
    };

    let out = helper(&[
        "install-rules",
        "--queue",
        "201",
        "--strategy",
        "multisplit:pos=midsld",
    ]);
    assert!(
        out.status.success(),
        "install-rules failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The install itself must have recorded an active session covering
    // both families before any consumer starts.
    let recorded = v2ray_rs_antidpi::state::read_journal(&journal)
        .expect("journal must exist after a successful install-rules");
    assert_eq!(recorded.queue, 201, "journal queue");
    assert_eq!(
        recorded.phase,
        v2ray_rs_antidpi::state::Phase::Active,
        "journal must be active after a successful install"
    );
    assert!(
        recorded.ipv4 && recorded.ipv6,
        "journal must record both families, got v4={} v6={}",
        recorded.ipv4,
        recorded.ipv6
    );

    // Presence in each family is asserted on a command that exited zero,
    // never inferred from a failed call.
    let families_present = |label: &str| {
        for family in ["ip", "ip6"] {
            let (code, text) = ns_output(&ns.name, "nft", &["list", "table", family, &table]);
            assert!(
                code == 0 && text.contains(&format!("table {family} {table}")),
                "{label}: {family} table not confirmed (exit {code}): {text}"
            );
        }
    };
    families_present("after install-rules");

    // Managed run with the recorded session: it must verify the journal,
    // bind queue 201, print the ready sentinel, and stay alive. A sentinel
    // without a live process, or a live process without the sentinel, both
    // fail here.
    let mut engine = KillOnDrop(
        Command::new("ip")
            .args(["netns", "exec", &ns.name, BIN])
            .args([
                "run",
                "--queue",
                "201",
                "--session-id",
                &recorded.session_id.to_string(),
                "--strategy",
                "multisplit:pos=midsld",
            ])
            .env("V2RAYRS_ANTIDPI_STATE", journal_s)
            .stdout(Stdio::piped())
            .stderr({
                let log =
                    std::fs::File::create(dir.path().join("engine.log")).expect("engine log file");
                Stdio::from(log)
            })
            .spawn()
            .expect("spawn engine inside netns"),
    );

    let sentinel = format!("{}201", v2ray_rs_antidpi::engine::READY_SENTINEL_PREFIX);
    let (tx, rx) = std::sync::mpsc::channel();
    let stdout = engine.0.stdout.take().expect("piped engine stdout");
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        let mut lines = std::io::BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match lines.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line.trim_end().to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ready = false;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(line) if line.starts_with(&sentinel) => {
                ready = true;
                break;
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let engine_log = std::fs::read_to_string(dir.path().join("engine.log")).unwrap_or_default();
    assert!(
        ready,
        "engine never reached the ready sentinel ({sentinel}): {engine_log}"
    );
    assert!(
        engine.0.try_wait().expect("engine status").is_none(),
        "engine exited right after readiness: {engine_log}"
    );

    // While the consumer holds the queue, both families must still be
    // readable: readiness plus liveness plus intact rules.
    families_present("while engine is bound");

    // Graceful stop: SIGTERM flips the engine's shutdown flag. Escalate to
    // SIGKILL only after a bounded unconfirmed wait.
    unsafe {
        assert_eq!(
            libc::kill(engine.0.id() as libc::pid_t, libc::SIGTERM),
            0,
            "SIGTERM to the engine failed"
        );
    }
    let Some(status) = wait_exit(&mut engine.0, Duration::from_secs(10)) else {
        let _ = unsafe { libc::kill(engine.0.id() as libc::pid_t, libc::SIGKILL) };
        let _ = engine.0.wait();
        panic!("engine ignored SIGTERM for 10s");
    };
    let _ = reader.join();
    assert!(
        status.success(),
        "engine exited unsuccessfully after SIGTERM: {status}"
    );

    // Removal by the same controller identity: both families and the
    // journal must go.
    let out = helper(&["remove-rules"]);
    assert!(
        out.status.success(),
        "remove-rules failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Absence needs a successful listing: `nft list tables` exits zero
    // whether or not anything matched, so a missing name on a zero exit is
    // evidence, while a failed `list table` call could also mean a broken
    // inspection.
    let (code, listed) = ns_output(&ns.name, "nft", &["list", "tables"]);
    assert_eq!(code, 0, "nft list tables failed: {listed}");
    for family in ["ip", "ip6"] {
        assert!(
            !listed.contains(&format!("table {family} {table}")),
            "{family} table still present after remove-rules: {listed}"
        );
    }

    // The journal must be gone specifically: any other read failure is a
    // broken cleanup, not a completed one.
    match std::fs::metadata(&journal) {
        Ok(_) => panic!("journal still present after remove-rules"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("journal read failed after remove-rules: {e}"),
    }
}

/// Root temp dirs default to 0755; the helper refuses a journal whose
/// parent is not owner-only, so the lab parent is locked down first.
fn private_dir(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .expect("make the lab journal parent owner-only");
}

/// TX and RX packet counters of a device inside a namespace.
fn link_packets(ns: &str, dev: &str) -> (u64, u64) {
    let (_, out) = ns_output(ns, "ip", &["-s", "link", "show", dev]);
    let parse = |label: &str| -> u64 {
        let mut lines = out.lines();
        while let Some(line) = lines.next() {
            if !line.contains(label) {
                continue;
            }
            let Some(nums) = lines.next() else {
                break;
            };
            return nums
                .split_whitespace()
                .nth(1)
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
        }
        0
    };
    (parse("TX:"), parse("RX:"))
}

/// End-to-end against the real kernel: `install-rules` programs the owned
/// `ip` table with the queue and leaves the `ip6` table as a marker-only
/// ownership artifact, then real TCP/443, UDP/443, and IPv6/443 traffic
/// crosses the namespaces with the default chains attached to queue 200.
/// The bulk client sends one sub-MSS line at a time and verifies each
/// reversed echo, so no outbound GSO forms and one skb stays one queue
/// message: the `ct packets` rule counters and the trace lines must both
/// stay inside the encoded window of 1..=12 matched skbs per direction.
/// The QUIC fake must fire on the first Initial. The IPv6 handshake runs
/// only when the lab namespace has
/// IPv6 (the marker-only `ip6` table assertion always runs), because a
/// kernel booted with `ipv6.disable=1` cannot carry the addresses. Skipped
/// without root, the tools, or a usable netns; fails under
/// `V2RAY_RS_REQUIRE_NETNS=1`.
#[test]
fn installed_rules_queue_the_first_packets_and_leave_ipv6_alone() {
    if !prerequisites_met(traffic_unsupported_reason()) {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    private_dir(dir.path());
    let mut lab = Lab::new("rt", dir.path());

    // IPv6 endpoints for the pass-through check, and offloads disabled on
    // both veth ends to keep skbs small. The bulk client below still
    // paces itself: an unpaced bulk upload forms outbound GSO before
    // egress on this kernel even with the offloads off, and NFQUEUE
    // delivers each GSO skb as separate MSS messages.
    // Lab::new names the B-side peer `vb-<tag>` and keeps only `veth_a`.
    let veth_b = lab.veth_a.replacen("va-", "vb-", 1);
    let v6_a = ns_exec(
        &lab.ns_a,
        "ip",
        &["addr", "add", "fd00::2/64", "dev", &lab.veth_a, "nodad"],
    );
    let v6_b = ns_exec(
        &lab.ns_b,
        "ip",
        &["addr", "add", "fd00::1/64", "dev", &veth_b, "nodad"],
    );
    let ipv6_ok = v6_a && v6_b;
    let ethtool_ok = ns_exec(
        &lab.ns_a,
        "ethtool",
        &["-K", &lab.veth_a, "tso", "off", "gso", "off", "gro", "off"],
    ) && ns_exec(
        &lab.ns_b,
        "ethtool",
        &["-K", &veth_b, "tso", "off", "gso", "off", "gro", "off"],
    );
    assert!(ethtool_ok, "ethtool could not disable offloads");

    // Documented helper prerequisites, set here because the helper never
    // writes sysctls itself:
    // - `nf_conntrack_acct=1` turns on per-flow packet/byte accounting;
    //   without it `ct packets` reads zero and the queue rules never match.
    // - `nf_conntrack_tcp_be_liberal=1` keeps out-of-window rawsend decoys
    //   (`tcp_seq` offsets, seqovl pieces) from being marked INVALID, so
    //   the conntrack entry survives the whole session and its `ct
    //   packets` window stays continuous.
    for ns in [&lab.ns_a, &lab.ns_b] {
        for key in [
            "net.netfilter.nf_conntrack_acct=1",
            "net.netfilter.nf_conntrack_tcp_be_liberal=1",
        ] {
            assert!(
                ns_exec(ns, "sysctl", &["-qw", key]),
                "cannot set {key} in {ns}"
            );
        }
    }

    start_echo_server(&mut lab);

    // Fresh install with the default chains (no --strategy), under the
    // same controller identity contract as the lifecycle test.
    let session = uuid::Uuid::new_v4();
    let journal = dir.path().join("antidpi.v1.json");
    let journal_s = journal.to_str().unwrap();
    let table = v2ray_rs_antidpi::fw::table_name();
    let install = Command::new("ip")
        .args(["netns", "exec", &lab.ns_a, BIN])
        .args([
            "install-rules",
            "--queue",
            "200",
            "--session-id",
            &session.to_string(),
        ])
        .env("V2RAYRS_ANTIDPI_STATE", journal_s)
        .output()
        .expect("install-rules inside netns");
    assert!(
        install.status.success(),
        "install-rules failed: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    // The ip6 table must exist but never queue; only the ip table queues.
    let (code, v6) = ns_output(&lab.ns_a, "nft", &["list", "table", "ip6", &table]);
    assert!(
        code == 0 && v6.contains(&format!("table ip6 {table}")),
        "ip6 table readback: {v6}"
    );
    assert!(!v6.contains("queue"), "ip6 table must not queue: {v6}");
    let (code, v4) = ns_output(&lab.ns_a, "nft", &["list", "table", "ip", &table]);
    assert!(
        code == 0 && v4.contains("queue"),
        "ip table must queue (exit {code}): {v4}"
    );

    // Engine with the default chains and trace logging into the shared log.
    let engine_log = std::fs::File::create(lab.log_path("engine")).expect("engine log file");
    let engine_out = engine_log.try_clone().expect("clone engine log file");
    let mut engine = KillOnDrop(
        Command::new("ip")
            .args(["netns", "exec", &lab.ns_a, "env"])
            .args([
                &format!("V2RAYRS_ANTIDPI_STATE={journal_s}"),
                "RUST_LOG=trace",
                BIN,
                "run",
                "--queue",
                "200",
                "--session-id",
                &session.to_string(),
            ])
            .stdout(Stdio::from(engine_out))
            .stderr(Stdio::from(engine_log))
            .spawn()
            .expect("spawn engine inside netns"),
    );
    // KillOnDrop kills the engine before Lab::drop tears the namespaces
    // down; tracking it here too would move the child out.
    let sentinel = format!("{}200", v2ray_rs_antidpi::engine::READY_SENTINEL_PREFIX);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let log = std::fs::read_to_string(lab.log_path("engine")).unwrap_or_default();
        if log.contains(&sentinel) {
            break;
        }
        assert!(
            engine.0.try_wait().expect("engine status").is_none(),
            "engine exited before readiness: {log}"
        );
        assert!(
            Instant::now() < deadline,
            "engine never became ready: {log}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    // Bulk TCP/443 in both directions with a stop-and-wait client: it
    // writes one newline-terminated sub-MSS line, waits for `s_server
    // -rev` to echo the exact reversed bytes, and only then writes the
    // next line. At most one sub-MSS write is ever in flight, so TCP
    // cannot coalesce bulk writes into outbound GSO skbs at the output
    // hook and one skb stays one queue message. `s_server -rev` reverses
    // the received bytes but keeps the trailing newline in place: the echo
    // of "0000xxx\n" starts with 'x' and ends with "0000\n", so the client
    // verifies the reversed body followed by the newline.
    use std::io::{BufReader, Read, Write};

    const LINES: usize = 500;
    let (tx0, rx0) = link_packets(&lab.ns_a, &lab.veth_a);
    let bulk_log = std::fs::File::create(lab.log_path("bulk")).expect("bulk log file");
    let mut client = KillOnDrop(
        Command::new("ip")
            .args([
                "netns",
                "exec",
                &lab.ns_a,
                "timeout",
                "25",
                "openssl",
                "s_client",
                "-connect",
                "10.205.0.1:443",
                "-servername",
                DOMAIN,
                "-groups",
                "X25519",
                "-quiet",
                "-no_ign_eof",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(bulk_log))
            .spawn()
            .expect("spawn tls client inside netns"),
    );
    let mut stdin = client.0.stdin.take().expect("client stdin");
    let stdout = client.0.stdout.take().expect("client stdout");
    let mut reader = BufReader::new(stdout);
    let mut echo = Vec::new();
    for i in 0..LINES {
        let sent = format!("{i:04}{}\n", "x".repeat(985));
        if let Err(e) = stdin
            .write_all(sent.as_bytes())
            .and_then(|()| stdin.flush())
        {
            lab.dump_log("bulk");
            lab.dump_log("engine");
            panic!("bulk client died before line {i}: {e}");
        }
        echo.resize(sent.len(), 0);
        if let Err(e) = reader.read_exact(&mut echo) {
            lab.dump_log("bulk");
            lab.dump_log("engine");
            panic!("bulk echo short at line {i}: {e}");
        }
        let mut want: Vec<u8> = sent.as_bytes()[..sent.len() - 1]
            .iter()
            .rev()
            .copied()
            .collect();
        want.push(b'\n');
        if echo != want {
            let hex =
                |b: &[u8]| -> String { b.iter().map(|x| format!("{x:02x}")).collect::<String>() };
            let n = echo.len();
            lab.dump_log("bulk");
            lab.dump_log("engine");
            panic!(
                "bulk echo mismatch at line {i}: echo[..16]={} echo[-16..]={} want[..16]={}",
                hex(&echo[..16.min(n)]),
                hex(&echo[n.saturating_sub(16)..]),
                hex(&want[..16]),
            );
        }
    }
    drop(stdin);
    // stdin EOF closes the TLS session (`-no_ign_eof`); the client must
    // exit on its own, and a hang is a failure with its stderr as
    // evidence. KillOnDrop still cleans up on every path.
    if wait_exit(&mut client.0, Duration::from_secs(10)).is_none() {
        lab.dump_log("bulk");
        lab.dump_log("engine");
        panic!("bulk client did not exit after stdin EOF");
    }
    let (tx1, rx1) = link_packets(&lab.ns_a, &lab.veth_a);
    assert!(
        tx1.saturating_sub(tx0) >= 300,
        "too few TX packets: {tx0} -> {tx1}"
    );
    assert!(
        rx1.saturating_sub(rx0) >= 300,
        "too few RX packets: {rx0} -> {rx1}"
    );

    // One raw QUIC Initial from the probe binary: the default UDP chain
    // fires the QUIC fake on the very first datagram.
    let initial = v2ray_rs_antidpi::quic::craft_quic_initial(
        &mut v2ray_rs_antidpi::tls::rand_bytes::Counter(0),
    );
    let pkt =
        v2ray_rs_antidpi::packet::OutBuilder::udp([10, 205, 0, 2], [10, 205, 0, 1], 55555, 443)
            .payload(initial)
            .build()
            .expect("valid packet");
    let hex: String = pkt.bytes.iter().map(|b| format!("{b:02x}")).collect();
    let (_, probe_out) = ns_output(
        &lab.ns_a,
        PROBE,
        &[
            "--udp",
            "--dst",
            "10.205.0.1",
            "--port",
            "443",
            "--mark",
            "0",
            "--bytes-hex",
            &hex,
        ],
    );
    assert!(
        probe_out.contains("sent #0"),
        "probe did not send: {probe_out}"
    );

    // IPv6/443 with no queue rule at all must still handshake — but only
    // when the kernel could actually attach the addresses; a host booted
    // with `ipv6.disable=1` answers `RTNETLINK: Operation not supported`.
    if ipv6_ok {
        let (code, v6_hs) = ns_output(
            &lab.ns_a,
            "timeout",
            &[
                "8",
                "openssl",
                "s_client",
                "-connect",
                "[fd00::1]:443",
                "-servername",
                DOMAIN,
                "-groups",
                "X25519",
                "-brief",
            ],
        );
        assert!(
            code == 0 && handshake_ok(&v6_hs),
            "ipv6 handshake failed (exit {code}): {v6_hs}"
        );
    } else {
        eprintln!(
            "ipv6 unavailable in the lab namespace; skipping the ipv6 handshake (address adds: veth_a ok={v6_a} veth_b ok={v6_b})"
        );
    }

    // Trace contract. The stop-and-wait bulk client keeps at most one
    // sub-MSS skb in flight, so no outbound GSO forms and NFQUEUE delivers
    // one message per matched skb: the trace counts ARE the matched skb
    // counts. The encoded `ct packets` window 1..=12 must hold per
    // direction (the broken native-endian encoding matched 11 extra skbs
    // per 256 packets over this ~750-packet flow). Nothing unparseable may
    // arrive, and the raw QUIC Initial must fire exactly one fake burst.
    let log = std::fs::read_to_string(lab.log_path("engine")).unwrap_or_default();
    let count = |needle: &str| log.lines().filter(|l| l.contains(needle)).count();
    let out_tcp = count("queued: out tcp") as u64;
    let in_tcp = count("queued: in tcp") as u64;
    let unparseable = count("queued: pass unparseable");
    let quic = count("quic fake sent x11");
    let (_, table_dump) = ns_output(&lab.ns_a, "nft", &["list", "table", "ip", &table]);
    let counts = format!(
        "counts: out_tcp={out_tcp} in_tcp={in_tcp} unparseable={unparseable} quic={quic}\n--- nft list table ip {table} ---\n{table_dump}\n--- engine log ---\n{log}"
    );
    eprintln!(
        "ACCEPTANCE-COUNTS out_tcp={out_tcp} in_tcp={in_tcp} unparseable={unparseable} quic={quic}"
    );
    assert!(
        (2..=12).contains(&out_tcp),
        "queued out tcp {out_tcp} outside 2..=12: {counts}"
    );
    assert!(in_tcp <= 12, "queued in tcp {in_tcp} above 12: {counts}");
    assert_eq!(
        unparseable, 0,
        "unparseable packets reached the queue: {counts}"
    );
    assert_eq!(quic, 1, "expected exactly one quic fake burst: {counts}");
}
