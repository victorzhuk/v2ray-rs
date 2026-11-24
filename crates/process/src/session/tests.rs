use super::builders::*;
use super::cleanup::*;
use super::supervisor::*;
use super::*;

fn antidpi_stub_path() -> &'static Path {
    Path::new("/nonexistent/v2ray-rs-antidpi")
}

#[test]
fn build_antidpi_runtime_gates_on_the_enabled_switch() {
    let mut settings = AppSettings::default();
    assert!(matches!(
        build_antidpi_runtime(&settings, antidpi_stub_path()),
        Ok(None)
    ));

    settings.antidpi.enabled = true;
    settings.antidpi.queue = 9;
    settings.antidpi.strategies = vec!["fake:blob=quic:repeats=11".to_string()];
    settings.antidpi.hostlist_path = Some("/tmp/hosts.txt".to_string());
    let rt = build_antidpi_runtime(&settings, antidpi_stub_path())
        .expect("valid settings build a runtime")
        .expect("enabled settings build a runtime");
    assert_eq!(rt.queue, 9);
    assert_eq!(rt.strategies, ["fake:blob=quic:repeats=11"]);
    assert_eq!(rt.hostlist, Some(PathBuf::from("/tmp/hosts.txt")));
    assert!(!rt.helper_path.as_os_str().is_empty());
}

#[test]
fn build_antidpi_runtime_rejects_an_invalid_hand_edited_config() {
    let mut settings = AppSettings::default();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 0;
    let err = build_antidpi_runtime(&settings, antidpi_stub_path())
        .expect_err("queue 0 must fail the candidate");
    assert!(err.contains("queue"), "{err}");

    settings.antidpi.queue = 200;
    settings.antidpi.strategies = (0..17).map(|i| format!("multisplit:pos={i}")).collect();
    let err = build_antidpi_runtime(&settings, antidpi_stub_path())
        .expect_err("17 strategies must fail the candidate");
    assert!(err.contains("strategies"), "{err}");
}
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn burst_flushes_as_one_ordered_batch() {
    let mut batcher = LogBatcher::default();
    for i in 0..100 {
        batcher.push(format!("line-{i}"));
    }

    let batch = batcher.take().expect("pending burst");
    assert_eq!(batch.dropped, 0);
    let expected: Vec<String> = (0..100).map(|i| format!("line-{i}")).collect();
    assert_eq!(batch.lines, expected);
    assert!(batcher.take().is_none(), "batch must drain empty");
}

#[test]
fn full_batch_drops_oldest_and_reports_the_count() {
    let mut batcher = LogBatcher::default();
    let extra = 10;
    for i in 0..(LOG_BATCH_CAP + extra) {
        batcher.push(format!("line-{i}"));
    }

    let batch = batcher.take().expect("pending batch");
    assert_eq!(batch.dropped, extra);
    assert_eq!(batch.lines.len(), LOG_BATCH_CAP);
    assert_eq!(batch.lines[0], format!("line-{extra}"));
    assert_eq!(
        batch.lines.last().map(String::as_str),
        Some(format!("line-{}", LOG_BATCH_CAP + extra - 1)).as_deref()
    );
}

#[test]
fn partial_batch_flushes_on_take() {
    let mut batcher = LogBatcher::default();
    batcher.push("a".into());
    batcher.push("b".into());
    batcher.push("c".into());

    let batch = batcher.take().expect("pending partial batch");
    assert_eq!(batch.lines, ["a", "b", "c"]);
    assert_eq!(batch.dropped, 0);
    assert!(batcher.take().is_none());
}

#[tokio::test]
async fn flush_reports_dropped_lines_with_a_marker() {
    let (sender, mut rx) = EventSender::channel();
    flush_log_batch(
        &sender,
        GENERATION,
        Some(LogBatch {
            lines: vec!["kept".into()],
            dropped: 2,
        }),
    );
    drop(sender);

    match rx.recv().await.expect("batch message") {
        SessionEvent::LogLines(generation, lines) => {
            assert_eq!(generation, GENERATION);
            assert_eq!(lines.len(), 2);
            assert!(lines[0].contains('2'), "marker must report the count");
            assert_eq!(lines[1], "kept");
        }
        other => panic!("unexpected message: {other:?}"),
    }
}
use v2ray_rs_core::models::{
    DnsConfig, DnsProtocol, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy, ImportedProfile,
    RuleAction, RuleMatch, ShadowsocksConfig, TransportSettings, TunConfig, VlessConfig,
    XhttpSettings,
};
use v2ray_rs_core::persistence::load_tun_session;
use v2ray_rs_core::profile::AppProfile;

use super::health::Health;

struct TestLogCapture {
    lines: std::sync::Mutex<Vec<String>>,
}

impl TestLogCapture {
    fn lines_containing(&self, needle: &str) -> Vec<String> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(needle))
            .cloned()
            .collect()
    }

    fn all_lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

impl log::Log for TestLogCapture {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.lines
                .lock()
                .unwrap()
                .push(format!("{}", record.args()));
        }
    }

    fn flush(&self) {}
}

fn install_test_capture() -> &'static TestLogCapture {
    static CAPTURE: TestLogCapture = TestLogCapture {
        lines: std::sync::Mutex::new(Vec::new()),
    };
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("no other logger installed in tests");
        log::set_max_level(log::LevelFilter::Info);
    });
    &CAPTURE
}

const GENERATION: u64 = 7;
const RECV_TIMEOUT: Duration = Duration::from_secs(20);

struct Stub {
    _tmp: tempfile::TempDir,
    paths: AppPaths,
    binary: PathBuf,
}

fn stub(script: &str) -> Stub {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();
    let binary = tmp.path().join("backend");
    std::fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    Stub {
        _tmp: tmp,
        paths,
        binary,
    }
}

fn executable(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn candidate(address: &str) -> ConnectionCandidate {
    ConnectionCandidate {
        node_ref: ConnectionNodeRef::Manual {
            node_id: uuid::Uuid::new_v4(),
        },
        source_name: "test".into(),
        node: node(address),
        latency_ms: None,
        real_delay_ms: None,
    }
}

fn connect(
    stub: &Stub,
    settings: AppSettings,
    candidates: Vec<ConnectionCandidate>,
) -> (SessionHandle, mpsc::UnboundedReceiver<SessionEvent>) {
    connect_with(stub, settings, candidates, |mgr| mgr)
}

fn no_subnets(_iface: &str) -> std::io::Result<Vec<ipnet::IpNet>> {
    Ok(Vec::new())
}

fn request(
    stub: &Stub,
    settings: AppSettings,
    candidates: Vec<ConnectionCandidate>,
) -> SessionRequest {
    SessionRequest {
        binary_path: stub.binary.clone(),
        candidates,
        writer: ConfigWriter::new(&settings, &stub.paths),
        paths: stub.paths.clone(),
        pid_path: stub.paths.pid_file_path(),
        geodata_dir: stub.paths.geodata_dir(),
        settings,
        enabled_rules: Vec::new(),
        subscriptions: Vec::new(),
        manual_nodes: Vec::new(),
        lifecycle: TunLifecycle::default(),
        generation: GENERATION,
        host_has_ipv6: true,
        health_timing: HealthTiming::default(),
        last_good_pins: Vec::new(),
    }
}

const FAST_HEALTH: HealthTiming = HealthTiming {
    initial_delay: Duration::from_millis(50),
    interval: Duration::from_millis(100),
    dns_recheck: Duration::from_millis(100),
};

fn connect_with_health(
    stub: &Stub,
    settings: AppSettings,
    candidates: Vec<ConnectionCandidate>,
    health_timing: HealthTiming,
) -> (SessionHandle, mpsc::UnboundedReceiver<SessionEvent>) {
    let (tx, rx) = EventSender::channel();
    let req = SessionRequest {
        health_timing,
        ..request(stub, settings, candidates)
    };
    let handle = spawn_with(req, tx, |mgr| mgr, no_subnets);
    (handle, rx)
}

fn connect_with(
    stub: &Stub,
    settings: AppSettings,
    candidates: Vec<ConnectionCandidate>,
    configure: impl Fn(ProcessManager) -> ProcessManager + Send + 'static,
) -> (SessionHandle, mpsc::UnboundedReceiver<SessionEvent>) {
    let (tx, rx) = EventSender::channel();
    let handle = spawn_with(
        request(stub, settings, candidates),
        tx,
        configure,
        no_subnets,
    );
    (handle, rx)
}

fn singbox_settings() -> AppSettings {
    let mut settings = AppSettings::default();
    settings.backend.backend_type = BackendType::SingBox;
    settings
}

/// Settings whose SOCKS port is a live listener, so a stub backend passes
/// the readiness probe. The listener must outlive the connection under test.
fn ready_singbox_settings() -> (std::net::TcpListener, AppSettings) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut settings = singbox_settings();
    settings.socks_port = listener.local_addr().unwrap().port();
    (listener, settings)
}

async fn next_state(
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
) -> (ProcessState, Option<ConnectionMetadata>) {
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no state reported in time")
            .expect("connection task ended without a terminal state");
        if let SessionEvent::State(generation, state, connection) = msg {
            assert_eq!(generation, GENERATION);
            return (state, connection);
        }
    }
}

/// The task drops every sender when it returns, so a closed channel with
/// no state in between proves nothing followed the terminal state.
async fn assert_nothing_after_terminal(rx: &mut mpsc::UnboundedReceiver<SessionEvent>) {
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => return,
            Some(SessionEvent::State(_, state, _)) => {
                panic!("{state:?} reported after the terminal state")
            }
            Some(_) => {}
        }
    }
}

#[test]
fn forwarder_relays_only_nonterminal_states() {
    assert!(relays(&ProcessState::Starting));
    assert!(relays(&ProcessState::Running));
    assert!(relays(&ProcessState::Stopping));
    assert!(!relays(&ProcessState::Stopped));
    assert!(!relays(&ProcessState::Error("boom".into())));
}

#[test]
fn failure_key_ignores_the_singbox_message_counter() {
    let first = failure_key(
        "\u{1b}[31mFATAL\u{1b}[0m[0000] start service: initialize inbound/tun[0]: configure tun interface: operation not permitted",
        "203.0.113.1",
        "203.0.113.1",
        8388,
    );
    let second = failure_key(
        "\u{1b}[31mFATAL\u{1b}[0m[0001] start service: initialize inbound/tun[0]: configure tun interface: operation not permitted",
        "203.0.113.2",
        "203.0.113.2",
        8388,
    );
    assert_eq!(first, second);
    assert!(!first.contains('\u{1b}'), "{first}");
}

#[test]
fn failure_key_ignores_xray_timestamps() {
    let first = failure_key(
        "2026/09/14 10:37:35.309646 [Warning] failed to handle connection",
        "203.0.113.1",
        "203.0.113.1",
        443,
    );
    let second = failure_key(
        "2026/09/14 10:41:02.884131 [Warning] failed to handle connection",
        "203.0.113.2",
        "203.0.113.2",
        443,
    );
    assert_eq!(first, second);
    assert!(first.starts_with("[Warning]"), "{first}");
}

#[test]
fn failure_key_ignores_each_candidates_own_host() {
    let first = failure_key(
        "tls: failed to verify certificate for 203.0.113.1:443",
        "203.0.113.1",
        "203.0.113.1",
        443,
    );
    let second = failure_key(
        "tls: failed to verify certificate for 203.0.113.2:443",
        "203.0.113.2",
        "203.0.113.2",
        443,
    );
    assert_eq!(first, second);
    assert_eq!(first, "tls: failed to verify certificate for <node>:<port>");
}

#[test]
fn failure_key_keeps_unrelated_hosts_distinct() {
    let first = failure_key(
        "tls: handshake with updates.example.com failed",
        "203.0.113.1",
        "203.0.113.1",
        443,
    );
    let second = failure_key(
        "tls: handshake with mirror.example.net failed",
        "203.0.113.2",
        "203.0.113.2",
        443,
    );
    assert_ne!(first, second);
}

#[test]
fn failure_key_keeps_a_port_inside_a_longer_number() {
    let key = failure_key("dial tcp 10.4.43.1:14430: refused", "node", "node", 443);
    assert_eq!(key, "dial tcp 10.4.43.1:14430: refused");
}

#[test]
fn failure_key_keeps_a_bare_port_number_in_prose() {
    let first = failure_key(
        "upstream updates.example.com reports 80% packet loss",
        "203.0.113.1",
        "203.0.113.1",
        80,
    );
    let second = failure_key(
        "upstream mirror.example.net reports 80% packet loss",
        "203.0.113.2",
        "203.0.113.2",
        80,
    );
    assert!(first.contains("80% packet loss"), "{first}");
    assert_ne!(first, second);

    let addressed = failure_key(
        "dial tcp 203.0.113.1:80: refused",
        "node",
        "203.0.113.1",
        80,
    );
    assert_eq!(addressed, "dial tcp <node>:<port>: refused");
}

#[test]
fn failure_key_keeps_a_midstring_timestamp() {
    let first = failure_key(
        "[Warning] handshake started at 2026/09/14 10:37:35 failed",
        "203.0.113.1",
        "203.0.113.1",
        443,
    );
    let second = failure_key(
        "[Warning] handshake started at 2026/09/14 10:41:02 failed",
        "203.0.113.2",
        "203.0.113.2",
        443,
    );
    assert!(first.contains("2026/09/14 10:37:35"), "{first}");
    assert_ne!(first, second);
}

#[test]
fn summarize_failures_strips_ansi() {
    let summary = summarize_failures(&[CandidateFailure::new(
        "203.0.113.1",
        "\u{1b}[31mFATAL\u{1b}[0m[0000] configure tun interface: operation not permitted",
        "203.0.113.1",
        443,
    )]);
    assert!(!summary.contains('\u{1b}'), "{summary}");
    assert!(summary.contains("FATAL[0000]"), "{summary}");
}

#[test]
fn repeats_previous_needs_two_equal_keys() {
    let failure = |label: &str, reason: &str| CandidateFailure::new(label, reason, label, 443);
    let shared = "\u{1b}[31mFATAL\u{1b}[0m[0000] configure tun interface: operation not permitted";

    assert!(!repeats_previous(&[]));
    assert!(!repeats_previous(&[failure("203.0.113.1", shared)]));
    assert!(repeats_previous(&[
        failure("203.0.113.1", shared),
        failure("203.0.113.2", shared),
    ]));
    assert!(!repeats_previous(&[
        failure("203.0.113.1", shared),
        failure("203.0.113.2", "config rejected"),
    ]));
}

