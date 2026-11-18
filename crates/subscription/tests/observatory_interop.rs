//! Live observatory interoperability checks.
//!
//! Each test starts the installed backend as a local Shadowsocks server whose
//! freedom outbound reaches a loopback HTTP endpoint, then drives the
//! production Real Delay entry point against one reachable and one unavailable
//! node. Backend versions are recorded in every failure message.

use std::net::TcpListener as StdTcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child as StdChild, Command as StdCommand, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::timeout;
use v2ray_rs_core::models::{
    BackendType, ProxyNode, RealDelaySettings, ShadowsocksConfig, SubscriptionNode,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::AppProfile;
use v2ray_rs_subscription::measure_real_delay;

const PROBE_DEADLINE: Duration = Duration::from_secs(45);
const FIXTURE_READY_TIMEOUT: Duration = Duration::from_secs(10);
const SS_METHOD: &str = "aes-128-gcm";
const SS_PASSWORD: &str = "interop-pass";

fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for dir in ["/usr/bin", "/usr/local/bin"] {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn require_or_skip(name: &str, found: Option<PathBuf>, required: bool) -> Option<PathBuf> {
    match found {
        Some(path) => Some(path),
        None if required => {
            panic!("required backend '{name}' is not installed; observatory acceptance cannot run")
        }
        None => {
            eprintln!("skipping {name} observatory interoperability: binary not installed");
            None
        }
    }
}

fn require_backend(name: &str) -> Option<PathBuf> {
    let required = std::env::var("V2RAY_RS_REQUIRE_BACKENDS").as_deref() == Ok("1");
    require_or_skip(name, find_binary(name), required)
}

async fn backend_version(binary: &Path) -> String {
    match Command::new(binary).arg("version").output().await {
        Ok(output) => {
            let bytes = if output.status.success() {
                output.stdout
            } else {
                output.stderr
            };
            String::from_utf8_lossy(&bytes)
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        }
        Err(error) => format!("version unavailable: {error}"),
    }
}

/// A local HTTP origin returning a fixed 200 response.
struct Origin {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Origin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_origin() -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = socket.flush().await;
            });
        }
    });
    Origin { port, task }
}

/// A backend process acting as a Shadowsocks server for the probe.
struct Fixture {
    child: StdChild,
    port: u16,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_loopback_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn spawn_fixture(
    binary: &Path,
    label: &str,
    dir: &Path,
    version: &str,
) -> Result<Fixture, String> {
    let port = free_loopback_port();
    let config_path = dir.join(format!("{label}-fixture.json"));
    let config = serde_json::json!({
        "log": { "loglevel": "warning" },
        "inbounds": [{
            "listen": "127.0.0.1",
            "port": port,
            "protocol": "shadowsocks",
            "settings": { "method": SS_METHOD, "password": SS_PASSWORD },
        }],
        "outbounds": [{
            "protocol": "freedom",
            // Current xray blocks private targets from Shadowsocks inbounds by
            // default; the loopback origin is the fixture's only destination.
            "settings": {
                "finalRules": [{ "action": "allow", "ip": ["127.0.0.1/32"] }],
            },
        }],
    });
    std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap())
        .map_err(|error| format!("{label} {version}: fixture config write failed: {error}"))?;
    let log_path = dir.join(format!("{label}-fixture.log"));
    let log = std::fs::File::create(&log_path)
        .map_err(|error| format!("{label} {version}: fixture log create failed: {error}"))?;