#[test]
fn strict_route_allowed_unless_singbox_lacks_ipv6() {
    let cases = [
        // (backend, host_has_ipv6, want)
        (BackendType::SingBox, false, false),
        (BackendType::SingBox, true, true),
        (BackendType::Xray, false, true),
        (BackendType::V2ray, false, true),
    ];
    for (backend, host_has_ipv6, want) in cases {
        assert_eq!(
            singbox_strict_route_allowed(backend, host_has_ipv6),
            want,
            "backend={backend:?} host_has_ipv6={host_has_ipv6}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_reason_reaches_the_exit_record() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Running) {
            break;
        }
        assert!(relays(&state), "start reported {state:?}");
    }

    let _ = handle.stop(StopReason::NodeSwitch);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log")).unwrap();
    let exit = log
        .lines()
        .find(|l| l.contains(" exit requested="))
        .unwrap_or_else(|| panic!("no exit record in {log}"));
    assert!(exit.contains("reason=node-switch"), "{exit}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_reports_no_stopped_and_stop_reports_one() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
    );

    loop {
        let (state, connection) = next_state(&mut rx).await;
        assert!(relays(&state), "failover reported {state:?}");
        if matches!(state, ProcessState::Running)
            && connection.is_some_and(|c| c.node_address == "203.0.113.2")
        {
            break;
        }
    }

    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn last_candidate_failure_reports_one_error() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && { grep -q 203.0.113.3 "$3" && exit 1; exit 0; }; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
    );
    let (_handle, mut rx) = connect(
        &stub,
        singbox_settings(),
        vec![candidate("203.0.113.1"), candidate("203.0.113.3")],
    );

    let msg = loop {
        match next_state(&mut rx).await {
            (ProcessState::Error(msg), _) => break msg,
            (state, _) => assert!(relays(&state), "failover reported {state:?}"),
        }
    };
    assert!(msg.starts_with("All candidates failed"), "{msg}");
    assert!(
        msg.contains("203.0.113.1: process exited with code 1"),
        "{msg}"
    );
    assert!(msg.contains("203.0.113.3: config rejected"), "{msg}");
    assert_nothing_after_terminal(&mut rx).await;
}

/// Lines the stub appended, one per candidate it was asked to check.
fn attempts(marker: &std::path::Path) -> usize {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn same_failure_on_consecutive_candidates_stops_failover() {
    let launches = tempfile::tempdir().unwrap();
    let marker = launches.path().join("launched");
    let stub = stub(&format!(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0
[ "$1" = check ] || exec sleep 30
echo attempt >> {}
printf '\033[31mFATAL\033[0m[0000] configure tun interface: operation not permitted\n' >&2
exit 1"#,
        marker.display()
    ));
    let (_handle, mut rx) = connect(
        &stub,
        singbox_settings(),
        vec![
            candidate("203.0.113.1"),
            candidate("203.0.113.2"),
            candidate("203.0.113.3"),
        ],
    );

    let (terminal, _) = drain(&mut rx).await;
    let Some(ProcessState::Error(msg)) = terminal else {
        panic!("expected a single error terminal, got {terminal:?}");
    };
    assert!(
        msg.starts_with("Connection failed on consecutive nodes with the same error"),
        "{msg}"
    );
    assert!(
        msg.contains("configure tun interface: operation not permitted"),
        "{msg}"
    );
    assert!(!msg.contains('\u{1b}'), "{msg}");
    assert_eq!(attempts(&marker), 2);

    let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
        .expect("generated config readable");
    assert!(config.contains("203.0.113.2"), "{config}");
    assert!(
        !config.contains("203.0.113.3"),
        "third candidate must not be attempted: {config}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn differing_failures_keep_failing_over() {
    let launches = tempfile::tempdir().unwrap();
    let marker = launches.path().join("launched");
    let stub = stub(&format!(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0
[ "$1" = check ] || exec sleep 30
echo attempt >> {}
grep -q 203.0.113.1 "$3" && echo "alpha refused" >&2 && exit 1
grep -q 203.0.113.2 "$3" && echo "bravo refused" >&2 && exit 1
echo "charlie refused" >&2
exit 1"#,
        marker.display()
    ));
    let (_handle, mut rx) = connect(
        &stub,
        singbox_settings(),
        vec![
            candidate("203.0.113.1"),
            candidate("203.0.113.2"),
            candidate("203.0.113.3"),
        ],
    );

    let (terminal, _) = drain(&mut rx).await;
    let Some(ProcessState::Error(msg)) = terminal else {
        panic!("expected a single error terminal, got {terminal:?}");
    };
    assert!(msg.starts_with("All candidates failed"), "{msg}");
    for reason in ["alpha refused", "bravo refused", "charlie refused"] {
        assert!(msg.contains(reason), "{msg}");
    }
    assert!(!msg.contains('\u{1b}'), "{msg}");
    assert_eq!(attempts(&marker), 3);
}

fn subscription_candidate(
    sub: &v2ray_rs_core::models::Subscription,
    idx: usize,
) -> ConnectionCandidate {
    ConnectionCandidate {
        node_ref: ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: sub.nodes[idx].id,
        },
        source_name: sub.name.clone(),
        node: sub.nodes[idx].node.clone(),
        latency_ms: None,
        real_delay_ms: None,
    }
}

fn subscription_with_nodes(addresses: &[&str]) -> v2ray_rs_core::models::Subscription {
    use v2ray_rs_core::models::{ShadowsocksConfig, SubscriptionNode};
    let mut sub =
        v2ray_rs_core::models::Subscription::new_from_url("vpndio", "https://example.com/sub");
    for (i, address) in addresses.iter().enumerate() {
        let mut node = SubscriptionNode::new(ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: (*address).into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "pw".into(),
            remark: Some(format!("node-{i}")),
        }));
        node.enabled = true;
        sub.nodes.push(node);
    }
    sub
}

/// Drains until the terminal state, also counting refresh offers and any
/// direct update-start request.
async fn drain_with_offers(
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
) -> (Option<ProcessState>, usize, bool) {
    let mut terminal = None;
    let mut offers = 0;
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => return (terminal, offers, false),
            Some(SessionEvent::OfferSubscriptionRefresh { .. }) => offers += 1,
            Some(SessionEvent::State(_, state, _)) if !relays(&state) => {
                assert!(terminal.is_none(), "{state:?} after the terminal state");
                terminal = Some(state);
            }
            Some(_) => {}
        }
    }
}

const REALITY_SIGNATURE: &str = "transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)";

#[tokio::test(flavor = "multi_thread")]
async fn reality_rejection_classifies_the_failure_report() {
    let stub = stub(&format!(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0
for i in 1 2 3; do echo "2026/09/14 09:57:30.532567 [Error] [1944052120] {REALITY_SIGNATURE}"; done
exit 1"#
    ));
    let (_listener, settings) = ready_singbox_settings();
    let mut settings = settings;
    settings.backend.backend_type = BackendType::Xray;
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);
    drop(handle);

    let (terminal, offers, update_started) = drain_with_offers(&mut rx).await;
    let Some(ProcessState::Error(msg)) = terminal else {
        panic!("expected an error terminal, got {terminal:?}");
    };
    assert!(
        msg.contains("server rejected REALITY authentication"),
        "{msg}"
    );
    assert!(
        msg.contains("REALITY keys may be stale"),
        "cause must carry the stale-keys advice: {msg}"
    );
    assert!(
        !msg.contains("process exited with code 1"),
        "classified cause replaces the generic text: {msg}"
    );
    // A manual node has no owning subscription: no refresh offer.
    assert_eq!(offers, 0);
    // No update starts on its own.
    assert!(!update_started);
}

#[tokio::test(flavor = "multi_thread")]
async fn reality_rejection_offers_refresh_once_per_subscription() {
    let stub = stub(&format!(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0
grep -q 203.0.113.1 "$3" && {{ for i in 1 2 3; do echo "{REALITY_SIGNATURE}"; done; exit 1; }}
grep -q 203.0.113.4 "$3" && {{ for i in 1 2 3; do echo "{REALITY_SIGNATURE}"; done; exit 1; }}
exec sleep 30"#
    ));
    let (_listener, settings) = ready_singbox_settings();
    let mut settings = settings;
    settings.backend.backend_type = BackendType::Xray;
    let sub = subscription_with_nodes(&["203.0.113.1", "203.0.113.4"]);
    let (tx, mut rx) = EventSender::channel();
    let req = SessionRequest {
        subscriptions: vec![sub.clone()],
        ..request(
            &stub,
            settings,
            vec![
                subscription_candidate(&sub, 0),
                subscription_candidate(&sub, 1),
            ],
        )
    };
    let _handle = spawn_with(req, tx, |mgr| mgr, no_subnets);

    let (terminal, offers, update_started) = drain_with_offers(&mut rx).await;
    let Some(ProcessState::Error(msg)) = terminal else {
        panic!("expected an error terminal, got {terminal:?}");
    };
    assert!(
        msg.contains("server rejected REALITY authentication"),
        "{msg}"
    );
    // Two classified candidates from the same subscription: one offer.
    assert_eq!(offers, 1, "one offer per connection per subscription");
    assert!(!update_started, "refresh stays user-initiated");
}

#[tokio::test(flavor = "multi_thread")]
async fn unmatched_failure_keeps_generic_report() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0
echo "2026/09/14 09:57:31 [Warning] failed to dial tcp: i/o timeout"
exit 1"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let mut settings = settings;
    settings.backend.backend_type = BackendType::Xray;
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);
    drop(handle);

    let (terminal, offers, _) = drain_with_offers(&mut rx).await;
    let Some(ProcessState::Error(msg)) = terminal else {
        panic!("expected an error terminal, got {terminal:?}");
    };
    assert!(msg.contains("process exited with code 1"), "{msg}");
    assert!(!msg.contains("REALITY"), "{msg}");
    assert_eq!(offers, 0);
}

/// Drains the connection until its channel closes, returning the terminal
/// state and every log line it emitted.
async fn drain(
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
) -> (Option<ProcessState>, Vec<String>) {
    let mut terminal = None;
    let mut lines = Vec::new();
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => return (terminal, lines),
            Some(SessionEvent::LogLines(generation, batch)) => {
                assert_eq!(generation, GENERATION);
                lines.extend(batch);
            }
            Some(SessionEvent::State(generation, state, _)) => {
                assert_eq!(generation, GENERATION);
                if !relays(&state) {
                    assert!(terminal.is_none(), "{state:?} after the terminal state");
                    terminal = Some(state);
                }
            }
            Some(_) => {}
        }
    }
}

fn strict_route_stub() -> Stub {
    stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
    )
}

fn strict_route_settings() -> AppSettings {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::SingBox;
    settings.tun.interface_name = "v2rstest2".into();
    settings
}

fn capless_probe(mgr: ProcessManager) -> ProcessManager {
    mgr.with_host_probe(crate::HostProbe {
        getcap: PathBuf::from("/bin/true"),
        helper: PathBuf::from("/bin/true"),
        helper_access: Some(crate::HelperAccess::Ready),
    })
}

fn xhttp_candidate(address: &str) -> ConnectionCandidate {
    ConnectionCandidate {
        node: ProxyNode::Vless(VlessConfig {
            address: address.into(),
            port: 443,
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Xhttp(XhttpSettings {
                path: "/x".into(),
                host: None,
                mode: "auto".into(),
            }),
            tls: None,
            remark: None,
        }),
        ..candidate(address)
    }
}