    let child = StdCommand::new(binary)
        .arg("run")
        .arg("-c")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| format!("{label} {version}: fixture spawn failed: {error}"))?;

    let mut fixture = Fixture { child, port };
    let deadline = Instant::now() + FIXTURE_READY_TIMEOUT;
    loop {
        match fixture.child.try_wait() {
            Ok(Some(status)) => {
                let stderr = std::fs::read_to_string(&log_path).unwrap_or_default();
                return Err(format!(
                    "{label} {version}: fixture exited with {status}: {}",
                    stderr.trim()
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(format!(
                    "{label} {version}: fixture status check failed: {error}"
                ));
            }
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(fixture);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{label} {version}: fixture did not listen on 127.0.0.1:{port}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn shadowsocks_node(port: u16) -> SubscriptionNode {
    SubscriptionNode::new(ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: "127.0.0.1".into(),
        port,
        method: SS_METHOD.into(),
        password: SS_PASSWORD.into(),
        remark: None,
    }))
}

/// PIDs still referencing the session's runtime directory, which contains the
/// ephemeral probe config passed on the backend command line.
fn probe_processes(runtime_dir: &Path) -> Vec<u32> {
    let needle = runtime_dir.to_string_lossy();
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        if String::from_utf8_lossy(&cmdline).contains(needle.as_ref()) {
            found.push(pid);
        }
    }
    found
}

async fn assert_backend_interop(backend: BackendType, binary: &Path) {
    static RUSTLS_PROVIDER: std::sync::Once = std::sync::Once::new();
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    let label = match backend {
        BackendType::Xray => "xray",
        BackendType::V2ray => "v2ray",
        BackendType::SingBox => "sing-box",
    };
    let version = backend_version(binary).await;
    let fixture_dir = TempDir::new().unwrap();
    let origin = spawn_origin().await;
    let fixture = match spawn_fixture(binary, label, fixture_dir.path(), &version).await {
        Ok(fixture) => fixture,
        Err(error) => panic!("{error}"),
    };

    let unavailable_port = free_loopback_port();
    let reachable = shadowsocks_node(fixture.port);
    let unavailable = shadowsocks_node(unavailable_port);
    let nodes = [&reachable, &unavailable];
    let config = RealDelaySettings {
        enabled: true,
        test_url: format!("http://127.0.0.1:{}/health", origin.port),
        timeout_ms: 1000,
        use_for_lowest_latency: false,
    };

    let paths_dir = TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, paths_dir.path());
    paths.ensure_dirs().unwrap();
    let runtime_dir = paths.runtime_dir().to_path_buf();

    let report = match timeout(
        PROBE_DEADLINE,
        measure_real_delay(backend, binary, &nodes, &config, &paths),
    )
    .await
    {
        Ok(report) => report,
        Err(_) => panic!("{label} {version}: Real Delay did not finish within {PROBE_DEADLINE:?}"),
    };

    assert_eq!(
        report.results.len(),
        2,
        "{label} {version}: expected one result per node, got {:?} (diagnostic {:?})",
        report.results,
        report.diagnostic
    );
    assert!(
        report.results[0].is_some(),
        "{label} {version}: reachable node received no delay; results {:?}, diagnostic {:?}",
        report.results,
        report.diagnostic
    );
    assert!(
        report.results[1].is_none(),
        "{label} {version}: unavailable node received a delay; results {:?}, diagnostic {:?}",
        report.results,
        report.diagnostic
    );

    let survivors = probe_processes(&runtime_dir);
    assert!(
        survivors.is_empty(),
        "{label} {version}: probe processes survived shutdown: {survivors:?}"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&runtime_dir)
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("real-delay-")
        })
        .map(|entry| entry.path())
        .collect();
    assert!(
        leftovers.is_empty(),
        "{label} {version}: probe config files survived shutdown: {leftovers:?}"
    );
}

#[tokio::test]
async fn xray_observatory_measures_local_fixture() {
    let Some(binary) = require_backend("xray") else {
        return;
    };
    assert_backend_interop(BackendType::Xray, &binary).await;
}

#[tokio::test]
async fn v2ray_observatory_measures_local_fixture() {
    let Some(binary) = require_backend("v2ray") else {
        return;
    };
    assert_backend_interop(BackendType::V2ray, &binary).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_probes_serialize_and_shut_down() {
    let (Some(xray), Some(v2ray)) = (require_backend("xray"), require_backend("v2ray")) else {
        return;
    };
    tokio::join!(
        assert_backend_interop(BackendType::Xray, &xray),
        assert_backend_interop(BackendType::V2ray, &v2ray),
    );
}

#[test]
#[should_panic(expected = "required backend 'xray' is not installed")]
fn missing_required_backend_fails() {
    let _ = require_or_skip("xray", None, true);
}

#[test]
fn missing_optional_backend_skips() {
    assert!(require_or_skip("xray", None, false).is_none());
}