fn assert_error_terminal(terminal: Option<ProcessState>) {
    assert!(
        matches!(terminal, Some(ProcessState::Error(_))),
        "expected an error terminal, got {terminal:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn singbox_tun_without_ipv6_turns_strict_route_off_once() {
    let stub = strict_route_stub();
    let settings = strict_route_settings();
    assert!(settings.tun.strict_route);
    let mut req = request(
        &stub,
        settings,
        vec![xhttp_candidate("203.0.113.1"), candidate("203.0.113.2")],
    );
    req.host_has_ipv6 = false;
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, lines) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
        .expect("generated config readable");
    assert!(config.contains("203.0.113.2"), "{config}");
    assert!(config.contains(r#""strict_route":false"#), "{config}");

    let notices = lines.iter().filter(|l| *l == STRICT_ROUTE_NOTICE).count();
    assert_eq!(notices, 1, "{lines:?}");
    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(log.matches(STRICT_ROUTE_NOTICE).count(), 1, "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn singbox_tun_with_ipv6_keeps_strict_route() {
    let stub = strict_route_stub();
    let settings = strict_route_settings();
    let mut req = request(&stub, settings, vec![candidate("203.0.113.2")]);
    req.host_has_ipv6 = true;
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, lines) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
        .expect("generated config readable");
    assert!(config.contains(r#""strict_route":true"#), "{config}");
    assert!(!lines.iter().any(|l| l == STRICT_ROUTE_NOTICE), "{lines:?}");
    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert!(!log.contains(STRICT_ROUTE_NOTICE), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn collect_failure_reports_single_error_without_spawn() {
    let stub = stub(r#"exit 0"#);
    let mut settings = singbox_settings();
    settings.tun.enabled = true;
    let req = request(&stub, settings, vec![candidate("203.0.113.1")]);
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(
        req,
        tx,
        |mgr| mgr,
        |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "ifaddrs failed",
            ))
        },
    );

    let (terminal, lines) = drain(&mut rx).await;
    match terminal {
        Some(ProcessState::Error(msg)) => {
            assert!(
                msg.starts_with("Collect connected subnets: "),
                "unexpected error text: {msg}"
            );
            assert!(msg.contains("ifaddrs failed"), "{msg}");
        }
        other => panic!("expected collection error terminal, got {other:?}"),
    }
    // No candidate started, so nothing was written or spawned.
    assert!(
        !lines.iter().any(|line| line.contains("candidate start")),
        "{lines:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn v2ray_tun_warning_logged_once_per_connection() {
    let stub = stub(r#"exit 1"#);
    let settings = v2ray_settings(2080, 2081, "127.0.0.1", true);
    let expected = v2ray_tun_warning(&settings).expect("warning");
    assert!(expected.contains("127.0.0.1:2080"), "{expected}");
    assert!(expected.contains("127.0.0.1:2081"), "{expected}");
    let req = request(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
    );
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, lines) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    assert_eq!(
        lines.iter().filter(|l| *l == &expected).count(),
        1,
        "{lines:?}"
    );
    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(log.matches(&expected).count(), 1, "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn v2ray_without_tun_logs_no_warning() {
    let stub = stub(r#"exit 1"#);
    let expected =
        v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", true)).expect("warning");
    let req = request(
        &stub,
        v2ray_settings(2080, 2081, "127.0.0.1", false),
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
    );
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, lines) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    assert!(!lines.iter().any(|l| l.contains(&expected)), "{lines:?}");
    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert!(!log.contains(&expected), "{log}");
}

#[test]
fn grant_fixable_is_exactly_the_capability_pair() {
    let fixable = [
        ProcessError::TunCapabilityMissing(PathBuf::from("/usr/bin/xray")),
        ProcessError::TunHelperCapabilityMissing(PathBuf::from("/usr/local/bin/v2ray-rs-netctl")),
        ProcessError::TunHelperEnrollmentRequired,
        ProcessError::TunHelperPermissions(PathBuf::from("/usr/local/bin/v2ray-rs-netctl")),
        ProcessError::AntiDpiHelperCapabilityMissing(PathBuf::from(
            "/usr/local/bin/v2ray-rs-antidpi",
        )),
        ProcessError::AntiDpiHelperEnrollmentRequired,
        ProcessError::AntiDpiHelperPermissions(PathBuf::from("/usr/local/bin/v2ray-rs-antidpi")),
    ];
    assert!(fixable.iter().all(grant_fixable));

    let other_host_level = [
        ProcessError::TunCapabilityProbe("getcap not installed".into()),
        ProcessError::TunMountUnsupported("/dev/net/tun: required key not available".into()),
        ProcessError::TunHelperMissing,
        ProcessError::TunHelperRelogin,
        ProcessError::TunHelperNoExec(PathBuf::from("/home/user/.local/bin/v2ray-rs-netctl")),
        ProcessError::TunHelperAccessProbe {
            path: PathBuf::from("/usr/local/bin/v2ray-rs-netctl"),
            detail: "mount inspection failed".into(),
        },
        ProcessError::AntiDpiHelperMissing,
        ProcessError::AntiDpiHelperRelogin,
        ProcessError::AntiDpiHelperNoExec(PathBuf::from("/home/user/.local/bin/v2ray-rs-antidpi")),
        ProcessError::AntiDpiHelperAccessProbe {
            path: PathBuf::from("/usr/local/bin/v2ray-rs-antidpi"),
            detail: "mount inspection failed".into(),
        },
        ProcessError::AntiDpiCapabilityProbe("getcap not installed".into()),
        ProcessError::AntiDpiHelper("install-rules failed: exit status 1".into()),
        ProcessError::BackendTooOld {
            backend: BackendType::Xray,
            installed: "25.3.5".into(),
            required: "26.1.13".into(),
        },
    ];
    assert!(other_host_level.iter().all(|e| !grant_fixable(e)));
    assert!(!grant_fixable(&ProcessError::ConfigCheck(
        "rejected".into()
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn host_level_failure_stops_the_candidate_loop() {
    let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
    // A getcap that prints nothing is the "no capabilities" verdict, and a
    // present helper gets the start past the helper gates to that verdict.
    // `/bin/true` rather than a freshly written script: exec'ing a file a
    // parallel test may still hold open for writing fails with ETXTBSY.
    let host = tempfile::tempdir().unwrap();
    let probe = crate::HostProbe {
        getcap: PathBuf::from("/bin/true"),
        helper: executable(host.path(), "v2ray-rs-netctl"),
        helper_access: Some(crate::HelperAccess::Ready),
    };
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "v2rstest1".into();
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let mut granted = false;
    let terminal = loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no message in time")
            .expect("connection task ended without a terminal state");
        match msg {
            SessionEvent::TunGrantRequired(generation) => {
                assert_eq!(generation, GENERATION);
                granted = true;
            }
            SessionEvent::State(generation, state, _) => {
                assert_eq!(generation, GENERATION);
                break state;
            }
            _ => {}
        }
    };

    let ProcessState::Error(msg) = terminal else {
        panic!("expected the host-level gate to stop the loop, got {terminal:?}");
    };
    assert!(
        !msg.starts_with("All candidates failed"),
        "a host-level failure is reported on its own, not summarized: {msg}"
    );
    assert!(msg.contains("lacks CAP_NET_ADMIN"), "{msg}");
    assert!(
        granted,
        "grant-fixable failure must emit TunGrantRequired first: {msg}"
    );
    // Exactly one start attempt: the config on disk is still candidate
    // 1's; a second attempt would have overwritten it with candidate 2.
    let config = std::fs::read_to_string(stub.paths.generated_dir().join("xray.json"))
        .expect("generated config readable");
    assert!(config.contains("203.0.113.1"), "{config}");
    assert!(
        !config.contains("203.0.113.2"),
        "second candidate must not be attempted: {config}"
    );
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_releases_the_parked_candidate_s_engine_before_the_next_install() {
    // One antidpi stub for both candidates: it logs every invocation into
    // a shared calls file. Candidate 1's backend dies before ready, so its
    // manager parks with a live engine; the parked engine and its rules
    // must be released before candidate 2 installs, or the teardown of the
    // parked manager would delete the replacement's chain contents.
    let host = tempfile::tempdir().unwrap();
    let calls = host.path().join("antidpi-calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
            &antidpi_stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\ncase \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; while :; do sleep 1; done ;; esac\nexit 0\n",
                calls.display()
            ),
        )
        .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let runs = host.path().join("backend-runs");
    let backend = stub(&format!(
        r#"[ "$1" = version ] && {{ echo 'sing-box version 1.13.0'; exit 0; }}
[ "$1" = check ] && exit 0
n=$(cat {runs} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {runs}
[ $n -le 1 ] && exit 1
exec sleep 30"#,
        runs = runs.display()
    ));
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    // Failover reaches Running on the second candidate.
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no state in time")
            .expect("connection task ended without Running");
        if let SessionEvent::State(_, ProcessState::Running, _) = msg {
            break;
        }
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let lines = loop {
        let lines: Vec<String> = std::fs::read_to_string(&calls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        let installs = lines
            .iter()
            .filter(|l| l.starts_with("install-rules"))
            .count();
        let runs_seen = lines.iter().filter(|l| l.starts_with("run ")).count();
        if installs == 2 && runs_seen >= 2 {
            break lines;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "failover never installed/ran twice: {lines:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    let first_remove = lines
        .iter()
        .position(|l| l.starts_with("remove-rules"))
        .expect("the parked candidate's rules must be released");
    let second_install = lines
        .iter()
        .position(|l| l.starts_with("install-rules"))
        .expect("first install present");
    let second_install = lines[second_install + 1..]
        .iter()
        .position(|l| l.starts_with("install-rules"))
        .expect("second install present")
        + second_install
        + 1;
    assert!(
        first_remove < second_install,
        "the parked engine's rules must be removed before the replacement installs: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("run ")).count(),
        2,
        "the parked engine must be terminated before the replacement spawns: {lines:?}"
    );

    let _ = handle.stop(StopReason::UserStop);
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_exit_notifies_once_for_the_live_generation_and_keeps_the_proxy() {
    // The anti-DPI engine prints its ready line and then self-exits. The
    // session must emit one generation-scoped EngineDegraded notice while
    // the proxy stays Running: no Error, no failover, no terminal state.
    let host = tempfile::tempdir().unwrap();
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
            &antidpi_stub,
            "#!/bin/sh\ncase \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exit 1 ;; esac\nexit 0\n",
        )
        .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend = stub(
        r#"[ "$1" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }
[ "$1" = check ] && exit 0
exec sleep 30"#,
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    // The proxy reaches Running before the engine is missed.
    let mut saw_running = false;
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no message in time")
            .expect("connection task ended without the degraded notice");
        match msg {
            SessionEvent::State(generation, state, _) => {
                assert_eq!(generation, GENERATION);
                saw_running |= state == ProcessState::Running;
            }
            SessionEvent::EngineDegraded { generation, detail } => {
                assert_eq!(generation, GENERATION);
                assert!(detail.contains("code=1"), "{detail}");
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_running,
        "the proxy must be Running when the degraded notice arrives"
    );

    // The healthy proxy keeps its state and the notice never repeats.
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(SessionEvent::State(generation, state, _)) => {
                assert_eq!(generation, GENERATION);
                assert!(
                    !matches!(state, ProcessState::Error(_) | ProcessState::Stopped),
                    "a degraded engine must not disturb the proxy state: {state:?}"
                );
            }
            Ok(SessionEvent::EngineDegraded { generation, .. }) => {
                panic!("duplicate degraded notice for generation {generation}");
            }
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("connection task ended while the proxy was still up");
            }
        }
    }

    let _ = handle.stop(StopReason::UserStop);
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn verified_install_refusal_is_terminal_without_removal_or_banner() {
    // install-rules exits 20: a live foreign owner was established under
    // the transaction lock before any mutation. The session must end in a
    // terminal error carrying the refusal, never launch the backend daemon,
    // never remove another controller's rules, and never arm the retry banner.
    let host = tempfile::tempdir().unwrap();
    let calls = host.path().join("calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> {calls}\ncase \"$1\" in \
install-rules) echo 'rules owned by a live process' >&2; exit 20 ;; \
remove-rules) exit 0 ;; esac\nexit 0\n",
            calls = calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend_calls = host.path().join("backend-calls");
    let backend = stub(&format!(
        "echo \"$@\" >> {bc}\n[ \"$1\" = version ] && {{ echo 'sing-box version 1.13.0'; exit 0; }}\n[ \"$1\" = check ] && exit 0\nexec sleep 30",
        bc = backend_calls.display()
    ));
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a terminal state");
        match msg {
            SessionEvent::State(generation, state, _) => {
                assert_eq!(generation, GENERATION);
                let ProcessState::Error(detail) = &state else {
                    panic!("a verified refusal must never reach {state:?}");
                };
                assert!(
                    detail.contains("owned by a live process"),
                    "the refusal detail must reach the terminal state: {detail}"
                );
                break;
            }
            SessionEvent::CleanupPending(generation, _) => {
                panic!("a verified refusal must not arm the retry banner: gen {generation}");
            }
            SessionEvent::CleanupRecoveryRequired(generation, _) => {
                panic!("a verified refusal must not park recovery: gen {generation}");
            }
            _ => {}
        }
    }
    let _ = handle.stop(StopReason::UserStop);
    // The refusal disarms this request's obligation: no removal may touch
    // the foreign owner, and the backend daemon never launched behind the
    // refused install. The pre-spawn config check legitimately runs the
    // backend binary before install-rules (`check -c ...`, and the optional
    // `version` probe), so presence of the call log alone proves nothing:
    // only the `run` subcommand is the daemon launch.
    let calls = std::fs::read_to_string(&calls).unwrap();
    assert!(
        !calls.contains("remove-rules"),
        "a verified refusal must never remove rules: {calls}"
    );
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.starts_with("install-rules"))
            .count(),
        1,
        "exactly one install attempt: {calls}"
    );
    let backend_calls = std::fs::read_to_string(&backend_calls)
        .expect("the pre-spawn config check must probe the backend, which records its argv");
    assert!(
        backend_calls
            .lines()
            .any(|line| line == "check" || line.starts_with("check ")),
        "the fixture must observe the preflight probe that explains the call log: {backend_calls}"
    );
    assert!(
        !backend_calls
            .lines()
            .any(|line| line == "run" || line.starts_with("run ")),
        "the backend daemon must not launch behind a refused rule install: {backend_calls}"
    );
    assert_nothing_after_terminal(&mut rx).await;
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_install_failure_removes_and_stays_an_error() {
    // install-rules exits 1: an interrupted or unknown failure proves
    // nothing. The obligation stays armed, so the failed candidate's
    // cleanup removes what may have been installed under the same session
    // id, and the startup error is preserved: the terminal state is an
    // Error, never a clean Stopped, and no retry banner arms because the
    // cleanup itself succeeded.
    let host = tempfile::tempdir().unwrap();
    let calls = host.path().join("calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> {calls}\ncase \"$1\" in \
install-rules) echo 'install interrupted' >&2; exit 1 ;; \
remove-rules) exit 0 ;; esac\nexit 0\n",
            calls = calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend = stub(
        "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n[ \"$1\" = check ] && exit 0\nexec sleep 30",
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a terminal state");
        match msg {
            SessionEvent::State(generation, state, _) => {
                assert_eq!(generation, GENERATION);
                let ProcessState::Error(detail) = &state else {
                    panic!("an ambiguous install failure must never reach {state:?}");
                };
                assert!(
                    detail.contains("install interrupted"),
                    "the startup failure must be preserved: {detail}"
                );
                break;
            }
            SessionEvent::CleanupPending(generation, _) => {
                panic!("a completed cleanup must not arm the banner: gen {generation}");
            }
            _ => {}
        }
    }
    let _ = handle.stop(StopReason::UserStop);
    // The unknown outcome kept the obligation armed: the cleanup removed
    // once, under the session id the install used.
    let calls = std::fs::read_to_string(&calls).unwrap();
    let install_id = calls
        .lines()
        .find(|line| line.starts_with("install-rules"))
        .and_then(|line| line.split("--session-id ").nth(1))
        .expect("the install attempt is logged with its session id");
    let removals: Vec<&str> = calls
        .lines()
        .filter(|line| line.starts_with("remove-rules"))
        .collect();
    assert_eq!(
        removals.len(),
        1,
        "the armed obligation removes exactly once: {calls}"
    );
    assert_eq!(
        install_id,
        removals[0]
            .split("--session-id ")
            .nth(1)
            .expect("the removal is logged with its session id"),
        "one session id owns the install and its removal"
    );
    assert_nothing_after_terminal(&mut rx).await;
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_error_survives_a_failed_removal_and_its_retry() {
    // install-rules succeeds and the engine dies before its ready sentinel:
    // a startup failure with the removal obligation armed. The first
    // removal fails once, the retry completes it, and the terminal state
    // stays the original startup Error for the same generation — never a
    // clean Stopped, never a recovery park.
    let host = tempfile::tempdir().unwrap();
    let gate = host.path().join("remove-gate");
    let calls = host.path().join("calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> {calls}\ncase \"$1\" in \
install-rules) exit 0 ;; \
run) echo 'v2rs-engine-refused' >&2; exit 7 ;; \
remove-rules) [ -f {gate} ] && exit 0; : > {gate}; echo 'boom' >&2; exit 1 ;; esac\nexit 0\n",
            calls = calls.display(),
            gate = gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend = stub(
        "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n[ \"$1\" = check ] && exit 0\nexec sleep 30",
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    // The parked report comes first and still names the startup failure;
    // the retry through the same handle then completes the cleanup.
    let mut pending = false;
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a terminal state");
        match msg {
            SessionEvent::State(generation, state, _) => {
                assert_eq!(generation, GENERATION);
                let ProcessState::Error(detail) = &state else {
                    panic!("a retried cleanup must never report {state:?}");
                };
                assert!(
                    detail.contains("v2rs-engine-refused"),
                    "the original startup failure must survive the cleanup retry: {detail}"
                );
                break;
            }
            SessionEvent::CleanupPending(generation, message) => {
                assert_eq!(generation, GENERATION);
                assert!(!pending, "one parked report per failed cleanup");
                pending = true;
                assert!(
                    message.contains("v2rs-engine-refused"),
                    "the parked report keeps the startup failure: {message}"
                );
                let _ = handle.stop(StopReason::AppQuit);
            }
            SessionEvent::CleanupRecoveryRequired(generation, _) => {
                panic!("a retryable cleanup must not park recovery: gen {generation}");
            }
            _ => {}
        }
    }
    assert!(
        pending,
        "the failed first removal must park before the retry"
    );
    let calls = std::fs::read_to_string(&calls).unwrap();
    let install_id = calls
        .lines()
        .find(|line| line.starts_with("install-rules"))
        .and_then(|line| line.split("--session-id ").nth(1))
        .expect("the install attempt is logged with its session id");
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.starts_with("install-rules"))
            .count(),
        1,
        "the cleanup retry must not mint a new install: {calls}"
    );
    let removals: Vec<&str> = calls
        .lines()
        .filter(|line| line.starts_with("remove-rules"))
        .collect();
    assert_eq!(
        removals.len(),
        2,
        "the failed attempt and its retry are the only removals: {calls}"
    );
    for removal in removals {
        assert_eq!(
            install_id,
            removal
                .split("--session-id ")
                .nth(1)
                .expect("each removal is logged with its session id"),
            "the retried cleanup owns the same session"
        );
    }
    assert_nothing_after_terminal(&mut rx).await;
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn require_recovery_represents_the_retained_failure_without_retrying() {
    // An unverified engine exit parks the manager with an obligation no
    // retry can discharge: require_recovery reports it on
    // CleanupRecoveryRequired, re-presents the same report on a further
    // Stop without attempting another teardown, and reports the retained
    // failure exactly once as the terminal state when the channel closes.
    let host = tempfile::tempdir().unwrap();
    let calls = host.path().join("helper-calls");
    let helper = host.path().join("v2ray-rs-netctl");
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\n[ \"$1\" = xray-down ] && exit 1\nexit 0\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut failed = ProcessManager::new(
        PathBuf::from("/bin/true"),
        PathBuf::from("/nonexistent/config.json"),
        host.path().join("backend.pid"),
        None,
    )
    .with_tun(Some(crate::TunRuntime {
        backend: BackendType::Xray,
        iface: "lo".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: helper.clone(),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: Some(crate::TunJournal {
            path: host.path().join("tun_session.v1.json"),
            session_id: "recovery-session".into(),
        }),
    }));
    // The parked manager is the real outcome of a failed teardown: its
    // Error text is what the recovery report must carry.
    failed
        .stop()
        .await
        .expect_err("the failing helper must leave the TUN teardown unfinished");

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SessionCmd>();
    let (tx, mut ev_rx) = EventSender::channel();
    let receiver = tokio::spawn(async move {
        let first = tokio::time::timeout(RECV_TIMEOUT, ev_rx.recv())
            .await
            .expect("no recovery report in time")
            .expect("event sender gone");
        match first {
            SessionEvent::CleanupRecoveryRequired(generation, message) => {
                assert_eq!(generation, GENERATION);
                assert!(message.contains("TUN teardown failed"), "{message}");
            }
            other => panic!("recovery must be announced first, got {other:?}"),
        }
        // A further Stop re-presents the same retained failure and never
        // re-enqueues a teardown attempt.
        cmd_tx
            .send(SessionCmd::Stop(StopReason::UserStop))
            .expect("command channel alive");
        let second = tokio::time::timeout(RECV_TIMEOUT, ev_rx.recv())
            .await
            .expect("no re-presented report in time")
            .expect("event sender gone");
        match second {
            SessionEvent::CleanupRecoveryRequired(generation, message) => {
                assert_eq!(generation, GENERATION);
                assert!(message.contains("TUN teardown failed"), "{message}");
            }
            other => panic!("a Stop during recovery re-presents the report, got {other:?}"),
        }
        // Closing the channel is the only way out: the retained failure
        // becomes the terminal state exactly once.
        drop(cmd_tx);
        let (state, _) = next_state(&mut ev_rx).await;
        let ProcessState::Error(message) = &state else {
            panic!("the retained failure must end as Error, got {state:?}");
        };
        assert!(message.contains("TUN teardown failed"), "{message}");
        assert_nothing_after_terminal(&mut ev_rx).await;
        ev_rx
    });
    tokio::time::timeout(
        RECV_TIMEOUT,
        require_recovery(vec![failed], GENERATION, &mut cmd_rx, &tx),
    )
    .await
    .expect("require_recovery never finished");
    drop(tx);
    let _ev_rx = receiver.await.unwrap();
    // The re-presenting Stop must not have run a second teardown.
    let calls = std::fs::read_to_string(&calls).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.starts_with("xray-down"))
            .count(),
        1,
        "recovery must never re-enqueue a removal attempt: {calls}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quit_from_running_parks_and_the_retry_stops_it() {
    // The quit contract through real actions: an AppQuit from Running must
    // report CleanupPending with no terminal state before it, and the
    // retry through the same handle must end in Stopped for the same
    // generation. Exactly two removals run, both for the session id the
    // install used, and no new session identity is minted.
    let host = tempfile::tempdir().unwrap();
    let gate = host.path().join("remove-gate");
    let calls = host.path().join("calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> {calls}\ncase \"$1\" in \
run) echo \"v2ray-rs-antidpi: ready queue=200\"; while :; do sleep 1; done ;; \
remove-rules) [ -f {gate} ] && exit 0; : > {gate}; echo 'boom' >&2; exit 1 ;; esac\nexit 0\n",
            calls = calls.display(),
            gate = gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend = stub(
        "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n[ \"$1\" = check ] && exit 0\nexec sleep 30",
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no state in time")
            .expect("connection task ended without Running");
        if let SessionEvent::State(_, ProcessState::Running, _) = msg {
            break;
        }
    }

    let _ = handle.stop(StopReason::AppQuit);
    // The quit parks: the cleanup report arrives before any terminal state.
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a cleanup report");
        match msg {
            SessionEvent::State(_, ProcessState::Stopped | ProcessState::Error(_), _) => {
                panic!("terminal state before the cleanup retry: {msg:?}");
            }
            SessionEvent::CleanupPending(generation, message) => {
                assert_eq!(generation, GENERATION);
                assert!(message.contains("cleanup failed"), "{message}");
                break;
            }
            _ => {}
        }
    }

    // The retry through the same handle reaches the same generation's
    // Stopped.
    let _ = handle.stop(StopReason::AppQuit);
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without the retried stop");
        if let SessionEvent::State(generation, ProcessState::Stopped, None) = msg {
            assert_eq!(generation, GENERATION);
            break;
        }
    }

    let calls = std::fs::read_to_string(&calls).unwrap();
    let install_id = calls
        .lines()
        .find(|line| line.starts_with("install-rules"))
        .and_then(|line| line.split("--session-id ").nth(1))
        .expect("the install is logged with its session id");
    let removals: Vec<&str> = calls
        .lines()
        .filter(|line| line.starts_with("remove-rules"))
        .collect();
    assert_eq!(
        removals.len(),
        2,
        "the failed attempt and the retry are the only removals: {calls}"
    );
    for removal in removals {
        assert_eq!(
            install_id,
            removal
                .split("--session-id ")
                .nth(1)
                .expect("each removal is logged with its session id"),
            "the parked session owns every removal"
        );
    }
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn quit_during_a_parked_disconnect_retries_the_same_session() {
    // The user quits while a Disconnect's cleanup is parked: the AppQuit
    // through the same handle retries the parked removal and reaches the
    // disconnect's Stopped — no new generation, no second install.
    let host = tempfile::tempdir().unwrap();
    let gate = host.path().join("remove-gate");
    let calls = host.path().join("calls");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> {calls}\ncase \"$1\" in \
run) echo \"v2ray-rs-antidpi: ready queue=200\"; while :; do sleep 1; done ;; \
remove-rules) [ -f {gate} ] && exit 0; : > {gate}; echo 'boom' >&2; exit 1 ;; esac\nexit 0\n",
            calls = calls.display(),
            gate = gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let backend = stub(
        "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n[ \"$1\" = check ] && exit 0\nexec sleep 30",
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &backend,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no state in time")
            .expect("connection task ended without Running");
        if let SessionEvent::State(_, ProcessState::Running, _) = msg {
            break;
        }
    }

    // The Disconnect parks.
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a cleanup report");
        match msg {
            SessionEvent::State(_, ProcessState::Stopped | ProcessState::Error(_), _) => {
                panic!("terminal state before the cleanup retry: {msg:?}");
            }
            SessionEvent::CleanupPending(generation, _) => {
                assert_eq!(generation, GENERATION);
                break;
            }
            _ => {}
        }
    }

    // The quit during the parked disconnect retries it.
    let _ = handle.stop(StopReason::AppQuit);
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without the retried stop");
        if let SessionEvent::State(generation, ProcessState::Stopped, None) = msg {
            assert_eq!(generation, GENERATION);
            break;
        }
    }

    let calls = std::fs::read_to_string(&calls).unwrap();
    let install_id = calls
        .lines()
        .find(|line| line.starts_with("install-rules"))
        .and_then(|line| line.split("--session-id ").nth(1))
        .expect("the install is logged with its session id");
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.starts_with("install-rules"))
            .count(),
        1,
        "the retry must not mint a new session: {calls}"
    );
    let removals: Vec<&str> = calls
        .lines()
        .filter(|line| line.starts_with("remove-rules"))
        .collect();
    assert_eq!(removals.len(), 2, "failed attempt and retry: {calls}");
    for removal in removals {
        assert_eq!(
            install_id,
            removal
                .split("--session-id ")
                .nth(1)
                .expect("each removal is logged with its session id"),
            "the parked session owns every removal"
        );
    }
    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_disconnect_cleanup_parks_until_retry() {
    // remove-rules fails exactly once (the gate file is created by the
    // failing call): the first disconnect must park the manager with a
    // CleanupPending report instead of a terminal Stopped, and a retry
    // through the same handle must clean the same session's rules.
    let host = tempfile::tempdir().unwrap();
    let gate = host.path().join("remove-gate");
    let antidpi_stub = host.path().join("v2ray-rs-antidpi");
    std::fs::write(
        &antidpi_stub,
        format!(
            "#!/bin/sh\ncase \"$1\" in \
run) echo \"v2ray-rs-antidpi: ready queue=200\"; while :; do sleep 1; done ;; \
remove-rules) [ -f {gate} ] && exit 0; : > {gate}; echo 'boom' >&2; exit 1 ;; esac\nexit 0\n",
            gate = gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&antidpi_stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let stub = stub(
        "[ \"$1\" = version ] && echo 'sing-box version 1.13.0' && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30",
    );
    let (listener, mut settings) = ready_singbox_settings();
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;

    let stub_path = antidpi_stub.clone();
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: stub_path.clone(),
                helper: stub_path.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
            .with_antidpi(Some(crate::AntiDpiRuntime {
                helper_path: stub_path.clone(),
                queue: 200,
                strategies: vec![],
                hostlist: None,
            }))
        },
    );

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no state in time")
            .expect("connection task ended without Running");
        if let SessionEvent::State(_, ProcessState::Running, _) = msg {
            break;
        }
    }

    let _ = handle.stop(StopReason::UserStop);

    // No terminal state before the retry: the cleanup report arrives
    // first and names what remains installed.
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without a cleanup report");
        match msg {
            SessionEvent::State(_, ProcessState::Stopped | ProcessState::Error(_), _) => {
                panic!("terminal state before the cleanup retry: {msg:?}");
            }
            SessionEvent::CleanupPending(generation, message) => {
                assert_eq!(generation, GENERATION);
                assert!(message.contains("cleanup failed"), "{message}");
                break;
            }
            _ => {}
        }
    }

    // Retry through the same handle: no new session identity is minted,
    // and the parked manager's rules are removed by the retry.
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no event in time")
            .expect("connection task ended without the retried stop");
        if let SessionEvent::State(generation, ProcessState::Stopped, None) = msg {
            assert_eq!(generation, GENERATION);
            break;
        }
    }

    drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn log_lines_carry_connection_generation() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.2; done"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no log line in time")
            .expect("connection task ended before a log line");
        if let SessionEvent::LogLines(generation, lines) = msg {
            assert_eq!(generation, GENERATION);
            assert!(
                lines.iter().any(|line| line == "v2rs-log-line"),
                "{lines:?}"
            );
            break;
        }
    }

    let _ = handle.stop(StopReason::UserStop);
}

#[tokio::test(flavor = "multi_thread")]
async fn marker_written_from_runtime_before_start() {
    // A failed teardown now parks the manager as CleanupPending instead of
    // reporting a terminal state, so the route helper pinned here must
    // answer xray-down successfully: the start fails at the capability
    // gate (the pinned getcap probe reports no capabilities) with no
    // kernel state to clean, and the host error is reported as before.
    let host = tempfile::tempdir().unwrap();
    let gate_helper = host.path().join("v2ray-rs-netctl");
    std::fs::write(&gate_helper, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&gate_helper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "v2rstest0".into();
    let gate = gate_helper.clone();
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| {
            mgr.with_host_probe(crate::HostProbe {
                getcap: gate.clone(),
                helper: gate.clone(),
                helper_access: Some(crate::HelperAccess::Ready),
            })
        },
    );

    let (state, _) = next_state(&mut rx).await;
    let ProcessState::Error(msg) = state else {
        panic!("expected the capability gate to fail the start, got {state:?}");
    };
    // Root skips the capability gate, so there the route helper preflight
    // is the gate that stops the start when the helper is not installed.
    assert!(
        msg.contains("CAP_NET_ADMIN")
            || msg.contains("TUN capabilities")
            || msg.contains("ignores file capabilities")
            || msg.contains("route helper v2ray-rs-netctl not found"),
        "start should fail at the capability gate: {msg}"
    );
    let session = load_tun_session(&stub.paths).unwrap();
    assert_eq!(session.backend, BackendType::Xray);
    assert_eq!(session.iface, "v2rstest0");
    assert!(session.ownership.is_some());
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn marker_write_failure_blocks_start() {
    let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "v2rstest0".into();
    // Force the marker write to fail: a directory where the session file
    // must be written.
    let marker_path = stub.paths.tun_session_path();
    std::fs::create_dir_all(&marker_path).unwrap();

    let (_handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    let (state, _) = next_state(&mut rx).await;
    let ProcessState::Error(msg) = state else {
        panic!("expected the marker failure to stop the start, got {state:?}");
    };
    assert!(
        msg.starts_with("Failed to record TUN session: "),
        "unexpected error text: {msg}"
    );
    assert_nothing_after_terminal(&mut rx).await;
    // Terminal before any backend spawn: the log file itself is opened
    // once before the candidate loop, so only its contents are
    // guaranteed clean; no session record.
    if let Ok(contents) = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log")) {
        assert!(!contents.contains(" session "), "{contents}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn no_marker_when_launched_without_tun() {
    let stub = stub("exit 1");
    let (_handle, mut rx) = connect(&stub, singbox_settings(), vec![candidate("203.0.113.1")]);

    loop {
        match next_state(&mut rx).await {
            (ProcessState::Error(_), _) => break,
            (state, _) => assert!(relays(&state), "reported {state:?}"),
        }
    }
    assert_eq!(load_tun_session(&stub.paths), None);
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn live_connect_writes_backend_diagnostics() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Running) {
            break;
        }
        assert!(relays(&state), "reported {state:?}");
    }

    let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    let session_at = contents.find(" session ").expect("session record");
    assert_eq!(contents.matches(" session ").count(), 1, "{contents}");
    assert!(
        contents.contains("backend=sing-box version=1.13.0 node=203.0.113.1 tun=off"),
        "{contents}"
    );
    let session_line = contents[session_at..].lines().next().unwrap_or_default();
    assert!(
        session_line.contains("tun=off utc_offset="),
        "{session_line}"
    );
    assert!(
        session_line
            .ends_with("hijack=off capture_dns=false strict=false nodes_pinned=true profile=app"),
        "{session_line}"
    );

    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }

    let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    let exit_at = contents.find(" exit ").expect("exit record");
    assert_eq!(contents.matches(" exit ").count(), 1, "{contents}");
    assert!(contents[exit_at..].contains("requested=true"), "{contents}");
    assert!(session_at < exit_at, "{contents}");
}

#[tokio::test(flavor = "multi_thread")]
async fn xray_tun_session_record_carries_dns_decisions() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; exec sleep 30"#,
    );
    let host = tempfile::tempdir().unwrap();
    let getcap = host.path().join("getcap");
    std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
    std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = crate::HostProbe {
        getcap,
        helper: executable(host.path(), "v2ray-rs-netctl"),
        helper_access: Some(crate::HelperAccess::Ready),
    };
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "v2rstest3".into();
    settings.tun.strict_route = false;
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let (terminal, _) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(contents.matches(" session ").count(), 1, "{contents}");
    assert!(contents.contains("tun=on utc_offset="), "{contents}");
    assert!(
        contents
            .contains("hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app"),
        "{contents}"
    );
}

/// Fake `netctl` that records every subcommand, so tests can prove no
/// teardown ran against the live session.
fn recording_helper(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let calls = dir.join("helper-calls");
    let helper = dir.join("v2ray-rs-netctl");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$1\" >> {}\nexit 0\n", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    (helper, calls)
}

/// Wiring shared by the failover tests: capability-verdict probe plus a
/// recording helper, TUN on a device that exists without privileges, and
/// a SOCKS listener so the replacement candidate passes the ready probe.
fn tun_failover_probe() -> (tempfile::TempDir, crate::HostProbe, PathBuf) {
    let host = tempfile::tempdir().unwrap();
    let getcap = host.path().join("getcap");
    std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
    std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (helper, calls) = recording_helper(host.path());
    let probe = crate::HostProbe {
        getcap,
        helper,
        helper_access: Some(crate::HelperAccess::Ready),
    };
    (host, probe, calls)
}

fn ready_port(settings: &mut AppSettings) -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    settings.socks_port = listener.local_addr().unwrap().port();
    listener
}

async fn wait_for_running(rx: &mut mpsc::UnboundedReceiver<SessionEvent>) {
    loop {
        let (state, _) = next_state(rx).await;
        if matches!(state, ProcessState::Running) {
            break;
        }
        assert!(relays(&state), "failover reported {state:?}");
    }
}

async fn stop_and_wait_stopped(
    handle: &SessionHandle,
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
) {
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_candidates_stay_parked_until_stop_releases_them() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && grep -q 203.0.113.2 "$3" && exit 1; [ "$2" = -test ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
    );
    let (_host, probe, calls) = tun_failover_probe();
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    // `lo` exists on every Linux host, so the fake xray-up runs against a
    // live device without kernel privileges.
    settings.tun.interface_name = "lo".into();
    settings.tun.strict_route = false;
    let _listener = ready_port(&mut settings);
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![
            candidate("203.0.113.1"),
            candidate("203.0.113.2"),
            candidate("203.0.113.3"),
        ],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    // Candidate 1 dies after `reserve` and `xray-up`, candidate 2 fails
    // the config check before launch, candidate 3 runs and waits forever
    // for a ready probe that never succeeds. Two `xray-up` lines mean
    // candidate 3 passed its route setup.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(&calls).unwrap_or_default();
        if text.matches("xray-up").count() == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the third candidate never reached xray-up: {}",
            std::fs::read_to_string(&calls).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    stop_and_wait_stopped(&handle, &mut rx).await;
    assert_nothing_after_terminal(&mut rx).await;

    let text = std::fs::read_to_string(&calls).unwrap();
    assert_eq!(text.matches("reserve").count(), 2, "{text}");
    assert_eq!(text.matches("xray-up").count(), 2, "{text}");
    assert_eq!(text.matches("xray-down").count(), 2, "{text}");
    let second_up = text
        .match_indices("xray-up")
        .nth(1)
        .map(|(i, _)| i)
        .expect("two xray-up lines");
    assert!(
        !text[..second_up].contains("xray-down"),
        "a parked candidate was released before the stop: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_tun_session_failure_releases_the_parked_candidate() {
    let stub = stub("");
    let legacy = stub.paths.legacy_tun_session_path();
    // Candidate 1 launches, leaves the legacy journal behind, and dies.
    // Candidate 2's prepared write then refuses to overwrite it.
    std::fs::write(
            &stub.binary,
            format!(
                "#!/bin/sh\n[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0; [ \"$2\" = -test ] && exit 0; : > {}; exit 1",
                legacy.display()
            ),
        )
        .unwrap();
    let (_host, probe, calls) = tun_failover_probe();
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "lo".into();
    settings.tun.strict_route = false;
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let mut error = None;
    while error.is_none() {
        let (state, _) = next_state(&mut rx).await;
        if let ProcessState::Error(msg) = state {
            error = Some(msg);
        } else {
            assert!(relays(&state), "reported {state:?}");
        }
    }
    let msg = error.unwrap();
    assert!(msg.contains("Failed to record TUN session"), "{msg}");
    assert!(msg.contains("--force-legacy"), "{msg}");
    let text = std::fs::read_to_string(&calls).unwrap_or_default();
    assert_eq!(text.matches("xray-down").count(), 1, "{text}");
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_during_preflight_retires_the_prepared_tun_journal() {
    let (host, probe, _probe_calls) = tun_failover_probe();
    let calls = host.path().join("helper-calls");
    let marker = host.path().join("version-blocked");
    std::fs::write(
        &probe.helper,
        format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&probe.helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stub = stub("");
    std::fs::write(
        &stub.binary,
        format!(
            "#!/bin/sh\n[ \"$1\" = version ] && {{ : > {}; sleep 30; }}\nexec sleep 30\n",
            marker.display()
        ),
    )
    .unwrap();
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "lo".into();
    settings.tun.strict_route = false;
    let _listener = ready_port(&mut settings);
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the version call never reached the stub"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let session = load_tun_session(&stub.paths).expect("prepared journal present");
    let session_id = session
        .ownership
        .expect("prepared ownership")
        .session_id
        .to_string();

    stop_and_wait_stopped(&handle, &mut rx).await;
    assert_nothing_after_terminal(&mut rx).await;

    let text = std::fs::read_to_string(&calls).unwrap();
    let downs: Vec<&str> = text.lines().filter(|l| l.contains("xray-down")).collect();
    assert_eq!(downs.len(), 1, "{text}");
    assert!(
        downs[0].contains(&format!(
            "--journal {}",
            stub.paths.tun_session_path().display()
        )),
        "{text}"
    );
    assert!(
        downs[0].contains(&format!("--session-id {session_id}")),
        "{text}"
    );
    assert!(!text.contains("reserve"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_cleanup_blocks_stopped_when_the_active_manager_stopped_cleanly() {
    let host = tempfile::tempdir().unwrap();
    let calls = host.path().join("helper-calls");
    let gate = host.path().join("gate");
    let helper = host.path().join("v2ray-rs-netctl");
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\n[ \"$1\" = xray-down ] && [ -f {} ] && exit 1\nexit 0\n",
            calls.display(),
            gate.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let orphan = || {
        ProcessManager::new(
            PathBuf::from("/bin/true"),
            PathBuf::from("/nonexistent/config.json"),
            host.path().join("backend.pid"),
            None,
        )
        .with_tun(Some(crate::TunRuntime {
            backend: BackendType::Xray,
            iface: "lo".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: helper.clone(),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
            exclude_routes: Vec::new(),
            device_ifindex: None,
            journal: Some(crate::TunJournal {
                path: host.path().join("tun_session.v1.json"),
                session_id: "orphan-session".into(),
            }),
        }))
    };

    // Control: the helper succeeds, so nothing is left to clean and the
    // terminal state is reported directly.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SessionCmd>();
    let (tx, mut ev_rx) = EventSender::channel();
    let mut no_orphans: Vec<ProcessManager> = Vec::new();
    finish_or_retry_cleanup(
        Vec::new(),
        &mut no_orphans,
        GENERATION,
        &mut cmd_rx,
        &tx,
        ProcessState::Stopped,
    )
    .await;
    let _ = cmd_tx;
    let event = tokio::time::timeout(RECV_TIMEOUT, ev_rx.recv())
        .await
        .expect("no control event in time")
        .expect("event sender gone");
    assert!(matches!(
        event,
        SessionEvent::State(GENERATION, ProcessState::Stopped, None)
    ));
    assert!(ev_rx.try_recv().is_err(), "unexpected extra control event");

    // Retry: the helper's xray-down fails while the gate exists, so the
    // orphan parks as CleanupPending; clearing the gate and sending a
    // Stop lets the retry reach Stopped.
    std::fs::write(&gate, b"").unwrap();
    // `retry_cleanup` reports the manager's current state before it
    // retries, so the orphan must already carry the failed teardown.
    let mut failed = orphan();
    failed
        .stop()
        .await
        .expect_err("the gate must fail the teardown");
    let mut orphaned = vec![failed];
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SessionCmd>();
    let (tx, mut ev_rx) = EventSender::channel();
    let receiver = tokio::spawn(async move {
        loop {
            match ev_rx.recv().await {
                Some(SessionEvent::CleanupPending(_, message)) => {
                    assert!(message.contains("TUN teardown failed"), "{message}");
                    break;
                }
                Some(SessionEvent::State(_, state, _)) => {
                    panic!("reported {state:?} before the retry")
                }
                Some(_) => {}
                None => panic!("event channel closed without a CleanupPending"),
            }
        }
        std::fs::remove_file(&gate).unwrap();
        cmd_tx
            .send(SessionCmd::Stop(StopReason::UserStop))
            .expect("command channel alive");
        ev_rx
    });
    tokio::time::timeout(
        RECV_TIMEOUT,
        finish_or_retry_cleanup(
            Vec::new(),
            &mut orphaned,
            GENERATION,
            &mut cmd_rx,
            &tx,
            ProcessState::Stopped,
        ),
    )
    .await
    .expect("the orphaned cleanup never finished");
    drop(tx);
    let mut ev_rx = receiver.await.unwrap();
    let (state, _) = next_state(&mut ev_rx).await;
    assert_eq!(state, ProcessState::Stopped);
    assert_nothing_after_terminal(&mut ev_rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn singbox_failover_after_a_pre_device_failure_disconnects_cleanly() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && grep -q 203.0.113.1 "$3" && exit 1; [ "$1" = check ] && exit 0; exec sleep 30"#,
    );
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::SingBox;
    settings.tun.interface_name = "v2rstest6".into();
    let _listener = ready_port(&mut settings);
    let (_host, probe, _calls) = tun_failover_probe();
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    wait_for_running(&mut rx).await;
    let _ = handle.stop(StopReason::UserStop);

    let mut states = Vec::new();
    loop {
        match tokio::time::timeout(RECV_TIMEOUT, rx.recv()).await {
            Ok(None) => break,
            Ok(Some(SessionEvent::CleanupPending(_, message))) => {
                panic!("CleanupPending after a clean disconnect: {message}")
            }
            Ok(Some(SessionEvent::State(_, state, _))) => states.push(state),
            Ok(Some(_)) => {}
            Err(_) => panic!("the session task hung after the stop"),
        }
    }
    assert_eq!(states.last(), Some(&ProcessState::Stopped), "{states:?}");
    assert!(
        states[..states.len() - 1]
            .iter()
            .all(|state| *state == ProcessState::Stopping),
        "only the live Stopping relay may precede the terminal state: {states:?}"
    );
    let session = load_tun_session(&stub.paths).expect("journal present");
    assert_eq!(
        session.ownership.expect("ownership").phase,
        v2ray_rs_core::persistence::TunPhase::Complete
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_keeps_the_adopted_tun_session_after_a_post_device_failure() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
    );
    let (_host, probe, calls) = tun_failover_probe();
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    // `lo` exists on every Linux host, so the fake xray-up runs against a
    // live device without kernel privileges.
    settings.tun.interface_name = "lo".into();
    settings.tun.strict_route = false;
    let _listener = ready_port(&mut settings);
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    wait_for_running(&mut rx).await;

    let contents = std::fs::read_to_string(&calls).unwrap_or_default();
    // Positive control: both candidates drove the fake helper, so the
    // absence below means the mechanism, not a broken fixture.
    assert_eq!(contents.matches("reserve").count(), 2, "{contents}");
    assert_eq!(contents.matches("xray-up").count(), 2, "{contents}");
    // Candidate 2 adopted candidate 1's journal, so the parked manager
    // must drop without touching the live session. Pre-change this
    // failed: the parked shutdown ran xray-down against the replacement.
    assert!(!contents.contains("xray-down"), "{contents}");
    let session = load_tun_session(&stub.paths).expect("journal present");
    assert_eq!(session.backend, BackendType::Xray);
    assert_eq!(session.iface, "lo");
    assert_ne!(
        session.ownership.expect("ownership").phase,
        v2ray_rs_core::persistence::TunPhase::Complete
    );

    // Only the replacement's own stop may tear the session down.
    stop_and_wait_stopped(&handle, &mut rx).await;
    let contents = std::fs::read_to_string(&calls).unwrap_or_default();
    assert_eq!(contents.matches("xray-down").count(), 1, "{contents}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_keeps_the_adopted_tun_session_after_a_pre_device_failure() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && grep -q 203.0.113.1 "$3" && exit 1; [ "$2" = -test ] && exit 0; exec sleep 30"#,
    );
    let (_host, probe, calls) = tun_failover_probe();
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "lo".into();
    settings.tun.strict_route = false;
    let _listener = ready_port(&mut settings);
    let (handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    wait_for_running(&mut rx).await;

    let contents = std::fs::read_to_string(&calls).unwrap_or_default();
    // Candidate 1 was rejected in the config check, before the helper ran:
    // only candidate 2's launch appears.
    assert_eq!(contents.matches("reserve").count(), 1, "{contents}");
    assert_eq!(contents.matches("xray-up").count(), 1, "{contents}");
    assert!(!contents.contains("xray-down"), "{contents}");
    let session = load_tun_session(&stub.paths).expect("journal present");
    assert_eq!(session.iface, "lo");
    assert_ne!(
        session.ownership.expect("ownership").phase,
        v2ray_rs_core::persistence::TunPhase::Complete
    );

    stop_and_wait_stopped(&handle, &mut rx).await;
    let contents = std::fs::read_to_string(&calls).unwrap_or_default();
    assert_eq!(contents.matches("xray-down").count(), 1, "{contents}");
}

#[tokio::test(flavor = "multi_thread")]
async fn singbox_failover_keeps_the_replacement_journal_uncompleted() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
    );
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::SingBox;
    settings.tun.interface_name = "v2rstest6".into();
    let _listener = ready_port(&mut settings);
    let (_host, probe, _calls) = tun_failover_probe();
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    wait_for_running(&mut rx).await;

    // Candidate 2 adopted candidate 1's journal (phase Applying), so the
    // parked manager must drop without running the sing-box absence check
    // or completing the live session's journal. Pre-change this failed:
    // the parked shutdown verified the absent interface and completed the
    // replacement's journal.
    let session = load_tun_session(&stub.paths).expect("journal present");
    assert_eq!(session.backend, BackendType::SingBox);
    assert_eq!(session.iface, "v2rstest6");
    assert_ne!(
        session.ownership.expect("ownership").phase,
        v2ray_rs_core::persistence::TunPhase::Complete
    );
}

#[test]
fn fallback_dns_notice_selects_the_xray_disabled_text_verbatim() {
    let summary = dns_summary(BackendType::Xray, false);
    assert_eq!(
        fallback_dns_notice(&summary, false, true, BackendType::Xray),
        Some(FALLBACK_DNS_DISABLED_NOTICE_XRAY)
    );
}

#[test]
fn fallback_dns_notice_selects_the_singbox_disabled_text_verbatim() {
    let summary = dns_summary(BackendType::SingBox, false);
    assert_eq!(
        fallback_dns_notice(&summary, false, true, BackendType::SingBox),
        Some(FALLBACK_DNS_DISABLED_NOTICE_SINGBOX)
    );
}

#[test]
fn fallback_dns_notice_selects_the_scoped_text_verbatim() {
    let summary = scoped_dns_summary();
    assert_eq!(
        fallback_dns_notice(&summary, true, true, BackendType::Xray),
        Some(FALLBACK_DNS_SCOPED_NOTICE)
    );
}

#[test]
fn fallback_dns_notice_stays_silent_without_tun_or_fallback() {
    let summary = dns_summary(BackendType::Xray, false);
    assert_eq!(
        fallback_dns_notice(&summary, false, false, BackendType::Xray),
        None
    );
    let unscoped = dns_summary(BackendType::Xray, true);
    assert_eq!(
        fallback_dns_notice(&unscoped, true, true, BackendType::Xray),
        None
    );
    assert_eq!(
        fallback_dns_notice(&scoped_dns_summary(), true, false, BackendType::Xray),
        None
    );
    assert_eq!(
        fallback_dns_notice(
            &dns_summary(BackendType::V2ray, false),
            false,
            true,
            BackendType::V2ray
        ),
        None
    );
}

fn dns_summary(backend: BackendType, dns_enabled: bool) -> EffectiveDns {
    let mut settings = AppSettings::default();
    settings.tun.enabled = true;
    settings.dns.enabled = dns_enabled;
    effective_dns(backend, &settings, &[], &["203.0.113.10"])
}

fn scoped_dns_summary() -> EffectiveDns {
    let mut settings = AppSettings::default();
    settings.tun.enabled = true;
    settings.dns.enabled = true;
    settings.dns.use_custom_rules = true;
    settings.dns.servers = vec![DnsServerConfig {
        tag: "domestic".into(),
        protocol: DnsProtocol::Udp,
        address: "77.88.8.8".into(),
        port: None,
        detour: None,
    }];
    settings.dns.rules = vec![DnsRule {
        match_condition: DnsRuleMatch::DomainSuffix {
            suffix: "ru".into(),
        },
        server_tag: "domestic".into(),
    }];
    effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"])
}

#[tokio::test(flavor = "multi_thread")]
async fn fallback_notice_once_across_two_failed_candidates() {
    let (stub, mut settings, _host, probe) = xray_tun_stub_with_probe("v2rsdnsn1");
    settings.dns.enabled = false;
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let (terminal, lines, toasts) = drain_within(&mut rx, Duration::from_secs(60)).await;
    assert_error_terminal(terminal);

    let notices: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains(FALLBACK_DNS_DISABLED_NOTICE_XRAY))
        .collect();
    assert_eq!(notices.len(), 1, "{lines:#?}");
    assert_eq!(notices[0], &FALLBACK_DNS_DISABLED_NOTICE_XRAY);
    assert!(
        toasts.iter().all(|t| !t.contains("DNS is disabled")),
        "{toasts:#?}"
    );

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(
        log.matches(FALLBACK_DNS_DISABLED_NOTICE_XRAY).count(),
        1,
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_notice_with_dns_enabled_unscoped_server() {
    let (stub, mut settings, _host, probe) = xray_tun_stub_with_probe("v2rsdnsn2");
    settings.dns.enabled = true;
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let (terminal, lines, _) = drain_within(&mut rx, Duration::from_secs(60)).await;
    assert_error_terminal(terminal);

    assert!(
        lines.iter().all(|l| !l.contains("DNS is disabled")
            && !l.contains("fallback 1.1.1.1")
            && !l.contains(FALLBACK_DNS_SCOPED_NOTICE)),
        "{lines:#?}"
    );
    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert!(!log.contains("DNS is disabled"), "{log}");
    assert!(!log.contains(FALLBACK_DNS_SCOPED_NOTICE), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn dns_lines_follow_session_in_backend_log() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; exec sleep 30"#,
    );
    let host = tempfile::tempdir().unwrap();
    let getcap = host.path().join("getcap");
    std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
    std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = crate::HostProbe {
        getcap,
        helper: executable(host.path(), "v2ray-rs-netctl"),
        helper_access: Some(crate::HelperAccess::Ready),
    };
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.tun.interface_name = "v2rsdnsl3".into();
    settings.tun.strict_route = false;
    settings.dns.enabled = false;
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("203.0.113.1")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let (terminal, _) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    let lines: Vec<&str> = log.lines().collect();
    let session_at = lines
        .iter()
        .position(|l| l.contains(" session "))
        .expect("session record");
    assert!(
        lines[session_at + 1].contains(" dns server=https://1.1.1.1/dns-query"),
        "{}",
        lines[session_at + 1]
    );
    assert!(
        lines[session_at + 2].contains(" dns server=https://8.8.8.8/dns-query"),
        "{}",
        lines[session_at + 2]
    );
    assert_eq!(
        lines.iter().filter(|l| l.contains(" dns server=")).count(),
        2,
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dns_lines_carry_source_profile_for_imported_profile_candidate() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
    sub.use_imported_profile = true;
    sub.imported_profile = Some(ImportedProfile {
        rules: Vec::new(),
        dns: Some(DnsConfig {
            enabled: true,
            ..DnsConfig::default()
        }),
        skipped: Vec::new(),
        imported_at: chrono::Utc::now(),
    });
    let candidate = ConnectionCandidate {
        node_ref: ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: uuid::Uuid::new_v4(),
        },
        ..candidate("203.0.113.9")
    };
    let mut req = request(&stub, settings, vec![candidate]);
    req.subscriptions = vec![sub];
    let (tx, mut rx) = EventSender::channel();
    let handle = spawn_with(req, tx, |mgr| mgr, no_subnets);

    loop {
        let (state, _) = next_state(&mut rx).await;
        assert!(relays(&state), "reported {state:?}");
        if matches!(state, ProcessState::Running) {
            break;
        }
    }
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(log.matches(" source=profile ").count(), 2, "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_is_traceable() {
    let capture = install_test_capture();
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 198.51.100.11 "$3" && { echo "FATAL start service: trace refused" >&2; exit 1; }; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(
        &stub,
        settings,
        vec![
            candidate("198.51.100.11"),
            candidate("198.51.100.12"),
            candidate("198.51.100.13"),
        ],
    );

    loop {
        let (state, _) = next_state(&mut rx).await;
        assert!(relays(&state), "failover reported {state:?}");
        if matches!(state, ProcessState::Running) {
            break;
        }
    }
    let _ = handle.stop(StopReason::UserStop);

    let lines = capture.lines_containing("198.51.100.1");
    let at = |prefix: &str, address: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(prefix) && line.contains(address))
            .unwrap_or_else(|| panic!("no {prefix:?} line for {address}: {lines:#?}"))
    };
    let started = at("candidate start 1/3 ", "198.51.100.11");
    let failed = at("candidate failed 1/3 ", "198.51.100.11");
    let next = at("candidate start 2/3 ", "198.51.100.12");
    assert!(started < failed && failed < next, "{lines:#?}");
    assert!(lines[failed].contains("reason="), "{}", lines[failed]);
    assert!(lines[failed].contains("trace refused"), "{}", lines[failed]);
    assert!(
        !lines.iter().any(|line| line.contains("198.51.100.13")),
        "{lines:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tun_off_connection_pins_no_hostname() {
    use std::net::ToSocketAddrs;
    assert!(
        ("localhost", 0).to_socket_addrs().unwrap().next().is_some(),
        "localhost must resolve for this test to mean anything"
    );
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exit 1"#,
    );
    let (_handle, mut rx) = connect(
        &stub,
        singbox_settings(),
        vec![candidate("localhost"), candidate("localhost")],
    );

    let (terminal, _) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    let sessions: Vec<&str> = log.lines().filter(|l| l.contains(" session ")).collect();
    assert_eq!(sessions.len(), 2, "{log}");
    for session in sessions {
        assert!(session.contains("nodes_pinned=false"), "{session}");
    }

    let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
        .expect("generated config readable");
    let config: serde_json::Value = serde_json::from_str(&config).unwrap();
    let servers = config["dns"]["servers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !servers.iter().any(|s| s["type"] == "hosts"),
        "{}",
        config["dns"]
    );
    assert!(
        !config["dns"].to_string().contains("localhost"),
        "{}",
        config["dns"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failover_resolves_every_candidate_host_before_first_start() {
    let capture = install_test_capture();
    let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
    let mut settings = xray_tun_settings();
    settings.tun.interface_name = "v2rstest4".into();
    let req = request(
        &stub,
        settings,
        vec![
            candidate("failover-a.invalid"),
            candidate("failover-b.invalid"),
        ],
    );
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, _) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let lines = capture.lines_containing("failover-");
    let at = |needle: &str| {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no {needle:?} line: {lines:#?}"))
    };
    let first_start = at("candidate start 1/2");
    assert!(
        at("cannot pin failover-a.invalid") < first_start,
        "{lines:#?}"
    );
    assert!(
        at("cannot pin failover-b.invalid") < first_start,
        "{lines:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn non_strict_tun_resolves_each_candidate_lazily() {
    let capture = install_test_capture();
    let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
    let mut settings = xray_tun_settings();
    settings.tun.interface_name = "v2rstest5".into();
    settings.tun.strict_route = false;
    let req = request(
        &stub,
        settings,
        // The first candidate fails config generation, so no tunnel is
        // parked; the second's lookup must happen at its own turn.
        vec![candidate("[::1]"), candidate("lazy-b.invalid")],
    );
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(req, tx, capless_probe, no_subnets);

    let (terminal, _) = drain(&mut rx).await;
    assert_error_terminal(terminal);

    let lines = capture.all_lines();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no {needle:?} line: {lines:#?}"))
    };
    assert!(
        at("candidate start 2/2 label=lazy-b.invalid") < at("cannot pin lazy-b.invalid"),
        "candidate 2 must be resolved at its own turn: {lines:#?}"
    );
}

#[test]
fn probe_skipped_when_parked_manager_holds_tun() {
    let manager = || {
        ProcessManager::new(
            PathBuf::from("/bin/true"),
            PathBuf::from("/nonexistent/config.json"),
            PathBuf::from("/nonexistent/pid"),
            None,
        )
    };
    let tunneled = manager().with_tun(Some(TunRuntime {
        backend: BackendType::Xray,
        iface: "v2rsprobe0".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: PathBuf::from("/bin/true"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    }));

    assert!(probe_allowed(None));
    assert!(probe_allowed(Some(&manager())));
    assert!(!probe_allowed(Some(&tunneled)));
}

/// Drains the connection until its channel closes, allowing `bound` between
/// messages, and returns the terminal state, log lines and toasts.
async fn drain_within(
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
    bound: Duration,
) -> (Option<ProcessState>, Vec<String>, Vec<String>) {
    let mut terminal = None;
    let mut lines = Vec::new();
    let mut toasts = Vec::new();
    loop {
        let msg = tokio::time::timeout(bound, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => return (terminal, lines, toasts),
            Some(SessionEvent::LogLines(_, batch)) => lines.extend(batch),
            Some(SessionEvent::Toast(toast)) => toasts.push(toast),
            Some(SessionEvent::State(_, state, _)) if !relays(&state) => {
                terminal = Some(state);
            }
            Some(_) => {}
        }
    }
}

/// Collects log lines containing `marker` and toasts from every message,
/// including those before Running, until Running is reported and `want`
/// matching lines arrived.
async fn collect_until_running(
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
    marker: &str,
    want: usize,
) -> (usize, Vec<String>) {
    let mut running = false;
    let mut relayed = 0;
    let mut toasts = Vec::new();
    while !running || relayed < want {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("warning lines not relayed in time")
            .expect("connection task ended before relaying the warnings");
        match msg {
            SessionEvent::State(_, state, _) => {
                assert!(relays(&state), "reported {state:?}");
                running |= matches!(state, ProcessState::Running);
            }
            SessionEvent::LogLines(_, batch) => {
                relayed += batch.iter().filter(|line| line.contains(marker)).count();
            }
            SessionEvent::Toast(toast) => toasts.push(toast),
            _ => {}
        }
    }
    (relayed, toasts)
}

async fn stop_and_count(
    handle: &SessionHandle,
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
    marker: &str,
    relayed: &mut usize,
    toasts: &mut Vec<String>,
) {
    let _ = handle.stop(StopReason::UserStop);
    let (_, lines, rest) = drain_within(rx, RECV_TIMEOUT).await;
    toasts.extend(rest);
    *relayed += lines.iter().filter(|line| line.contains(marker)).count();
}

#[tokio::test]
async fn reality_warning_toasts_once_per_connection() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; for i in 1 2 3; do echo "2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)"; done; exec sleep 30"#,
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut settings = AppSettings::default();
    settings.backend.backend_type = BackendType::Xray;
    settings.socks_port = listener.local_addr().unwrap().port();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    let marker = "potential MITM";
    let (mut relayed, mut toasts) = collect_until_running(&mut rx, marker, 3).await;
    stop_and_count(&handle, &mut rx, marker, &mut relayed, &mut toasts).await;

    assert_eq!(relayed, 3);
    assert_eq!(
        toasts,
        [
            "Backend warning: [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)"
        ]
    );
    drop(listener);
}

#[tokio::test]
async fn startup_deprecation_warning_toasts_once() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; echo "2026/09/15 08:51:54.138024 [Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible."; echo "2026/09/15 08:51:54.138100 [Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible."; exec sleep 30"#,
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut settings = AppSettings::default();
    settings.backend.backend_type = BackendType::Xray;
    settings.socks_port = listener.local_addr().unwrap().port();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    let marker = "WebSocket transport";
    let (mut relayed, mut toasts) = collect_until_running(&mut rx, marker, 2).await;
    stop_and_count(&handle, &mut rx, marker, &mut relayed, &mut toasts).await;

    assert_eq!(relayed, 2);
    assert_eq!(toasts.len(), 1, "toasts: {toasts:?}");
    assert!(
        toasts[0].starts_with("Backend warning: [Warning] common/errors: The feature WebSocket"),
        "toast: {}",
        toasts[0]
    );
    drop(listener);
}

fn xray_tun_stub_with_probe(
    iface: &str,
) -> (Stub, AppSettings, tempfile::TempDir, crate::HostProbe) {
    let stub = stub(
        r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; exit 1"#,
    );
    let host = tempfile::tempdir().unwrap();
    let getcap = host.path().join("getcap");
    std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
    std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = crate::HostProbe {
        getcap,
        helper: executable(host.path(), "v2ray-rs-netctl"),
        helper_access: Some(crate::HelperAccess::Ready),
    };
    let mut settings = xray_tun_settings();
    settings.tun.interface_name = iface.into();
    settings.tun.strict_route = false;
    (stub, settings, host, probe)
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_narrows_pins_to_listening_address() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stub, settings, _host, probe) = xray_tun_stub_with_probe("v2rsprobe1");
    let candidate = ConnectionCandidate {
        node: ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "probe-a.invalid".into(),
            port,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: None,
        }),
        ..candidate("probe-a.invalid")
    };
    let mut req = request(&stub, settings, vec![candidate]);
    req.last_good_pins = ["127.0.0.1", "127.0.0.2"]
        .map(|ip| HostOverride {
            domain: "probe-a.invalid".into(),
            ip: ip.into(),
        })
        .to_vec();
    let (tx, mut rx) = EventSender::channel();
    let _handle = spawn_with(
        req,
        tx,
        move |mgr| mgr.with_host_probe(probe.clone()),
        no_subnets,
    );

    let (terminal, _, _) = drain_within(&mut rx, Duration::from_secs(60)).await;
    assert_error_terminal(terminal);

    let config = std::fs::read_to_string(stub.paths.generated_dir().join("xray.json"))
        .expect("generated config readable");
    let config: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(
        config["dns"]["hosts"]["probe-a.invalid"], "127.0.0.1",
        "{}",
        config["dns"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unpinned_candidates_notify_capture_off_once() {
    let (stub, settings, _host, probe) = xray_tun_stub_with_probe("v2rsnotice0");
    let (_handle, mut rx) = connect_with(
        &stub,
        settings,
        vec![candidate("notice-a.invalid"), candidate("notice-b.invalid")],
        move |mgr| mgr.with_host_probe(probe.clone()),
    );

    let (terminal, lines, toasts) = drain_within(&mut rx, Duration::from_secs(60)).await;
    assert_error_terminal(terminal);

    let notices: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("notice: DNS capture off for this session:"))
        .collect();
    assert_eq!(notices.len(), 1, "{lines:#?}");
    assert!(
        notices[0].starts_with("notice: DNS capture off for this session: notice-a.invalid"),
        "{}",
        notices[0]
    );
    assert_eq!(toasts, vec![dns_capture_off_toast("notice-a.invalid")]);

    let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(
        log.matches("DNS capture off for this session").count(),
        1,
        "{log}"
    );
    let sessions: Vec<&str> = log.lines().filter(|l| l.contains(" session ")).collect();
    assert_eq!(sessions.len(), 2, "{log}");
    for session in sessions {
        assert!(session.contains("capture_dns=false"), "{session}");
        assert!(session.contains("nodes_pinned=false"), "{session}");
    }
}

#[tokio::test]
async fn filter_reachable_keeps_only_listening_address() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let open: IpAddr = "127.0.0.1".parse().unwrap();
    let closed: IpAddr = "127.0.0.2".parse().unwrap();

    let kept = filter_reachable(
        "probe-open.invalid",
        &[closed, open],
        port,
        PIN_PROBE_CONNECT_TIMEOUT,
        PIN_PROBE_TOTAL,
    )
    .await;

    assert_eq!(kept, vec![open]);
}

#[tokio::test]
async fn filter_reachable_keeps_all_when_none_accept() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let addrs: Vec<IpAddr> = vec!["127.0.0.2".parse().unwrap(), "127.0.0.1".parse().unwrap()];

    let kept = filter_reachable(
        "probe-closed.invalid",
        &addrs,
        port,
        PIN_PROBE_CONNECT_TIMEOUT,
        PIN_PROBE_TOTAL,
    )
    .await;

    assert_eq!(kept, addrs);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_candidates_first_exits_before_ready() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && { echo "FATAL start service: listen failed" >&2; exit 1; }; exec sleep 30"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(
        &stub,
        settings,
        vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
    );

    loop {
        let (state, connection) = next_state(&mut rx).await;
        assert!(relays(&state), "failover reported {state:?}");
        if matches!(state, ProcessState::Running) {
            let address = connection.map(|c| c.node_address);
            assert_eq!(address.as_deref(), Some("203.0.113.2"));
            break;
        }
    }

    let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
        .expect("backend.log readable");
    assert_eq!(contents.matches(" session ").count(), 2, "{contents}");
    let exit_at = contents.find(" exit ").expect("exit record");
    assert_eq!(
        contents[..exit_at].matches(" session ").count(),
        1,
        "{contents}"
    );
    assert!(
        contents[..exit_at].contains("node=203.0.113.1"),
        "{contents}"
    );

    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_halts_every_forwarder() {
    let stub = stub(
        r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.05; done"#,
    );
    let (_listener, settings) = ready_singbox_settings();
    let (handle, mut rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Running) {
            break;
        }
        assert!(relays(&state), "reported {state:?}");
    }

    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(&mut rx).await;
        if matches!(state, ProcessState::Stopped) {
            break;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => break,
            Some(SessionEvent::LogLines(_, lines)) => {
                panic!("log lines {lines:?} relayed after the terminal state")
            }
            Some(SessionEvent::State(_, state, _)) => {
                panic!("{state:?} reported after the terminal state")
            }
            Some(_) => {}
        }
    }
}

struct FakeProxy {
    port: u16,
    accepts: Arc<std::sync::atomic::AtomicUsize>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeProxy {
    fn accepts(&self) -> usize {
        self.accepts.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_healthy(&self, healthy: bool) {
        self.healthy
            .store(healthy, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn fake_proxy(healthy: bool) -> FakeProxy {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = FakeProxy {
        port: listener.local_addr().unwrap().port(),
        accepts: Arc::default(),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(healthy)),
    };
    let accepts = proxy.accepts.clone();
    let healthy = proxy.healthy.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            accepts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let reply = if healthy.load(std::sync::atomic::Ordering::SeqCst) {
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    proxy
}

fn health_settings(proxy: &FakeProxy) -> (std::net::TcpListener, AppSettings) {
    let (listener, mut settings) = ready_singbox_settings();
    settings.http_port = proxy.port;
    settings.real_delay.test_url = "http://probe.test/generate_204".into();
    (listener, settings)
}

fn sleeping_stub() -> Stub {
    stub(
        r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
    )
}

async fn wait_running(rx: &mut mpsc::UnboundedReceiver<SessionEvent>) {
    loop {
        let (state, _) = next_state(rx).await;
        if matches!(state, ProcessState::Running) {
            return;
        }
        assert!(relays(&state), "reported {state:?}");
    }
}

async fn next_health(rx: &mut mpsc::UnboundedReceiver<SessionEvent>) -> Health {
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("no health reported in time")
            .expect("connection task ended before a health report");
        if let SessionEvent::Health(generation, health) = msg {
            assert_eq!(generation, GENERATION);
            return health;
        }
    }
}

async fn stop_and_wait(handle: &SessionHandle, rx: &mut mpsc::UnboundedReceiver<SessionEvent>) {
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let (state, _) = next_state(rx).await;
        if matches!(state, ProcessState::Stopped) {
            return;
        }
        assert!(relays(&state), "stop reported {state:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn running_candidate_reports_applied_pins() {
    let localhost = tokio::net::lookup_host(("localhost", 0)).await;
    assert!(
        localhost.is_ok_and(|mut addrs| addrs.next().is_some()),
        "localhost must resolve"
    );
    let stub = sleeping_stub();
    let host = tempfile::tempdir().unwrap();
    let getcap = host.path().join("getcap");
    std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
    std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = crate::HostProbe {
        getcap,
        helper: executable(host.path(), "v2ray-rs-netctl"),
        helper_access: Some(crate::HelperAccess::Ready),
    };
    let (_listener, mut settings) = ready_singbox_settings();
    settings.tun.enabled = true;
    settings.tun.interface_name = "v2rspins0".into();
    let (handle, mut rx) =
        connect_with(&stub, settings, vec![candidate("localhost")], move |mgr| {
            mgr.with_host_probe(probe.clone())
        });
    wait_running(&mut rx).await;

    let _ = handle.stop(StopReason::UserStop);
    let mut reports = Vec::new();
    let mut stopped = false;
    while let Some(msg) = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
        .await
        .expect("connection task outlived its terminal state")
    {
        match msg {
            SessionEvent::NodePins(generation, pins) => reports.push((generation, pins)),
            SessionEvent::State(_, ProcessState::Stopped, _) => stopped = true,
            _ => {}
        }
    }
    assert!(stopped);

    assert_eq!(reports.len(), 1, "{reports:?}");
    let (generation, pins) = &reports[0];
    assert_eq!(*generation, GENERATION);
    assert!(!pins.is_empty());
    for pin in pins {
        assert_eq!(pin.domain, "localhost", "{pins:?}");
        assert!(pin.ip.parse::<IpAddr>().is_ok(), "{pins:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn health_monitor_reports_unhealthy_then_healthy() {
    let stub = sleeping_stub();
    let proxy = fake_proxy(false).await;
    let (_listener, settings) = health_settings(&proxy);
    let (handle, mut rx) =
        connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
    wait_running(&mut rx).await;

    let health = next_health(&mut rx).await;
    assert!(
        matches!(health, Health::Unhealthy(ref r) if r.contains("502")),
        "{health:?}"
    );
    proxy.set_healthy(true);
    assert_eq!(next_health(&mut rx).await, Health::Healthy);

    let seen = proxy.accepts();
    tokio::time::timeout(RECV_TIMEOUT, async {
        while proxy.accepts() < seen + 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("health probes stopped after recovery");
    let _ = handle.stop(StopReason::UserStop);
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => break,
            Some(SessionEvent::Health(_, health)) => {
                panic!("repeated health report {health:?}")
            }
            Some(_) => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn health_monitor_is_not_spawned_when_disabled() {
    let stub = sleeping_stub();
    let proxy = fake_proxy(true).await;
    let (_listener, mut settings) = health_settings(&proxy);
    settings.health_check.enabled = false;
    let (handle, mut rx) =
        connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
    wait_running(&mut rx).await;

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(proxy.accepts(), 0);

    stop_and_wait(&handle, &mut rx).await;
    assert_nothing_after_terminal(&mut rx).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn health_monitor_stops_with_the_connection() {
    let stub = sleeping_stub();
    let proxy = fake_proxy(true).await;
    let (_listener, settings) = health_settings(&proxy);
    let (handle, mut rx) =
        connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
    wait_running(&mut rx).await;
    assert_eq!(next_health(&mut rx).await, Health::Healthy);

    stop_and_wait(&handle, &mut rx).await;
    assert_nothing_after_terminal(&mut rx).await;
    let after_stop = proxy.accepts();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(proxy.accepts(), after_stop, "probe sent after stop");
}

const DNS_BURST: &str = r#"sleep 2; i=0; while [ $i -lt 25 ]; do echo "2026/09/16 12:00:00 [Info] app/dns: failed to retrieve response for example.com"; i=$((i+1)); done; echo dns-burst-done; exec sleep 30"#;

fn dns_burst_stub(version: &str) -> Stub {
    stub(&format!(
        r#"[ "$1" = version ] && echo "{version}" && exit 0; [ "$1" = check ] && exit 0; [ "$2" = -test ] && exit 0; {DNS_BURST}"#
    ))
}

/// Stops the connection once the burst is relayed and returns every DNS
/// health report it emitted.
async fn dns_reports(
    handle: &SessionHandle,
    rx: &mut mpsc::UnboundedReceiver<SessionEvent>,
) -> Vec<bool> {
    let mut reports = Vec::new();
    loop {
        let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("connection task outlived its terminal state");
        match msg {
            None => return reports,
            Some(SessionEvent::DnsHealth(generation, failing)) => {
                assert_eq!(generation, GENERATION);
                reports.push(failing);
            }
            Some(SessionEvent::LogLines(_, lines))
                if lines.iter().any(|line| line == "dns-burst-done") =>
            {
                let _ = handle.stop(StopReason::UserStop);
            }
            Some(_) => {}
        }
    }
}

#[test]
fn stop_on_closed_channel_reports_session_gone() {
    let (tx, rx) = mpsc::unbounded_channel::<SessionCmd>();
    drop(rx);
    let handle = SessionHandle { cmd_tx: tx };

    assert_eq!(
        handle.stop(StopReason::UserStop),
        Err(SessionStopError::SessionGone),
        "a closed session channel must surface that the session is gone"
    );
}

#[tokio::test]
async fn repeated_stops_never_block_or_panic_on_the_runtime() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SessionCmd>();
    let handle = SessionHandle { cmd_tx: tx };
    for _ in 0..10 {
        handle
            .stop(StopReason::UserStop)
            .expect("the session task is alive");
    }
    let mut queued = 0;
    while matches!(rx.try_recv(), Ok(SessionCmd::Stop(StopReason::UserStop))) {
        queued += 1;
    }
    assert_eq!(queued, 10);
}

#[tokio::test(flavor = "multi_thread")]
async fn xray_dns_burst_reports_dns_health() {
    let stub = dns_burst_stub("Xray 26.6.27");
    let (_listener, mut settings) = ready_singbox_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.health_check.enabled = false;
    let (handle, mut rx) =
        connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);

    assert_eq!(dns_reports(&handle, &mut rx).await, vec![true]);
}

#[tokio::test(flavor = "multi_thread")]
async fn singbox_dns_burst_reports_nothing() {
    let stub = dns_burst_stub("sing-box version 1.13.0");
    let (_listener, mut settings) = ready_singbox_settings();
    settings.backend.backend_type = BackendType::SingBox;
    settings.health_check.enabled = false;
    let (handle, mut rx) =
        connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);

    assert_eq!(dns_reports(&handle, &mut rx).await, Vec::<bool>::new());
}

#[test]
fn dns_window_clears_after_the_recheck_tick() {
    let base = Instant::now();
    let mut window = DnsFailureWindow::default();
    let mut reported = false;
    for k in 0..20 {
        window.observe(
            "[Info] app/dns: failed to retrieve response for example.com",
            base + Duration::from_secs(k),
        );
    }
    let last = base + Duration::from_secs(19);
    assert_eq!(dns_flip(&window, &mut reported, last), Some(true));
    assert_eq!(dns_flip(&window, &mut reported, last), None);
    let tick = last + Duration::from_secs(61);
    assert_eq!(dns_flip(&window, &mut reported, tick), Some(false));
    assert_eq!(dns_flip(&window, &mut reported, tick), None);
}

fn node(address: &str) -> ProxyNode {
    ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: address.into(),
        port: 8388,
        method: "aes-256-gcm".into(),
        password: "secret".into(),
        remark: None,
    })
}

fn no_host_tools() -> HostTools {
    HostTools {
        bypass_uid: None,
        helper_path: PathBuf::new(),
        antidpi_path: PathBuf::new(),
    }
}

fn tun_settings() -> AppSettings {
    AppSettings {
        tun: TunConfig {
            enabled: true,
            ..TunConfig::default()
        },
        ..AppSettings::default()
    }
}

fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings {
    let mut settings = AppSettings {
        tun: TunConfig {
            enabled: tun,
            ..TunConfig::default()
        },
        ..AppSettings::default()
    };
    settings.backend.backend_type = BackendType::V2ray;
    settings.listen_address = listen.to_string();
    settings.socks_port = socks;
    settings.http_port = http;
    settings
}

#[test]
fn v2ray_with_tun_warns_with_both_endpoints() {
    let warning = v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", true))
        .expect("v2ray with TUN enabled must warn");

    assert!(warning.contains("127.0.0.1:2080"), "{warning}");
    assert!(warning.contains("127.0.0.1:2081"), "{warning}");
    assert!(warning.contains("v2ray"), "{warning}");
}

#[test]
fn v2ray_without_tun_does_not_warn() {
    assert!(v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", false)).is_none());
}

#[test]
fn singbox_and_xray_with_tun_do_not_warn() {
    for backend in [BackendType::SingBox, BackendType::Xray] {
        let mut settings = v2ray_settings(2080, 2081, "127.0.0.1", true);
        settings.backend.backend_type = backend;

        assert!(v2ray_tun_warning(&settings).is_none(), "{backend:?}");
    }
}

#[test]
fn ipv6_listen_address_is_bracketed() {
    let warning = v2ray_tun_warning(&v2ray_settings(2080, 2081, "::1", true))
        .expect("v2ray with TUN enabled must warn");

    assert!(warning.contains("[::1]:2080"), "{warning}");
    assert!(warning.contains("[::1]:2081"), "{warning}");
    assert!(!warning.contains("::1:2080"), "{warning}");
}

fn xray_tun_settings() -> AppSettings {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings
}

#[test]
fn pin_hosts_empty_without_tun() {
    let settings = AppSettings::default();
    let hosts = pin_hosts(&[candidate("proxy.example.com")], &[], &[], &[], &settings);
    assert!(hosts.is_empty(), "{hosts:?}");
}

#[test]
fn pin_hosts_empty_for_v2ray() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::V2ray;
    let hosts = pin_hosts(&[candidate("proxy.example.com")], &[], &[], &[], &settings);
    assert!(hosts.is_empty(), "{hosts:?}");
}

#[test]
fn pin_hosts_dedupes_shared_hostname() {
    let hosts = pin_hosts(
        &[
            candidate("proxy.example.com"),
            candidate("other.example.com"),
            candidate("proxy.example.com"),
        ],
        &[],
        &[],
        &[],
        &xray_tun_settings(),
    );
    assert_eq!(
        hosts,
        vec![
            ("proxy.example.com".to_string(), 8388),
            ("other.example.com".to_string(), 8388),
        ]
    );
}

#[test]
fn pin_hosts_includes_via_node() {
    let via = ManualNode {
        id: uuid::Uuid::new_v4(),
        node: node("via.example.com"),
        enabled: true,
    };
    let rule = RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::Domain {
            pattern: "example.org".into(),
        },
        action: RuleAction::Proxy,
        enabled: true,
        group: None,
        via_node: Some(ConnectionNodeRef::Manual { node_id: via.id }),
    };
    let hosts = pin_hosts(
        &[candidate("proxy.example.com")],
        &[],
        &[via],
        &[rule],
        &xray_tun_settings(),
    );
    assert_eq!(
        hosts,
        vec![
            ("proxy.example.com".to_string(), 8388),
            ("via.example.com".to_string(), 8388),
        ]
    );
}

#[test]
fn pin_hosts_skips_user_override() {
    let mut settings = xray_tun_settings();
    settings.dns.hosts.push(HostOverride {
        domain: "proxy.example.com".into(),
        ip: "203.0.113.9".into(),
    });
    let hosts = pin_hosts(&[candidate("proxy.example.com")], &[], &[], &[], &settings);
    assert!(hosts.is_empty(), "{hosts:?}");
}

#[test]
fn pin_hosts_skips_ip_literals() {
    let hosts = pin_hosts(
        &[candidate("203.0.113.9"), candidate("2001:db8::1")],
        &[],
        &[],
        &[],
        &xray_tun_settings(),
    );
    assert!(hosts.is_empty(), "{hosts:?}");
}

#[tokio::test]
async fn resolve_pins_uses_last_good_when_lookup_fails() {
    let last_good = [
        HostOverride {
            domain: "broken-h2.invalid".into(),
            ip: "203.0.113.7".into(),
        },
        HostOverride {
            domain: "broken-h2.invalid".into(),
            ip: "not-an-ip".into(),
        },
        HostOverride {
            domain: "fresh-h2.invalid".into(),
            ip: "203.0.113.99".into(),
        },
    ];
    let resolved = resolve_pins(
        vec![
            ("broken-h2.invalid".into(), 443),
            ("fresh-h2.invalid".into(), 443),
        ],
        &last_good,
        Duration::from_secs(5),
        |host: String, _port| async move {
            if host == "fresh-h2.invalid" {
                Ok(vec!["198.51.100.5".parse().unwrap()])
            } else {
                Err(std::io::Error::other("no such host"))
            }
        },
    )
    .await;

    assert_eq!(
        resolved.get("broken-h2.invalid"),
        Some(&vec!["203.0.113.7".parse::<IpAddr>().unwrap()])
    );
    assert_eq!(
        resolved.get("fresh-h2.invalid"),
        Some(&vec!["198.51.100.5".parse::<IpAddr>().unwrap()])
    );
}

#[tokio::test(start_paused = true)]
async fn resolve_pins_drops_timed_out_host_without_last_good() {
    let capture = install_test_capture();
    let started = tokio::time::Instant::now();
    let resolved = resolve_pins(
        vec![("slow.invalid".into(), 443)],
        &[],
        Duration::from_millis(100),
        |_host, _port| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(vec!["198.51.100.5".parse().unwrap()])
        },
    )
    .await;

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!resolved.contains_key("slow.invalid"), "{resolved:?}");
    assert!(
        !capture
            .lines_containing("cannot pin slow.invalid: lookup timed out")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_pins_caps_lookups_at_16() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let gate = Arc::new(Semaphore::new(0));
    let hosts = (0..40)
        .map(|i| (format!("host{i}-h2.invalid"), 443))
        .collect();
    let lookup = {
        let in_flight = Arc::clone(&in_flight);
        let peak = Arc::clone(&peak);
        let entered = Arc::clone(&entered);
        let gate = Arc::clone(&gate);
        move |_host: String, _port: u16| {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            let entered = Arc::clone(&entered);
            let gate = Arc::clone(&gate);
            async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                entered.notify_one();
                let _ = gate.acquire().await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(vec!["198.51.100.5".parse().unwrap()])
            }
        }
    };

    let resolving =
        tokio::spawn(
            async move { resolve_pins(hosts, &[], Duration::from_secs(60), lookup).await },
        );
    while in_flight.load(Ordering::SeqCst) < PIN_LOOKUP_CONCURRENCY {
        tokio::time::timeout(RECV_TIMEOUT, entered.notified())
            .await
            .expect("lookups never reached the cap");
    }
    gate.close();
    let resolved = resolving.await.unwrap();

    assert_eq!(resolved.len(), 40);
    assert_eq!(peak.load(Ordering::SeqCst), PIN_LOOKUP_CONCURRENCY);
}

#[test]
fn ip_addressed_nodes_need_no_pin() {
    let settings = tun_settings();
    assert_eq!(unpinned_host(&settings, &[node("203.0.113.9")]), None);
}

#[test]
fn hostname_without_a_pin_leaves_capture_off() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;

    let nodes = [node("proxy.example.com")];
    let unpinned = unpinned_host(&settings, &nodes);
    assert_eq!(unpinned, Some("proxy.example.com"));
    let runtime = build_tun_runtime(&settings, unpinned.is_none(), &no_host_tools()).unwrap();
    assert!(!runtime.capture_dns);
}

#[test]
fn pinned_hostname_arms_capture() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.dns.hosts.push(HostOverride {
        domain: "proxy.example.com".into(),
        ip: "203.0.113.9".into(),
    });

    let nodes = [node("proxy.example.com")];
    let unpinned = unpinned_host(&settings, &nodes);
    assert_eq!(unpinned, None);
    let runtime = build_tun_runtime(&settings, unpinned.is_none(), &no_host_tools()).unwrap();
    assert!(runtime.capture_dns);
}

#[test]
fn xray_runtime_carries_exclude_routes() {
    let mut settings = xray_tun_settings();
    settings.tun.exclude_routes = vec!["10.15.12.100/32".into(), "91.230.107.224/32".into()];
    let runtime = build_tun_runtime(&settings, true, &no_host_tools()).unwrap();
    assert_eq!(
        runtime.exclude_routes,
        ["10.15.12.100/32", "91.230.107.224/32"]
    );
}

#[test]
fn singbox_runtime_has_no_exclude_routes() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::SingBox;
    settings.tun.exclude_routes = vec!["10.15.12.100/32".into()];
    let runtime = build_tun_runtime(&settings, true, &no_host_tools()).unwrap();
    assert!(
        runtime.exclude_routes.is_empty(),
        "{:?}",
        runtime.exclude_routes
    );
}

#[test]
fn wrong_family_pin_leaves_capture_off() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.dns.strategy = DnsStrategy::Ipv4Only;
    settings.dns.hosts.push(HostOverride {
        domain: "proxy.example.com".into(),
        ip: "2001:db8::1".into(),
    });

    let nodes = [node("proxy.example.com")];
    assert_eq!(
        unpinned_host(&settings, &nodes),
        Some("proxy.example.com"),
        "an override of the wrong family is an empty answer, not a pin"
    );
}

#[test]
fn prefer_ipv6_pin_needs_a_tunnel_v6_address() {
    let mut settings = tun_settings();
    settings.backend.backend_type = BackendType::Xray;
    settings.dns.strategy = DnsStrategy::PreferIpv6;
    settings.dns.hosts.push(HostOverride {
        domain: "proxy.example.com".into(),
        ip: "2001:db8::1".into(),
    });

    let nodes = [node("proxy.example.com")];
    assert_eq!(
        unpinned_host(&settings, &nodes),
        Some("proxy.example.com"),
        "without tun.address_v6 the session resolves IPv4 only: a v6-only pin is an empty answer"
    );

    settings.tun.address_v6 = Some("fd12::1/126".into());
    assert_eq!(unpinned_host(&settings, &nodes), None);
}
