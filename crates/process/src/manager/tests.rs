use super::*;
use crate::tun::TunRuntime;
use v2ray_rs_core::models::BackendType;
use v2ray_rs_core::rotating_log::DEFAULT_MAX_BYTES;

fn write_script(dir: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("backend");
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager {
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    let binary = write_script(dir.path(), script_body);
    ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
}

const VERSION_STUB: &str = "if [ \"$1\" = version ]; then echo 'Xray 26.3.27 (Xray, Penetrates Everything.)'; exit 0; fi\n";
const SINGBOX_VERSION_STUB: &str =
    "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n";
fn backend_log(dir: &std::path::Path) -> Arc<RotatingFileWriter> {
    Arc::new(RotatingFileWriter::open(dir.join("backend.log"), DEFAULT_MAX_BYTES).unwrap())
}

#[tokio::test]
async fn explicit_tun_start_renews_completed_runtime_identity() {
    use v2ray_rs_core::persistence::{TunOwnership, TunPhase, TunSession};
    let dir = tempfile::TempDir::new().unwrap();
    let paths = v2ray_rs_core::persistence::AppPaths::for_profile_in(
        v2ray_rs_core::profile::AppProfile::Test,
        dir.path(),
    );
    let mut session = TunSession {
        backend: BackendType::Xray,
        iface: "lo".into(),
        ownership: Some(TunOwnership::new().unwrap()),
    };
    session.ownership.as_mut().unwrap().phase = TunPhase::Complete;
    v2ray_rs_core::persistence::save_tun_session(&paths, &session).unwrap();
    let journal_path = paths.tun_session_path();
    let old = session.ownership.unwrap().session_id.to_string();
    let (helper, _) = stub_helper(dir.path(), "echo \"$1 cap_net_admin=ep\"\n");
    let probe = helper.clone();
    let mut rt = xray_on_lo(helper);
    rt.device_ifindex = Some(999);
    rt.journal = Some(crate::tun::TunJournal {
        path: journal_path,
        session_id: old.clone(),
    });
    let mut mgr =
        manager_for(&dir, &format!("{VERSION_STUB}exit 1\n")).with_backend(BackendType::Xray);
    mgr.tun = Some(rt);
    mgr.host_probe = Some(HostProbe {
        getcap: probe.clone(),
        helper: probe,
        helper_access: Some(crate::tun::HelperAccess::Ready),
    });
    // No backend is spawned: configuration rejection isolates the start
    // boundary, and every earlier gate (including the journal renewal)
    // must succeed first.
    let start = mgr.start().await;
    assert!(
        matches!(start, Err(ProcessError::ConfigCheck(_))),
        "expected the config rejection, got {start:?}"
    );
    let runtime = mgr.tun.as_ref().unwrap();
    assert_ne!(runtime.journal.as_ref().unwrap().session_id, old);
    assert!(runtime.device_ifindex.is_none());
}

#[tokio::test]
async fn config_check_failure_prevents_spawn() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
            &dir,
            &format!("{SINGBOX_VERSION_STUB}if [ \"$1\" = check ]; then echo 'FATAL bad dns config' >&2; exit 1; fi\nexec sleep 30\n"),
        )
        .with_backend(BackendType::SingBox);

    let result = mgr.start_with_connection(None).await;
    assert!(
        matches!(result, Err(ProcessError::ConfigCheck(_))),
        "expected ConfigCheck error, got {result:?}"
    );
    match mgr.state() {
        ProcessState::Error(msg) => assert!(msg.contains("FATAL bad dns config"), "{msg}"),
        other => panic!("expected Error state, got {other:?}"),
    }
    assert!(mgr.child.is_none(), "no backend should have been spawned");
}

#[tokio::test]
async fn config_check_success_starts_backend() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exit 0\nexec sleep 30\n"),
    )
    .with_backend(BackendType::SingBox);

    mgr.start_with_connection(None).await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn crash_error_includes_last_stderr_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "echo 'fatal: tls handshake exploded' >&2\nexit 3\n");
    mgr.set_auto_restart(false);

    mgr.start_with_connection(None).await.unwrap();
    let code = mgr.wait_and_handle_exit().await.unwrap();
    assert_eq!(code, Some(3));
    match mgr.state() {
        ProcessState::Error(msg) => {
            assert!(msg.contains("code 3"), "{msg}");
            assert!(msg.contains("tls handshake exploded"), "{msg}");
        }
        other => panic!("expected Error state, got {other:?}"),
    }
}

#[tokio::test]
async fn startup_failure_writes_one_session_and_no_crash() {
    let dir = tempfile::TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut mgr = manager_for(&dir, "echo FATAL >&2\nexit 1\n")
        .with_log_file(Some(backend_log(dir.path())))
        .with_ready_probe(listener.local_addr().unwrap());
    mgr.restart_delay = Duration::from_millis(50);

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::ExitedBeforeReady(_))),
        "{result:?}"
    );

    let log = dir.path().join("backend.log");
    let sessions = |lines: &[String]| lines.iter().filter(|l| l.contains(" session ")).count();
    let lines = read_lines(&log);
    assert_eq!(sessions(&lines), 1, "{lines:#?}");
    let exit = lines
        .iter()
        .find(|l| l.contains(" exit "))
        .unwrap_or_else(|| panic!("no exit record: {lines:#?}"));
    for field in [
        "exit requested=false reason=start-failed",
        "code=1",
        "crashes_in_window=0",
        "last_output=FATAL",
    ] {
        assert!(exit.contains(field), "{field} missing in {exit}");
    }

    sleep(Duration::from_secs(3)).await;
    assert_eq!(sessions(&read_lines(&log)), 1);
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
}

#[tokio::test]
async fn respawn_readiness_failure_counts_as_crash() {
    let dir = tempfile::TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let runs = dir.path().join("runs");
    let script = format!(
        "echo run >> {runs}\n[ $(wc -l < {runs}) -le 1 ] && {{ sleep 2; exit 1; }}\nexit 1\n",
        runs = runs.display()
    );
    let mut mgr = manager_for(&dir, &script).with_ready_probe(listener.local_addr().unwrap());
    mgr.restart_delay = Duration::from_millis(50);

    mgr.start().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    while mgr.state() == ProcessState::Running {
        mgr.wait_and_handle_exit().await.unwrap();
    }

    match mgr.state() {
        ProcessState::Error(msg) => {
            assert!(msg.contains("3 crashes within 60s"), "{msg}");
            assert!(msg.contains("restart failed"), "{msg}");
        }
        other => panic!("expected Error state, got {other:?}"),
    }
    assert_eq!(mgr.crash_times.len(), MAX_CRASHES);
    assert_eq!(read_lines(&runs).len(), MAX_CRASHES);
}

#[tokio::test]
async fn backend_log_captures_all_lines_under_load() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}seq 1 20000; exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    let lines = wait_for_lines(&dir.path().join("backend.log"), 20_001).await;
    assert_eq!(
        lines.len(),
        20_001,
        "session record plus 20,000 stdout lines"
    );
    assert!(lines[0].contains("session backend="), "{}", lines[0]);
    for (i, line) in lines[1..].iter().enumerate() {
        assert!(
            line.ends_with(&format!("stdout {}", i + 1)),
            "out of order at position {}: {line}",
            i + 1
        );
    }
    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn session_record_precedes_spawn() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        &format!("{VERSION_STUB}echo out-line\necho err-line >&2\nexec sleep 30\n"),
    )
    .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    let lines = wait_for_lines(&dir.path().join("backend.log"), 3).await;
    let session = &lines[0];
    assert!(session.contains("session backend=unknown"), "{session}");
    assert!(session.contains("version=26.3.27"), "{session}");
    assert!(session.contains("node=none"), "{session}");
    assert!(session.contains("tun=off"), "{session}");
    assert!(
        lines[1..].iter().any(|l| l.contains("stdout out-line")),
        "{lines:?}"
    );
    assert!(
        lines[1..].iter().any(|l| l.contains("stderr err-line")),
        "{lines:?}"
    );
    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn session_record_states_utc_offset() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = read_lines(&dir.path().join("backend.log"));
    let session = lines
        .iter()
        .find(|l| l.contains(" session "))
        .unwrap_or_else(|| panic!("no session record: {lines:?}"));
    let expected = format!("utc_offset={}", local_utc_offset());
    assert!(session.contains(&expected), "{session}");
    let value = session
        .split(' ')
        .find_map(|f| f.strip_prefix("utc_offset="))
        .unwrap();
    let b = value.as_bytes();
    assert!(
        b.len() == 6
            && matches!(b[0], b'+' | b'-')
            && b[1..3].iter().all(u8::is_ascii_digit)
            && b[3] == b':'
            && b[4..].iter().all(u8::is_ascii_digit),
        "{session}"
    );
}

#[tokio::test]
async fn session_record_version_stripped_of_escapes() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
            &dir,
            "if [ \"$1\" = version ]; then printf '\\033[1mcustom-build\\033[0m\\n'; exit 0; fi\nexec sleep 30\n",
        )
        .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let log = std::fs::read_to_string(dir.path().join("backend.log")).unwrap();
    assert!(!log.contains('\u{1b}'), "{log:?}");
    assert!(log.contains("version=custom-build node="), "{log}");
}

#[tokio::test]
async fn backend_escapes_stripped_before_file_buffer_and_stream() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
            &dir,
            &format!("{VERSION_STUB}printf '\\033[31mFATAL\\033[0m[0000] start service: boom\\n' >&2\nprintf '\\033]0;t\\007plain out\\n'\nexit 0\n"),
        )
        .with_log_file(Some(backend_log(dir.path())));
    let mut rx = mgr.subscribe_logs();
    mgr.set_auto_restart(false);

    mgr.start_with_connection(None).await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();

    let log = std::fs::read_to_string(dir.path().join("backend.log")).unwrap();
    assert!(!log.contains('\u{1b}'), "{log:?}");
    assert!(
        log.contains("stderr FATAL[0000] start service: boom"),
        "{log}"
    );
    assert!(log.contains("stdout plain out"), "{log}");

    let buffered: Vec<String> = mgr
        .log_buffer()
        .lock()
        .unwrap()
        .last_n(50)
        .iter()
        .map(|l| l.content.clone())
        .collect();
    assert!(
        buffered
            .iter()
            .any(|c| c == "FATAL[0000] start service: boom"),
        "{buffered:?}"
    );
    assert!(buffered.iter().any(|c| c == "plain out"), "{buffered:?}");
    assert!(
        buffered.iter().all(|c| !c.contains('\u{1b}')),
        "{buffered:?}"
    );

    let mut streamed = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let ProcessEvent::LogLine(line) = event {
            streamed.push(line.content);
        }
    }
    assert!(
        streamed
            .iter()
            .any(|c| c == "FATAL[0000] start service: boom"),
        "{streamed:?}"
    );
    assert!(
        streamed.iter().all(|c| !c.contains('\u{1b}')),
        "{streamed:?}"
    );
}

#[tokio::test]
async fn helper_lines_stripped_of_escapes() {
    let dir = tempfile::TempDir::new().unwrap();
    let mgr = manager_for(&dir, "exit 0\n").with_log_file(Some(backend_log(dir.path())));
    let mut rx = mgr.subscribe_logs();

    mgr.log_helper(
        "xray-up",
        &HelperRun {
            output: vec!["\u{1b}[1mlink up\u{1b}[0m".into()],
            result: Err("\u{1b}[31mno\u{1b}[0m".into()),
            timed_out: false,
        },
    );

    let log = std::fs::read_to_string(dir.path().join("backend.log")).unwrap();
    assert!(!log.contains('\u{1b}'), "{log:?}");
    assert!(log.contains("helper link up"), "{log}");
    assert!(log.contains("helper xray-up failed: no"), "{log}");

    let buffered: Vec<String> = mgr
        .log_buffer()
        .lock()
        .unwrap()
        .last_n(10)
        .iter()
        .map(|l| l.content.clone())
        .collect();
    assert_eq!(buffered, ["link up", "xray-up failed: no"]);

    let mut streamed = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let ProcessEvent::LogLine(line) = event {
            streamed.push(line.content);
        }
    }
    assert_eq!(streamed, ["link up", "xray-up failed: no"]);
}

#[tokio::test]
async fn session_record_appends_caller_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    let fields = "hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app";
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())))
        .with_session_fields(fields.into());

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = read_lines(&dir.path().join("backend.log"));
    let sessions: Vec<&String> = lines.iter().filter(|l| l.contains(" session ")).collect();
    assert_eq!(sessions.len(), 1, "{lines:?}");
    assert!(
        sessions[0].contains("tun=off utc_offset="),
        "{}",
        sessions[0]
    );
    assert!(sessions[0].ends_with(fields), "{}", sessions[0]);
}

#[tokio::test]
async fn session_fields_cannot_forge_a_second_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())))
        .with_session_fields("hijack=hijack\n2026-01-01 session forged=true\rx=y".into());

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = read_lines(&dir.path().join("backend.log"));
    let sessions: Vec<&String> = lines.iter().filter(|l| l.contains(" session ")).collect();
    assert_eq!(sessions.len(), 1, "{lines:?}");
    assert!(
        sessions[0].contains("hijack=hijack 2026-01-01 session forged=true x=y"),
        "{}",
        sessions[0]
    );
}

#[tokio::test]
async fn session_extra_dns_lines_follow_session_on_start() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())))
        .with_session_extra(vec![
            "dns server=udp://1.1.1.1".into(),
            "dns server=udp://8.8.8.8".into(),
        ]);

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = wait_for_lines(&dir.path().join("backend.log"), 3).await;
    assert!(lines[0].contains("session backend="), "{}", lines[0]);
    assert!(
        lines[1].contains("dns server=udp://1.1.1.1"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains("dns server=udp://8.8.8.8"),
        "{}",
        lines[2]
    );
    assert_eq!(lines.iter().filter(|l| l.contains(" session ")).count(), 1);
}

#[tokio::test]
async fn session_extra_dns_lines_repeat_after_crash_respawn() {
    let dir = tempfile::TempDir::new().unwrap();
    let script = format!("{VERSION_STUB}{}", crashing_backend(dir.path(), 1));
    let mut mgr = manager_for(&dir, &script)
        .with_log_file(Some(backend_log(dir.path())))
        .with_session_extra(vec![
            "dns server=udp://9.9.9.9".into(),
            "dns server=udp://1.0.0.1".into(),
        ]);
    mgr.restart_delay = Duration::from_millis(50);
    mgr.start().await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();
    let _ = mgr.shutdown().await;

    let lines = wait_for_lines(&dir.path().join("backend.log"), 7).await;
    let dns = lines.iter().filter(|l| l.contains(" dns server=")).count();
    assert_eq!(dns, 4, "{lines:#?}");
    let records: Vec<&String> = lines
        .iter()
        .filter(|l| {
            l.contains("session backend=") || l.contains(" dns server=") || l.contains(" exit ")
        })
        .collect();
    assert_eq!(records.len(), 8, "{lines:#?}");
    let sessions: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("session backend="))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(sessions, [0, 4], "{records:#?}");
    assert!(
        records[1].contains("dns server=udp://9.9.9.9"),
        "{}",
        records[1]
    );
    assert!(
        records[2].contains("dns server=udp://1.0.0.1"),
        "{}",
        records[2]
    );
    assert!(
        records[5].contains("dns server=udp://9.9.9.9"),
        "{}",
        records[5]
    );
    assert!(
        records[6].contains("dns server=udp://1.0.0.1"),
        "{}",
        records[6]
    );
}

#[tokio::test]
async fn session_extra_flattens_newlines() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())))
        .with_session_extra(vec![
            "dns server=udp://1.1.1.1\n2026-01-01 dns forged=true".into(),
        ]);

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = read_lines(&dir.path().join("backend.log"));
    let dns: Vec<&String> = lines.iter().filter(|l| l.contains(" dns ")).collect();
    assert_eq!(dns.len(), 1, "{lines:?}");
    assert!(
        dns[0].contains("dns server=udp://1.1.1.1 2026-01-01 dns forged=true"),
        "{}",
        dns[0]
    );
}

#[tokio::test]
async fn no_session_extra_writes_no_dns_lines() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();

    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    assert!(lines.iter().all(|l| !l.contains(" dns ")), "{lines:?}");
    assert!(lines[0].contains("session backend="), "{}", lines[0]);
}

#[tokio::test]
async fn session_record_sanitizes_node_name_newlines() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));
    let connection: ConnectionMetadata = serde_json::from_str(
        r#"{
                "node_ref": {"type": "manual", "node_id": "00000000-0000-0000-0000-000000000000"},
                "source": "Manual",
                "source_id": "00000000-0000-0000-0000-000000000001",
                "node_name": "evil\nnode\rname",
                "node_address": "example.com",
                "node_port": 443,
                "backend": "xray",
                "strategy": "list-order",
                "latency_ms": null,
                "connected_since": "2026-01-01T00:00:00Z"
            }"#,
    )
    .unwrap();

    mgr.start_with_connection(Some(connection)).await.unwrap();
    mgr.stop().await.unwrap();

    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    let sessions: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("session backend="))
        .collect();
    assert_eq!(
        sessions.len(),
        1,
        "a newline in the node name must not forge extra record lines: {lines:?}"
    );
    assert!(
        sessions[0].contains("node=evil node name"),
        "{}",
        sessions[0]
    );
}

#[tokio::test]
async fn exit_record_marks_requested_stop() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));

    mgr.start().await.unwrap();
    mgr.stop().await.unwrap();
    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    assert!(lines[0].contains("session backend="), "{}", lines[0]);
    let exit = lines
        .iter()
        .find(|l| l.contains("exit requested="))
        .expect("exit record");
    assert!(
        exit.contains("exit requested=true reason=user-stop"),
        "{exit}"
    );
    assert!(exit.contains("signal=15"), "{exit}");
    assert!(exit.contains("crashes_in_window=0"), "{exit}");
}

#[tokio::test]
async fn exit_record_reason_per_stop_reason() {
    for reason in [
        StopReason::NodeSwitch,
        StopReason::ApplyRestart,
        StopReason::AppQuit,
        StopReason::HealthFailover,
    ] {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())));

        mgr.start().await.unwrap();
        let _ = mgr.shutdown_with(reason).await;
        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        let exits: Vec<_> = lines.iter().filter(|l| l.contains(" exit ")).collect();
        assert_eq!(exits.len(), 1, "{lines:#?}");
        let expected = format!("exit requested=true reason={reason}");
        assert!(exits[0].contains(&expected), "{}", exits[0]);
    }
}

#[tokio::test]
async fn xray_setup_receives_observed_device_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let args_path = dir.path().join("helper-args");
    let (helper, _) = stub_helper(
        dir.path(),
        &format!("printf '%s\\n' \"$@\" > {}\n", args_path.display()),
    );
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"));
    mgr.tun = Some(xray_on_lo(helper));
    mgr.launch().await.unwrap();
    let args = std::fs::read_to_string(&args_path).unwrap();
    let expected = format!(
        "--ifindex\n{}\n",
        crate::tun::read_device_ifindex("lo").unwrap()
    );
    let _ = mgr.shutdown().await;
    assert!(args.contains(&expected), "{args}");
}

#[tokio::test]
async fn exit_record_marks_start_failed_on_xray_up_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
        .with_log_file(Some(backend_log(dir.path())));
    let (helper, _) = stub_helper(dir.path(), "[ \"$1\" = xray-up ] && exit 1\nexit 0\n");
    mgr.tun = Some(xray_on_lo(helper));

    let result = mgr.launch().await;
    assert!(
        matches!(result, Err(ProcessError::TunHelper(_))),
        "{result:?}"
    );
    assert!(mgr.child.is_none());

    let lines = read_lines(&dir.path().join("backend.log"));
    let exits: Vec<_> = lines
        .iter()
        .filter(|l| l.contains(" exit requested="))
        .collect();
    assert_eq!(exits.len(), 1, "{lines:#?}");
    assert!(
        exits[0].contains("exit requested=false reason=start-failed"),
        "{}",
        exits[0]
    );
}

#[tokio::test]
async fn exit_record_marks_unrequested_crash() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        &format!("{VERSION_STUB}echo 'fatal: tls handshake exploded' >&2\nexit 3\n"),
    )
    .with_log_file(Some(backend_log(dir.path())));
    mgr.set_auto_restart(false);

    mgr.start().await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();
    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    assert!(lines[0].contains("session backend="), "{}", lines[0]);
    let exit = lines
        .iter()
        .find(|l| l.contains("exit requested="))
        .expect("exit record");
    assert!(exit.contains("exit requested=false reason=crash"), "{exit}");
    assert!(exit.contains("code=3"), "{exit}");
    assert!(exit.contains("crashes_in_window=1"), "{exit}");
    assert!(
        exit.contains("last_output=fatal: tls handshake exploded"),
        "{exit}"
    );
}

async fn crash_exit_record(dir: &tempfile::TempDir, body: &str) -> (String, String) {
    let mut mgr = manager_for(dir, &format!("{VERSION_STUB}{body}"))
        .with_log_file(Some(backend_log(dir.path())));
    mgr.set_auto_restart(false);

    mgr.start().await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();
    let ProcessState::Error(msg) = mgr.state() else {
        panic!("expected Error state, got {:?}", mgr.state());
    };
    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    let exit = lines
        .iter()
        .find(|l| l.contains(" exit requested="))
        .unwrap_or_else(|| panic!("no exit record: {lines:#?}"))
        .clone();
    (exit, msg)
}

#[tokio::test]
async fn last_error_survives_access_noise() {
    let dir = tempfile::TempDir::new().unwrap();
    let access =
        "2026/09/16 10:00:01 from 127.0.0.1:50002 accepted tcp:example.com:443 [socks -> proxy]";
    let body = format!(
        "echo '[Warning] cert about to expire'\necho '2026/09/16 10:00:00 from 127.0.0.1:50001 accepted tcp:example.org:443 [socks -> proxy]'\necho '{access}'\nexit 3\n"
    );
    let (exit, msg) = crash_exit_record(&dir, &body).await;
    assert!(
        exit.contains(&format!(
            "last_output={access} last_error=[Warning] cert about to expire"
        )),
        "{exit}"
    );
    assert_eq!(msg, format!("process exited with code 3: {access}"));
}

#[tokio::test]
async fn last_error_is_none_without_warnings() {
    let dir = tempfile::TempDir::new().unwrap();
    let (exit, _) = crash_exit_record(&dir, "echo 'started'\nexit 3\n").await;
    assert!(
        exit.ends_with("last_output=started last_error=none"),
        "{exit}"
    );
}

#[tokio::test]
async fn last_error_matches_through_ansi() {
    let dir = tempfile::TempDir::new().unwrap();
    let (exit, _) = crash_exit_record(
        &dir,
        "printf '\\033[33m[Warning]\\033[0m tls retry\\n'\necho 'done'\nexit 3\n",
    )
    .await;
    assert!(exit.ends_with("last_error=[Warning] tls retry"), "{exit}");
}

fn stub_helper(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let calls = dir.join("calls");
    let helper = dir.join("netctl");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$1\" >> {}\n{body}", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    (helper, calls)
}

/// A `getcap` stub that reports every queried file as holding the
/// capability pair, so a pinned probe can reach the gate under test
/// without the host's real `getcap`.
fn stub_getcap(dir: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("getcap");
    std::fs::write(
        &path,
        "#!/bin/sh\necho \"$1 cap_net_admin=ep cap_net_raw=ep\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn xray_on_lo(helper: PathBuf) -> TunRuntime {
    TunRuntime {
        backend: BackendType::Xray,
        iface: "lo".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: helper,
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    }
}

// --- Anti-DPI lifecycle --------------------------------------------------

/// Writes an anti-DPI stub beside `calls` and logs every invocation's
/// arguments into it. The engine spawns by this exact resolved path, so
/// the stub identity is fully controlled by `antidpi_rt`.
fn stub_antidpi(calls: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let helper = calls.parent().unwrap().join("v2ray-rs-antidpi");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$@\" >> {}\n{body}", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    helper
}

fn antidpi_rt(helper: PathBuf) -> crate::antidpi::AntiDpiRuntime {
    crate::antidpi::AntiDpiRuntime {
        helper_path: helper,
        queue: 200,
        strategies: vec!["multisplit:pos=midsld".to_string()],
        hostlist: None,
    }
}

/// Pins the preflight to `Ready` for lifecycle tests: the stub helper is
/// user-owned, which the real classification would reject, and the caps
/// probe cannot be stubbed for the anti-DPI pair.
fn pin_ready(stub: &Path) -> HostProbe {
    HostProbe {
        getcap: stub.to_path_buf(),
        helper: stub.to_path_buf(),
        helper_access: Some(crate::tun::HelperAccess::Ready),
    }
}

#[tokio::test]
async fn antidpi_install_precedes_spawn_and_engine_follows_route_setup() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exec sleep 30 ;; esac\nexit 0\n",
    );
    let (netctl, _) = stub_helper(dir.path(), "echo \"$1 cap_net_admin=ep\"\nexit 0\n");
    let marker = dir.path().join("backend-runs");
    let probe = netctl.clone();
    let mut mgr = manager_for(
        &dir,
        &format!("echo backend >> {}\nexec sleep 30\n", marker.display()),
    )
    .with_tun(Some(xray_on_lo(netctl)))
    .with_host_probe(HostProbe {
        getcap: probe.clone(),
        helper: probe,
        helper_access: Some(crate::tun::HelperAccess::Ready),
    })
    .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    // The engine stub logs its arguments right after the spawn syscall
    // returns; give it a moment and the backend stub the same.
    let deadline = Instant::now() + Duration::from_secs(2);
    let lines = loop {
        let lines = read_lines(&calls);
        if lines.iter().any(|l| l.starts_with("run ")) && std::fs::read_to_string(&marker).is_ok() {
            break lines;
        }
        assert!(
            Instant::now() < deadline,
            "engine or backend never started: {lines:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let install = lines
        .iter()
        .position(|l| l.starts_with("install-rules"))
        .expect("install-rules ran");
    let up = lines
        .iter()
        .position(|l| l.starts_with("xray-up"))
        .expect("xray-up ran");
    let run = lines
        .iter()
        .position(|l| l.starts_with("run "))
        .expect("engine ran");
    // install-rules is awaited before try_spawn, so it precedes the
    // backend's route setup; the engine starts after that setup.
    assert!(install < up && up < run, "{lines:?}");
    // The engine arguments carry the queue and the user strategies.
    assert!(lines[run].contains("--queue 200"), "{:?}", lines[run]);
    assert!(
        lines[run].contains("--strategy multisplit:pos=midsld"),
        "{:?}",
        lines[run]
    );
    // One session id per manager: identical on install and engine run,
    // and reused on the stop's owner-qualified removal.
    let session = mgr
        .antidpi_session_id
        .as_deref()
        .expect("with_antidpi allocates the session id")
        .to_string();
    assert!(
        lines[install].contains(&format!("--session-id {session}")),
        "{:?}",
        lines[install]
    );
    assert!(
        lines[run].contains(&format!("--session-id {session}")),
        "{:?}",
        lines[run]
    );
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
    let lines = read_lines(&calls);
    let remove = lines
        .iter()
        .position(|l| l.starts_with("remove-rules"))
        .expect("stop removed the rules");
    assert!(
        !lines[remove].contains("--sweep")
            && lines[remove].contains(&format!("--session-id {session}")),
        "{:?}",
        lines[remove]
    );
}

#[tokio::test]
async fn antidpi_install_failure_aborts_the_start_without_spawning() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let marker = dir.path().join("backend-runs");
    // Every subcommand fails, install-rules first.
    let antidpi = stub_antidpi(&calls, "exit 1\n");
    let mut mgr = manager_for(
        &dir,
        &format!("echo backend >> {}\nexec sleep 30\n", marker.display()),
    )
    .with_host_probe(pin_ready(&antidpi))
    .with_antidpi(Some(antidpi_rt(antidpi)));

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiHelper(_))),
        "expected an anti-DPI helper error, got {result:?}"
    );
    assert!(
        mgr.child.is_none(),
        "no backend may spawn after a failed install"
    );
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    // The failed start attempts the rule removal on the way down. The
    // stub fails that too, so the stop surfaces the cleanup error and
    // keeps the obligation for the next stop.
    let removal = mgr.stop().await;
    assert!(
        matches!(removal, Err(ProcessError::AntiDpiHelper(_))),
        "expected the cleanup error, got {removal:?}"
    );
    assert!(mgr.antidpi_rules_installed);
    let lines = read_lines(&calls);
    assert!(
        lines.iter().any(|l| l.starts_with("remove-rules")),
        "rules removal must be attempted after a failed start: {lines:?}"
    );
}

#[tokio::test]
async fn antidpi_stop_terminates_the_engine_then_removes_rules() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let exitlog = dir.path().join("engine-exit");
    let body = format!(
        "case \"$1\" in\n  run) echo \"v2ray-rs-antidpi: ready queue=200\"; trap 'echo engine-exit >> {}' TERM; while :; do sleep 1; done ;;\n  remove-rules) [ -f {} ] && echo remove-rules-ok >> {} ;;\nesac\nexit 0\n",
        exitlog.display(),
        exitlog.display(),
        calls.display()
    );
    let antidpi = stub_antidpi(&calls, &body);
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    // Wait for the engine stub to log its arguments before stopping, so
    // the SIGTERM cannot arrive before the stub even started.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !read_lines(&calls).iter().any(|l| l.starts_with("run ")) {
        assert!(Instant::now() < deadline, "engine never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    mgr.stop().await.unwrap();
    let lines = read_lines(&calls);
    assert!(lines.iter().any(|l| l.starts_with("install-rules")));
    assert!(lines.iter().any(|l| l.starts_with("run ")));
    // The stub only reports remove-rules-ok once the engine's own TERM
    // trap has fired — termination precedes rule removal.
    assert!(
        lines.iter().any(|l| l == "remove-rules-ok"),
        "remove-rules must run after the engine died: {lines:?}"
    );
    let remove = lines
        .iter()
        .position(|l| l.starts_with("remove-rules"))
        .expect("remove-rules ran");
    assert!(
        !lines[remove].contains("--sweep"),
        "a requested stop uses plain remove-rules: {:?}",
        lines[remove]
    );
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn antidpi_stop_removes_after_an_unexpected_engine_exit() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    // The engine self-exits immediately after starting.
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exit 1 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    // The engine self-exits; wait until the monitor records the
    // unexpected exit instead of probing a cached pid — the manager no
    // longer keeps one, and only the monitor reaps the child. Subscribe
    // before the start: the stub exits right after its ready line.
    let mut events = mgr.subscribe_logs();
    mgr.start().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "engine exit was never recorded");
        match events.try_recv() {
            Ok(ProcessEvent::LogLine(line)) => {
                if line.content.contains("exited unexpectedly") {
                    break;
                }
            }
            _ => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }

    let session = mgr
        .antidpi_session_id
        .as_deref()
        .expect("with_antidpi allocates the session id")
        .to_string();
    mgr.stop().await.unwrap();
    let lines = read_lines(&calls);
    assert!(
        lines.iter().any(|l| l.starts_with("remove-rules")
            && l.contains(&format!("--session-id {session}"))
            && !l.contains("--sweep")),
        "an unexpected engine exit still removes this manager's rules: {lines:?}"
    );
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn stop_without_a_launch_retires_the_prepared_tun_journal() {
    let dir = tempfile::TempDir::new().unwrap();
    let (helper, calls) = stub_helper(dir.path(), "");
    let mut rt = xray_on_lo(helper);
    rt.journal = Some(crate::TunJournal {
        path: dir.path().join("tun_session.v1.json"),
        session_id: "prepared-session".into(),
    });
    let mut mgr = manager_for(&dir, "exit 0\n").with_tun(Some(rt));

    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
    let lines = read_lines(&calls);
    assert!(
        lines.iter().any(|l| l.starts_with("xray-down")),
        "a cancelled start must still release its prepared TUN journal: {lines:?}"
    );
}

#[tokio::test]
async fn log_broadcast_lag_cannot_lose_the_degraded_advisory() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    // The engine readies, floods the log channel far past its 1024-slot
    // capacity while the test is not draining, and only then exits: the
    // degraded notice must survive the guaranteed log lag.
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; \
i=0; while [ $i -lt 1500 ]; do echo \"noise-$i\" >&2; i=$((i+1)); done; exit 1 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    let mut logs = mgr.subscribe_logs();
    let mut advisories = mgr.subscribe_advisories();
    let mut second = mgr.subscribe_advisories();
    mgr.start().await.unwrap();

    // Let the stub finish flooding and exiting before the first receive,
    // so the broadcast receiver is provably lagged, not merely behind.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut lagged = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "the log drain never caught up");
        match logs.try_recv() {
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Lagged(_)) => lagged = true,
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => {
                panic!("the log channel closed while the manager was up")
            }
        }
    }
    assert!(
        lagged,
        "the test must prove the log broadcast actually lagged"
    );

    let ProcessAdvisory::AntidpiDegraded { detail } = advisories
        .recv()
        .await
        .expect("the advisory channel closed without the degraded notice");
    assert!(detail.contains("code=1"), "{detail}");
    // Exactly once: nothing re-sends the notice after the drain caught up.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        advisories.try_recv().is_err(),
        "the degraded advisory must be delivered exactly once"
    );

    // Every live subscriber gets its own copy of the same notice.
    let ProcessAdvisory::AntidpiDegraded { detail } = second
        .recv()
        .await
        .expect("the second advisory channel closed without the degraded notice");
    assert!(detail.contains("code=1"), "{detail}");
    assert!(
        second.try_recv().is_err(),
        "the degraded advisory must be delivered exactly once"
    );
}

async fn sleep_engine_child(dir: &tempfile::TempDir) -> tokio::process::Child {
    let script = write_script(dir.path(), "exec sleep 30\n");
    tokio::process::Command::new(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[test]
fn wait_error_is_not_a_confirmed_exit() {
    let ok = std::process::Command::new("true").status().unwrap();
    assert!(!exit_confirmed(Some(&EngineExit {
        status: None,
        wait_failed: true,
    })));
    assert!(!exit_confirmed(Some(&EngineExit {
        status: None,
        wait_failed: false,
    })));
    assert!(exit_confirmed(Some(&EngineExit {
        status: Some(ok),
        wait_failed: false,
    })));
    assert!(!exit_confirmed(None));
}

#[tokio::test]
async fn retain_live_engine_keeps_only_a_running_engine() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n");

    // Empty exit receiver: the monitor is alive and no record arrived —
    // the engine may still run, so the handle is kept.
    let (stop_tx, _stop_rx) = mpsc::unbounded_channel::<()>();
    let (_exit_tx, exit_rx) = mpsc::unbounded_channel();
    mgr.antidpi_engine = Some(AntidpiEngine { stop_tx, exit_rx });
    mgr.retain_live_engine();
    assert!(mgr.antidpi_engine.is_some());

    // Disconnected receiver without a record: the monitor is gone and the
    // exit state is unknown — the handle is dropped so the next launch
    // re-installs and re-spawns with fresh readiness.
    mgr.antidpi_engine.as_mut().unwrap().exit_rx.close();
    mgr.retain_live_engine();
    assert!(mgr.antidpi_engine.is_none());

    // A delivered record means the exit was observed — also dropped.
    let (stop_tx, _stop_rx) = mpsc::unbounded_channel::<()>();
    let (exit_tx, exit_rx) = mpsc::unbounded_channel();
    exit_tx
        .send(EngineExit {
            status: Some(std::process::Command::new("true").status().unwrap()),
            wait_failed: false,
        })
        .unwrap();
    drop(exit_tx);
    mgr.antidpi_engine = Some(AntidpiEngine { stop_tx, exit_rx });
    mgr.retain_live_engine();
    assert!(mgr.antidpi_engine.is_none());
}

#[tokio::test]
async fn closed_stop_sender_requests_engine_exit() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n");
    let child = sleep_engine_child(&dir).await;
    let (stop_tx, mut exit_rx) = mgr.monitor_engine(child);
    // The manager dropped its handle: the monitor must treat the closed
    // sender as a shutdown request and reap the child, not poll forever.
    drop(stop_tx);
    let exit = tokio::time::timeout(Duration::from_secs(5), exit_rx.recv())
        .await
        .expect("monitor answered the shutdown request")
        .expect("monitor delivered the exit record");
    assert!(exit_confirmed(Some(&exit)), "{exit:?}");
}

#[tokio::test]
async fn unconfirmed_engine_exit_keeps_the_handle_and_fails_the_stop() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_antidpi(Some(antidpi_rt(PathBuf::from("/nonexistent"))));
    let (stop_tx, stop_rx) = mpsc::unbounded_channel::<()>();
    let (exit_tx, exit_rx) = mpsc::unbounded_channel();
    drop(stop_rx);
    drop(exit_tx);
    mgr.antidpi_engine = Some(AntidpiEngine { stop_tx, exit_rx });

    let result = mgr.teardown_antidpi().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiHelper(_))),
        "{result:?}"
    );
    assert!(
        mgr.antidpi_engine.is_some(),
        "the observation handle must survive an unconfirmed exit"
    );
}

#[tokio::test]
async fn manager_drop_leaves_engine_reaping_to_the_monitor() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n");
    let child = sleep_engine_child(&dir).await;
    let pid = child.id().expect("child pid");
    let (stop_tx, exit_rx) = mgr.monitor_engine(child);
    mgr.antidpi_engine = Some(AntidpiEngine { stop_tx, exit_rx });

    drop(mgr);
    // The manager keeps no pid and cannot signal one; the monitor sees the
    // closed stop channel and reaps the child itself.
    let deadline = Instant::now() + Duration::from_secs(10);
    while nix::sys::signal::kill(Pid::from_raw(pid as i32), None) != Err(nix::errno::Errno::ESRCH) {
        assert!(
            Instant::now() < deadline,
            "engine pid {pid} outlived the dropped manager"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn antidpi_engine_is_reused_across_a_crash_respawn() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; while :; do sleep 1; done ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1))
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    // First backend dies; the respawn loop relaunches it while the engine
    // child from the first launch is still alive.
    mgr.wait_and_handle_exit().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    let lines = read_lines(&calls);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("install-rules"))
            .count(),
        1,
        "a live engine skips re-installation: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("run ")).count(),
        1,
        "a live engine is never double-spawned: {lines:?}"
    );
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn antidpi_missing_helper_fails_the_start_without_spawning() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let marker = dir.path().join("backend-runs");
    let mut mgr = manager_for(
        &dir,
        &format!("echo backend >> {}\nexec sleep 30\n", marker.display()),
    )
    .with_antidpi(Some(antidpi_rt(PathBuf::from(
        "/nonexistent/v2ray-rs-antidpi",
    ))));

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiHelperMissing)),
        "expected the missing-helper classification, got {result:?}"
    );
    assert!(mgr.child.is_none(), "no backend may spawn");
    assert!(!marker.exists(), "no backend spawn marker");
    assert!(
        !read_lines(&calls)
            .iter()
            .any(|l| l.starts_with("install-rules")),
        "no install may run for a missing helper"
    );
}

#[tokio::test]
async fn antidpi_unprovisioned_helper_fails_the_start_before_any_spawn() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(&calls, "exit 0\n");
    // The stub is user-owned 0755 beside a tempdir: the classifier rejects
    // it (noexec on a nosuid /tmp, permissions elsewhere, enrollment on a
    // fresh group) before any spawn or install.
    // The host probe skips the nosuid mount gate by contract, so the
    // helper-access verdict under test is reached instead of the mount
    // verdict the tempdir's own mount would produce.
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(HostProbe {
            getcap: stub_getcap(dir.path()),
            helper: antidpi.clone(),
            helper_access: None,
        })
        .with_antidpi(Some(antidpi_rt(antidpi)));

    let result = mgr.start().await;
    match result {
        Err(
            ProcessError::AntiDpiHelperNoExec(_)
            | ProcessError::AntiDpiHelperPermissions(_)
            | ProcessError::AntiDpiHelperEnrollmentRequired,
        ) => {}
        other => panic!("expected an anti-DPI preflight error, got {other:?}"),
    }
    assert!(mgr.child.is_none(), "no backend may spawn");
    assert!(
        !read_lines(&calls)
            .iter()
            .any(|l| l.starts_with("install-rules")),
        "no install may run for an unprovisioned helper"
    );
}

#[tokio::test]
async fn antidpi_dead_engine_is_respawned_with_the_backend() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    // The engine self-exits immediately; the backend crashes once and then
    // stays up, so the respawn must re-install and re-spawn both.
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exit 1 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1))
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    // The respawned engine's stub logs its arguments shortly after the
    // spawn; poll for the second one.
    let deadline = Instant::now() + Duration::from_secs(2);
    let lines = loop {
        let lines = read_lines(&calls);
        if lines.iter().filter(|l| l.starts_with("run ")).count() >= 2 {
            break lines;
        }
        assert!(
            Instant::now() < deadline,
            "second engine never started: {lines:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("install-rules"))
            .count(),
        2,
        "a dead engine must be re-installed on respawn: {lines:?}"
    );
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn engine_spawns_by_absolute_path_without_touching_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let argv0 = dir.path().join("argv0");
    let body = format!(
        "echo \"$0\" >> {}\ncase \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exec sleep 30 ;; esac\nexit 0\n",
        argv0.display()
    );
    let antidpi = stub_antidpi(&calls, &body);
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi.clone())));

    let path_before = std::env::var("PATH").unwrap();
    mgr.start().await.unwrap();
    let path_after = std::env::var("PATH").unwrap();

    assert_eq!(path_before, path_after, "PATH must not be mutated");
    let recorded = std::fs::read_to_string(&argv0).unwrap();
    assert!(
        recorded.lines().all(|l| l == antidpi.to_string_lossy()),
        "argv[0] must be the resolved absolute path: {recorded:?}"
    );
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn silent_engine_is_a_startup_timeout_and_stops_the_backend() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(&calls, "case \"$1\" in run) sleep 30 ;; esac\nexit 0\n");
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiStartupTimeout { .. })),
        "expected the startup timeout, got {result:?}"
    );
    assert!(mgr.child.is_none(), "backend must be stopped");
    assert!(mgr.antidpi_engine.is_none(), "engine must be reaped");
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
}

#[tokio::test]
async fn engine_exiting_before_the_ready_line_is_a_startup_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo boom >&2; exit 7 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    let result = mgr.start().await;
    let Err(ProcessError::AntiDpiStartupExited { status, stderr }) = result else {
        panic!("expected the startup-exit error, got {result:?}");
    };
    assert_eq!(status, "code=7", "{status}");
    assert_eq!(stderr.as_deref(), Some("boom"), "{stderr:?}");
    assert!(mgr.child.is_none(), "backend must be stopped");
    assert!(mgr.antidpi_engine.is_none(), "engine must be reaped");
}

#[tokio::test]
async fn engine_dying_after_the_ready_line_is_post_readiness_death() {
    // The complete exact sentinel followed by an immediate exit proves
    // readiness: the start proceeds and the monitor records the death
    // like any later engine exit.
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=200\"; exit 3 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    assert!(
        mgr.antidpi_engine.is_some(),
        "the engine handle stays with the manager"
    );
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn a_near_miss_ready_line_is_still_a_startup_error() {
    // Only the complete exact sentinel counts: a lookalike line followed
    // by an exit must fail the start, never report readiness.
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let antidpi = stub_antidpi(
        &calls,
        "case \"$1\" in run) echo \"v2ray-rs-antidpi: ready queue=201\"; exit 7 ;; esac\nexit 0\n",
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiStartupExited { .. })),
        "expected the startup-exit error, got {result:?}"
    );
    assert!(mgr.child.is_none(), "backend must be stopped");
}

#[tokio::test]
async fn failed_removal_keeps_the_obligation_and_the_next_stop_retries_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let calls = dir.path().join("calls");
    let flag = dir.path().join("remove-failed-once");
    let antidpi = stub_antidpi(
        &calls,
        &format!(
            "case \"$1\" in\n  install-rules) exit 0 ;;\n  remove-rules) [ -f {f} ] && exit 0; touch {f}; exit 1 ;;\n  run) echo \"v2ray-rs-antidpi: ready queue=200\"; exec sleep 30 ;;\nesac\nexit 0\n",
            f = flag.display()
        ),
    );
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_host_probe(pin_ready(&antidpi))
        .with_antidpi(Some(antidpi_rt(antidpi)));

    mgr.start().await.unwrap();
    let result = mgr.stop().await;
    assert!(
        matches!(result, Err(ProcessError::AntiDpiHelper(_))),
        "a failed cleanup must surface the helper error, got {result:?}"
    );
    assert!(
        mgr.antidpi_rules_installed,
        "a failed removal must keep the cleanup obligation"
    );
    // The stop tail never reaches Stopped with a failed cleanup: Error
    // keeps the manager parked for the retry.
    match mgr.state() {
        ProcessState::Error(message) => {
            assert!(
                message.starts_with("anti-DPI cleanup failed: "),
                "state must name the failed cleanup: {message}"
            );
        }
        other => panic!("expected Error state, got {other:?}"),
    }

    // The next stop retries the outstanding obligation with no child.
    mgr.stop().await.unwrap();
    assert!(
        !mgr.antidpi_rules_installed,
        "the retry must clear the obligation"
    );
    assert_eq!(mgr.state(), ProcessState::Stopped);
    let lines = read_lines(&calls);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("remove-rules"))
            .count(),
        2,
        "the retry must run remove-rules exactly once more: {lines:?}"
    );
}

#[test]
fn has_tun_runtime_reflects_attached_runtime() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!manager_for(&dir, "exec sleep 30\n").has_tun_runtime());
    assert!(
        !manager_for(&dir, "exec sleep 30\n")
            .with_tun(None)
            .has_tun_runtime()
    );
    let (helper, _) = stub_helper(dir.path(), "exit 0\n");
    assert!(
        manager_for(&dir, "exec sleep 30\n")
            .with_tun(Some(xray_on_lo(helper)))
            .has_tun_runtime()
    );
}

fn xray_on_lo_with_journal(helper: PathBuf, dir: &std::path::Path, session_id: &str) -> TunRuntime {
    let mut rt = xray_on_lo(helper);
    rt.journal = Some(crate::tun::TunJournal {
        path: dir.join("tun_session.v1.json"),
        session_id: session_id.into(),
    });
    rt
}

#[test]
fn tun_identity_is_backend_iface_and_session() {
    let dir = tempfile::tempdir().unwrap();
    let (helper, _) = stub_helper(dir.path(), "exit 0\n");

    let mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(xray_on_lo_with_journal(
        helper.clone(),
        dir.path(),
        "s1",
    )));
    assert_eq!(mgr.tun_identity(), Some((BackendType::Xray, "lo", "s1")));

    let mut other_iface = xray_on_lo_with_journal(helper.clone(), dir.path(), "s1");
    other_iface.iface = "tun9".into();
    let mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(other_iface));
    assert_ne!(mgr.tun_identity(), Some((BackendType::Xray, "lo", "s1")));

    let mut other_session = xray_on_lo_with_journal(helper.clone(), dir.path(), "s1");
    other_session.journal.as_mut().unwrap().session_id = "s2".into();
    let mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(other_session));
    assert_ne!(mgr.tun_identity(), Some((BackendType::Xray, "lo", "s1")));

    let mut other_backend = xray_on_lo_with_journal(helper, dir.path(), "s1");
    other_backend.backend = BackendType::SingBox;
    let mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(other_backend));
    assert_ne!(mgr.tun_identity(), Some((BackendType::Xray, "lo", "s1")));

    let mut no_journal = xray_on_lo(PathBuf::from("netctl"));
    no_journal.journal = None;
    let mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(no_journal));
    assert_eq!(mgr.tun_identity(), None);

    assert_eq!(manager_for(&dir, "exec sleep 30\n").tun_identity(), None);
}

#[tokio::test]
async fn abandon_superseded_tun_drops_the_teardown_and_helper_calls() {
    // Control: a crashed candidate's armed runtime still tears its kernel
    // state down, so the empty calls file below proves the mechanism
    // rather than a broken fixture.
    let dir = tempfile::TempDir::new().unwrap();
    let (helper, calls) = stub_helper(dir.path(), "exit 0\n");
    let mut mgr = manager_for(&dir, "exit 1\n");
    mgr.set_auto_restart(false);
    mgr.start().await.unwrap();
    mgr.tun = Some(xray_on_lo_with_journal(helper, dir.path(), "s1"));
    mgr.wait_and_handle_exit().await.unwrap();
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(
        read_lines(&calls).is_empty(),
        "give-up must not call the helper"
    );
    let _ = mgr.shutdown_with(StopReason::NodeSwitch).await;
    assert_eq!(read_lines(&calls), ["xray-down"]);

    // A superseded runtime handed its kernel state to the replacement:
    // shutdown releases the process without any helper invocation.
    let armed = tempfile::TempDir::new().unwrap();
    let (helper, calls) = stub_helper(armed.path(), "exit 0\n");
    let mut mgr = manager_for(&armed, "exit 1\n");
    mgr.set_auto_restart(false);
    mgr.start().await.unwrap();
    mgr.tun = Some(xray_on_lo_with_journal(helper, armed.path(), "s1"));
    mgr.wait_and_handle_exit().await.unwrap();
    mgr.abandon_superseded_tun();
    assert!(!mgr.has_tun_runtime());
    assert_eq!(mgr.tun_identity(), None);
    let _ = mgr.shutdown_with(StopReason::NodeSwitch).await;
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert!(read_lines(&calls).is_empty());
}

#[tokio::test]
async fn abandoned_singbox_runtime_shutdown_leaves_the_journal_untouched() {
    let dir = tempfile::TempDir::new().unwrap();
    let journal = dir.path().join("tun_session.v1.json");
    std::fs::write(&journal, b"{}\n").unwrap();
    let mut rt = xray_on_lo(dir.path().join("unused-netctl"));
    rt.backend = BackendType::SingBox;
    rt.journal = Some(crate::tun::TunJournal {
        path: journal.clone(),
        session_id: "s1".into(),
    });
    let mut mgr = manager_for(&dir, "exec sleep 30\n").with_tun(Some(rt));

    mgr.abandon_superseded_tun();
    let _ = mgr.shutdown_with(StopReason::NodeSwitch).await;
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert_eq!(std::fs::read(&journal).unwrap(), b"{}\n");
}

fn crashing_backend(dir: &std::path::Path, crashes: usize) -> String {
    let runs = dir.join("runs");
    format!(
        "echo run >> {runs}\nn=$(wc -l < {runs})\n[ $n -le {crashes} ] && exit 1\nexec sleep 30\n",
        runs = runs.display()
    )
}

fn read_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

async fn wait_for_lines(path: &std::path::Path, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let lines = read_lines(path);
        if lines.len() >= n || Instant::now() >= deadline {
            return lines;
        }
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn respawn_skips_preflight() {
    let dir = tempfile::TempDir::new().unwrap();
    let fresh = ProcessManager::new(
        dir.path().join("b"),
        dir.path().join("c"),
        dir.path().join("p"),
        None,
    );
    assert_eq!(fresh.restart_delay, CRASH_RESTART_DELAY);

    let checks = dir.path().join("checks");
    let script = format!(
        "{SINGBOX_VERSION_STUB}if [ \"$1\" = check ]; then echo check >> {}; exit 0; fi\n{}",
        checks.display(),
        crashing_backend(dir.path(), 1)
    );
    let mut mgr = manager_for(&dir, &script).with_backend(BackendType::SingBox);
    mgr.restart_delay = Duration::from_millis(50);

    mgr.start().await.unwrap();
    mgr.wait_and_handle_exit().await.unwrap();

    assert_eq!(mgr.state(), ProcessState::Running);
    assert_eq!(read_lines(&checks).len(), 1);
    assert_eq!(wait_for_lines(&dir.path().join("runs"), 2).await.len(), 2);
    let _ = mgr.shutdown().await;
}

#[tokio::test]
async fn crash_respawn_records_crash_count() {
    let dir = tempfile::TempDir::new().unwrap();
    let script = format!("{VERSION_STUB}{}", crashing_backend(dir.path(), 1));
    let mut mgr = manager_for(&dir, &script).with_log_file(Some(backend_log(dir.path())));
    mgr.restart_delay = Duration::from_millis(50);
    mgr.start().await.unwrap();

    let mut rx = mgr.subscribe();
    mgr.wait_and_handle_exit().await.unwrap();

    assert_eq!(
        drain_states(&mut rx),
        [ProcessState::Starting, ProcessState::Running]
    );
    let lines = wait_for_lines(&dir.path().join("backend.log"), 3).await;
    let records: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("session backend=") || l.contains(" exit requested="))
        .collect();
    assert_eq!(records.len(), 3, "{lines:#?}");
    assert!(records[0].contains("session backend="), "{}", records[0]);
    assert!(
        records[1].contains("exit requested=false reason=crash"),
        "{}",
        records[1]
    );
    assert!(records[1].contains("code=1"), "{}", records[1]);
    assert!(records[1].contains("crashes_in_window=1"), "{}", records[1]);
    assert!(records[2].contains("session backend="), "{}", records[2]);
    let _ = mgr.shutdown().await;
}

#[tokio::test]
async fn failed_respawn_is_retried() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1));
    mgr.restart_delay = Duration::from_millis(50);
    mgr.start().await.unwrap();

    let ups = dir.path().join("ups");
    let (helper, calls) = stub_helper(
        dir.path(),
        &format!(
            "[ \"$1\" = xray-up ] || exit 0\necho up >> {ups}\n[ $(wc -l < {ups}) -le 1 ] && exit 1\nexit 0\n",
            ups = ups.display()
        ),
    );
    mgr.tun = Some(xray_on_lo(helper));

    let mut rx = mgr.subscribe();
    mgr.wait_and_handle_exit().await.unwrap();

    assert_eq!(mgr.state(), ProcessState::Running);
    assert_eq!(
        drain_states(&mut rx),
        [ProcessState::Starting, ProcessState::Running]
    );
    assert_eq!(read_lines(&calls), ["xray-up", "xray-up"]);
    let _ = mgr.shutdown().await;
}

#[tokio::test]
async fn respawn_budget_exhaustion_errors() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exit 1\n");
    mgr.restart_delay = Duration::from_millis(50);
    mgr.start().await.unwrap();
    let (helper, calls) = stub_helper(dir.path(), "");
    mgr.tun = Some(xray_on_lo(helper));

    while mgr.state() == ProcessState::Running {
        mgr.wait_and_handle_exit().await.unwrap();
    }

    match mgr.state() {
        ProcessState::Error(msg) => assert!(msg.contains("3 crashes within 60s"), "{msg}"),
        other => panic!("expected Error state, got {other:?}"),
    }
    let calls = read_lines(&calls);
    assert_eq!(calls, ["xray-up", "xray-up"]);
    assert!(!calls.iter().any(|c| c == "xray-down"));
}

#[tokio::test]
async fn stop_during_respawn_wait_reaches_stopped() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1));
    mgr.restart_delay = Duration::from_secs(5);
    mgr.start().await.unwrap();
    let (helper, calls) = stub_helper(dir.path(), "");
    mgr.tun = Some(xray_on_lo(helper));

    let waited = tokio::time::timeout(Duration::from_millis(300), mgr.wait_and_handle_exit()).await;
    assert!(waited.is_err(), "respawn should still be waiting");
    assert_eq!(mgr.state(), ProcessState::Starting);

    let _ = mgr.shutdown().await;
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert_eq!(read_lines(&calls), ["xray-down"]);
    assert_eq!(read_lines(&dir.path().join("runs")).len(), 1);
}

#[tokio::test]
async fn crash_without_restart_keeps_routing_state() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exit 1\n");
    mgr.set_auto_restart(false);
    mgr.start().await.unwrap();
    let (helper, calls) = stub_helper(dir.path(), "");
    mgr.tun = Some(xray_on_lo(helper));

    mgr.wait_and_handle_exit().await.unwrap();

    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(
        read_lines(&calls).is_empty(),
        "give-up must not call the helper"
    );
}

#[tokio::test]
async fn crash_respawn_keeps_journal_identity() {
    use v2ray_rs_core::persistence::{TunOwnership, TunPhase, TunSession};
    let dir = tempfile::TempDir::new().unwrap();
    let paths = v2ray_rs_core::persistence::AppPaths::for_profile_in(
        v2ray_rs_core::profile::AppProfile::Test,
        dir.path(),
    );
    let mut session = TunSession {
        backend: BackendType::Xray,
        iface: "lo".into(),
        ownership: Some(TunOwnership::new().unwrap()),
    };
    session.ownership.as_mut().unwrap().phase = TunPhase::Complete;
    v2ray_rs_core::persistence::save_tun_session(&paths, &session).unwrap();
    let original = session.ownership.unwrap().session_id.to_string();

    let script = format!("{VERSION_STUB}{}", crashing_backend(dir.path(), 1));
    let mut mgr = manager_for(&dir, &script);
    mgr.restart_delay = Duration::from_millis(50);
    let (helper, _) = stub_helper(dir.path(), "echo \"$1 cap_net_admin=ep\"\n");
    let probe = helper.clone();
    let mut rt = xray_on_lo(helper);
    rt.journal = Some(crate::tun::TunJournal {
        path: paths.tun_session_path(),
        session_id: original.clone(),
    });
    mgr.tun = Some(rt);
    mgr.host_probe = Some(HostProbe {
        getcap: probe.clone(),
        helper: probe,
        helper_access: Some(crate::tun::HelperAccess::Ready),
    });

    // An explicit start renews a completed journal identity...
    mgr.start().await.unwrap();
    let journal = mgr.tun.as_ref().unwrap().journal.as_ref().unwrap();
    assert_ne!(journal.session_id, original);
    let renewed = journal.session_id.clone();

    // ...while a crash respawn must keep it.
    mgr.wait_and_handle_exit().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    assert_eq!(
        mgr.tun
            .as_ref()
            .unwrap()
            .journal
            .as_ref()
            .unwrap()
            .session_id,
        renewed
    );
    mgr.shutdown().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn tun_start_refuses_without_capability() {
    // Root skips the capability probe by design, so there is no gate to hit.
    if !crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw()) {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    // /bin/sh exists but has no cap_net_admin, so the TUN gate must refuse
    // to start (whether getcap reports it missing or cannot be run).
    let mut mgr = ProcessManager::new(
        PathBuf::from("/bin/sh"),
        config,
        dir.path().join("backend.pid"),
        None,
    )
    .with_tun(Some(TunRuntime {
        backend: BackendType::Xray,
        iface: "tun0".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: dir.path().join("missing-netctl"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    }));

    let result = mgr.start_with_connection(None).await;
    assert!(
        matches!(
            result,
            Err(ProcessError::TunCapabilityMissing(_)) | Err(ProcessError::TunCapabilityProbe(_))
        ),
        "expected a TUN capability error, got {result:?}"
    );
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(mgr.child.is_none(), "no backend should have been spawned");
}

#[tokio::test]
async fn tun_start_refuses_when_helper_is_missing() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    let mut mgr = ProcessManager::new(
        PathBuf::from("/bin/sh"),
        config,
        dir.path().join("backend.pid"),
        None,
    )
    .with_tun(Some(TunRuntime {
        backend: BackendType::Xray,
        iface: "tun0".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: dir.path().join("missing-netctl"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    }))
    .with_backend(BackendType::Xray)
    .with_log_file(Some(backend_log(dir.path())))
    // The host probe skips the nosuid mount gate by contract, so the
    // helper-access verdict under test is reached instead of the mount
    // verdict the tempdir's own mount would produce.
    .with_host_probe(HostProbe {
        getcap: stub_getcap(dir.path()),
        helper: dir.path().join("missing-netctl"),
        helper_access: None,
    });

    let result = mgr.start_with_connection(None).await;
    assert!(
        matches!(result, Err(ProcessError::TunHelperMissing)),
        "expected TunHelperMissing, got {result:?}"
    );
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(mgr.child.is_none(), "no backend should have been spawned");
    let lines = wait_for_lines(&dir.path().join("backend.log"), 0).await;
    assert!(
        !lines.iter().any(|l| l.contains("session")),
        "a helper preflight failure must not write a session record: {lines:?}"
    );
}

#[tokio::test]
async fn tun_start_refuses_when_helper_not_yet_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    // A freshly granted relocated helper is group-executable only, which a
    // process that has not picked up the group yet cannot run: mode 0o644.
    let helper = dir.path().join("netctl");
    std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o644)).unwrap();

    let mut mgr = ProcessManager::new(
        PathBuf::from("/bin/sh"),
        config,
        dir.path().join("backend.pid"),
        None,
    )
    .with_tun(Some(xray_on_lo(helper.clone())))
    .with_backend(BackendType::Xray)
    .with_log_file(Some(backend_log(dir.path())))
    // The host probe skips the nosuid mount gate by contract, so the
    // helper-access verdict under test is reached instead of the mount
    // verdict the tempdir's own mount would produce.
    .with_host_probe(HostProbe {
        getcap: stub_getcap(dir.path()),
        helper,
        helper_access: None,
    });

    let result = mgr.start_with_connection(None).await;
    // A root caller is classified by a different branch: 0o644 is neither
    // root-owned-with-0750 nor an enrollment problem, so the same helper is
    // a probe failure. Either verdict must refuse the start.
    if crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw()) {
        assert!(
            matches!(result, Err(ProcessError::TunHelperPermissions(_))),
            "expected TunHelperPermissions, got {result:?}"
        );
    } else {
        assert!(
            matches!(result, Err(ProcessError::TunHelperAccessProbe { .. })),
            "expected TunHelperAccessProbe, got {result:?}"
        );
    }
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(mgr.child.is_none(), "no backend should have been spawned");
    let lines = wait_for_lines(&dir.path().join("backend.log"), 0).await;
    assert!(
        !lines.iter().any(|l| l.contains("session")),
        "a helper preflight failure must not write a session record: {lines:?}"
    );
}

#[tokio::test]
async fn singbox_tun_start_skips_helper_gates() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    // sing-box self-routes, so a missing helper must not block the start;
    // the backend capability gate still applies.
    let mut mgr = ProcessManager::new(
        PathBuf::from("/bin/sh"),
        config,
        dir.path().join("backend.pid"),
        None,
    )
    .with_tun(Some(TunRuntime {
        backend: BackendType::SingBox,
        iface: "tun0".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: dir.path().join("missing-netctl"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    }))
    .with_backend(BackendType::SingBox);

    let result = mgr.start_with_connection(None).await;
    // Root skips the capability probe by design; what must hold on every
    // host is that the missing helper never gates a sing-box start.
    if crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw()) {
        assert!(
            matches!(
                result,
                Err(ProcessError::TunCapabilityMissing(_))
                    | Err(ProcessError::TunCapabilityProbe(_))
            ),
            "expected a backend capability error, got {result:?}"
        );
    } else {
        assert!(
            !matches!(
                result,
                Err(ProcessError::TunHelperMissing)
                    | Err(ProcessError::TunHelperRelogin)
                    | Err(ProcessError::TunHelperCapabilityMissing(_))
            ),
            "the missing helper must not gate a sing-box start, got {result:?}"
        );
    }
    assert!(mgr.child.is_none(), "no backend should have been spawned");
}

#[tokio::test]
async fn stop_recovers_from_error_without_child() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    // A missing binary fails start() before any child spawns, leaving the
    // manager in Error with no child — the state stop()/shutdown() must
    // recover instead of no-oping.
    let mut mgr = ProcessManager::new(
        dir.path().join("nonexistent-binary"),
        config,
        dir.path().join("backend.pid"),
        None,
    );

    let result = mgr.start().await;
    assert!(
        matches!(result, Err(ProcessError::BinaryNotFound(_))),
        "expected BinaryNotFound, got {result:?}"
    );
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(mgr.child.is_none());

    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);

    // stop() is idempotent on an already-Stopped manager.
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

fn drain_states(rx: &mut broadcast::Receiver<ProcessEvent>) -> Vec<ProcessState> {
    let mut states = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let ProcessEvent::StateChanged { to, .. } = event {
            states.push(to);
        }
    }
    states
}

#[tokio::test]
async fn stop_from_starting_without_child_reaches_stopped() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exec sleep 30\nexec sleep 30\n"),
    )
    .with_backend(BackendType::SingBox);

    let started =
        tokio::time::timeout(Duration::from_millis(300), mgr.start_with_connection(None)).await;
    assert!(
        started.is_err(),
        "start should still be checking the config"
    );
    assert_eq!(mgr.state(), ProcessState::Starting);
    assert!(mgr.child.is_none());

    let mut rx = mgr.subscribe();
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert_eq!(
        drain_states(&mut rx),
        [ProcessState::Stopping, ProcessState::Stopped]
    );
}

#[tokio::test]
async fn stop_from_running_without_child_reaches_stopped() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n");
    mgr.start().await.unwrap();
    let mut child = mgr.child.take().unwrap();
    child.kill().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);

    let mut rx = mgr.subscribe();
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert_eq!(
        drain_states(&mut rx),
        [ProcessState::Stopping, ProcessState::Stopped]
    );
}

#[tokio::test]
async fn stop_from_error_releases_tun_state() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    let mut mgr = ProcessManager::new(
        dir.path().join("nonexistent-binary"),
        config,
        dir.path().join("backend.pid"),
        None,
    );
    assert!(mgr.start().await.is_err());
    assert!(matches!(mgr.state(), ProcessState::Error(_)));

    let calls = dir.path().join("calls");
    let helper = dir.path().join("netctl");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$1\" >> {}\n", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    mgr.tun = Some(TunRuntime {
        backend: BackendType::Xray,
        iface: "lo".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: helper,
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    });

    let mut rx = mgr.subscribe();
    mgr.stop().await.unwrap();
    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
    assert_eq!(drain_states(&mut rx), [ProcessState::Stopped]);
    // A second stop on an already-stopped manager still runs the shared
    // cleanup tail, so the route helper is asked to tear the TUN state
    // down twice: the second stop is a retry of the same obligation, not
    // a second teardown of live kernel state.
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "xray-down\nxray-down\n"
    );
}

#[tokio::test]
async fn teardown_failure_is_logged() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n");
    mgr.start().await.unwrap();

    // A stand-in for the TUN session marker: it must survive the failed
    // teardown so startup recovery can clean the kernel up.
    let marker = dir.path().join("tun_session.json");
    std::fs::write(&marker, "{}").unwrap();

    let calls = dir.path().join("calls");
    let helper = dir.path().join("netctl");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$1\" >> {}\nexit 1\n", calls.display()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    mgr.tun = Some(TunRuntime {
        backend: BackendType::Xray,
        iface: "lo".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: helper,
        bypass_uid: None,
        capture_dns: false,
        strict: true,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    });

    let err = mgr.stop().await.unwrap_err();
    assert!(err.to_string().contains("xray-down"), "{err}");
    assert!(
        matches!(mgr.state(), ProcessState::Error(msg) if msg.contains("TUN teardown failed")),
        "expected Error state, got {:?}",
        mgr.state()
    );
    assert!(
        marker.exists(),
        "session marker must survive failed teardown"
    );
    assert_eq!(std::fs::read_to_string(&calls).unwrap(), "xray-down\n");
    let logged = mgr
        .log_buffer()
        .lock()
        .unwrap()
        .last_n(10)
        .iter()
        .any(|l| l.content.contains("xray-down failed"));
    assert!(logged, "expected the teardown failure in the log buffer");

    // stop() still works from Error without a child; the failing helper
    // keeps the manager in Error instead of a clean Stopped.
    assert!(mgr.stop().await.is_err());
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(marker.exists());
}

#[test]
fn parse_semver_triple_from_xray_version_output() {
    assert_eq!(
        parse_semver_triple("Xray 26.3.27 (Xray, Penetrates Everything.) cc66b68"),
        Some((26, 3, 27))
    );
    assert_eq!(parse_semver_triple("Xray 25.12.8 (...)"), Some((25, 12, 8)));
    assert_eq!(parse_semver_triple("no version here"), None);
    assert_eq!(
        parse_semver_triple("sing-box version 1.13.0\n\nEnvironment: go1.24.1 linux/amd64"),
        Some((1, 13, 0))
    );
}

#[test]
fn xray_tun_minimum_version_comparison() {
    assert!((25, 12, 8) < XRAY_TUN_MIN_VERSION);
    assert!((26, 1, 12) < XRAY_TUN_MIN_VERSION);
    assert!((26, 1, 13) >= XRAY_TUN_MIN_VERSION);
    assert!((26, 3, 27) >= XRAY_TUN_MIN_VERSION);
}

#[test]
fn singbox_minimum_version_comparison() {
    assert!((1, 11, 0) < SINGBOX_MIN_VERSION);
    assert!((1, 12, 4) < SINGBOX_MIN_VERSION);
    assert!((1, 13, 0) >= SINGBOX_MIN_VERSION);
    assert!((1, 14, 0) >= SINGBOX_MIN_VERSION);
}

fn contains_line(mgr: &ProcessManager, content: &str) -> bool {
    mgr.log_buffer()
        .lock()
        .unwrap()
        .last_n(10)
        .iter()
        .any(|l| l.content == content)
}

#[tokio::test]
async fn singbox_start_fails_before_check_on_old_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let checked = dir.path().join("checked");
    let script = format!(
        "[ \"$1\" = version ] && {{ echo 'sing-box version 1.12.4'; exit 0; }}\n[ \"$1\" = check ] && {{ touch {}; exit 0; }}\nexec sleep 30\n",
        checked.display()
    );
    let mut mgr = manager_for(&dir, &script).with_backend(BackendType::SingBox);

    let result = mgr.start_with_connection(None).await;
    assert!(
        matches!(
            result,
            Err(ProcessError::BackendTooOld { backend: BackendType::SingBox, ref installed, ref required })
                if installed == "1.12.4" && required == "1.13.0"
        ),
        "expected BackendTooOld, got {result:?}"
    );
    match mgr.state() {
        ProcessState::Error(msg) => {
            assert!(msg.contains("1.12.4") && msg.contains("1.13.0"), "{msg}")
        }
        other => panic!("expected Error state, got {other:?}"),
    }
    assert!(!checked.exists(), "config check must not run");
    assert!(mgr.child.is_none(), "no backend should have been spawned");
}

#[tokio::test]
async fn singbox_start_proceeds_on_minimum_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exit 0\nexec sleep 30\n"),
    )
    .with_backend(BackendType::SingBox);

    mgr.start_with_connection(None).await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    assert!(
        !mgr.log_buffer()
            .lock()
            .unwrap()
            .last_n(10)
            .iter()
            .any(|l| l.content.contains("minimum-version check skipped"))
    );
    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn singbox_start_warns_on_unreadable_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(
        &dir,
        "[ \"$1\" = version ] && exit 0\n[ \"$1\" = check ] && exit 0\nexec sleep 30\n",
    )
    .with_backend(BackendType::SingBox)
    .with_log_file(Some(backend_log(dir.path())));

    mgr.start_with_connection(None).await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    let warning = "warning: could not read sing-box version; minimum-version check skipped";
    assert!(
        contains_line(&mgr, warning),
        "expected the unreadable-version warning"
    );
    let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with(&format!("notice {warning}"))),
        "{lines:?}"
    );
    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn xray_tun_start_warns_on_unreadable_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut mgr = manager_for(&dir, "[ \"$1\" = version ] && exit 0\nexit 1\n")
        .with_tun(Some(xray_on_lo(dir.path().join("missing-netctl"))))
        .with_backend(BackendType::Xray)
        .with_host_probe(HostProbe {
            getcap: PathBuf::from("/bin/true"),
            helper: dir.path().join("missing-netctl"),
            helper_access: None,
        });

    let result = mgr.start_with_connection(None).await;
    assert!(
        matches!(result, Err(ProcessError::TunHelperMissing)),
        "later TUN gates must still run, got {result:?}"
    );
    assert!(contains_line(
        &mgr,
        "warning: could not read xray version; minimum-version check skipped"
    ));
}

#[tokio::test]
async fn xray_start_without_tun_skips_version_probe() {
    let dir = tempfile::TempDir::new().unwrap();
    let probed = dir.path().join("probed");
    let script = format!(
        "[ \"$1\" = version ] && {{ touch {}; exit 0; }}\n[ \"$2\" = -test ] && exit 0\nexec sleep 30\n",
        probed.display()
    );
    let mut mgr = manager_for(&dir, &script).with_backend(BackendType::Xray);

    mgr.start_with_connection(None).await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);
    assert!(
        !probed.exists(),
        "a non-TUN xray start must not probe the version"
    );
    mgr.stop().await.unwrap();
}

#[test]
fn tun_mount_unsupported_carries_manual_setcap_wording() {
    let unsupported = crate::privilege::PrivilegeError::Unsupported {
        path: PathBuf::from("/tmp/.mount_abc/usr/bin/xray"),
        caps: crate::privilege::BACKEND_CAPS.to_string(),
    };
    let text = ProcessError::TunMountUnsupported(unsupported.to_string()).to_string();
    assert!(text.contains("ignores file capabilities"), "{text}");
    assert!(text.contains("sudo setcap"), "{text}");
}

#[tokio::test]
async fn tun_start_fails_fast_on_pre_tun_xray_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let binary = dir.path().join("fake-xray");
    std::fs::write(
        &binary,
        "#!/bin/sh\necho 'Xray 25.12.8 (Xray, Penetrates Everything.)'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    let rt = TunRuntime {
        backend: BackendType::Xray,
        iface: "tun-test".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: dir.path().join("missing-netctl"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    };
    let mut mgr = ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
        .with_tun(Some(rt))
        .with_backend(BackendType::Xray);

    let result = mgr.start().await;
    assert!(
        matches!(
            result,
            Err(ProcessError::BackendTooOld { backend: BackendType::Xray, ref installed, ref required })
                if installed == "25.12.8" && required == "26.1.13"
        ),
        "expected BackendTooOld, got {result:?}"
    );
    assert!(mgr.child.is_none());
}

#[tokio::test]
async fn tun_start_warns_on_panic_affected_xray_version() {
    let dir = tempfile::TempDir::new().unwrap();
    let binary = dir.path().join("fake-xray");
    std::fs::write(
        &binary,
        "#!/bin/sh\necho 'Xray 26.3.27 (Xray, Penetrates Everything.)'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    let rt = TunRuntime {
        backend: BackendType::Xray,
        iface: "tun-test".into(),
        addr_v4: "172.19.0.1/30".into(),
        addr_v6: None,
        helper_path: dir.path().join("missing-netctl"),
        bypass_uid: None,
        capture_dns: false,
        strict: false,
        exclude_routes: Vec::new(),
        device_ifindex: None,
        journal: None,
    };
    let mut mgr = ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
        .with_tun(Some(rt))
        .with_backend(BackendType::Xray);

    // The fake script has no capabilities, so the start still fails at the
    // CAP_NET_ADMIN gate — after the version warning has been recorded.
    let _ = mgr.start().await;

    let warned = mgr
        .log_buffer()
        .lock()
        .unwrap()
        .last_n(10)
        .iter()
        .any(|l| l.content.contains("Xray-core #6364"));
    assert!(warned, "expected a TUN panic advisory log line");
}

#[tokio::test]
async fn ready_probe_accepts_live_backend() {
    let dir = tempfile::TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut mgr =
        manager_for(&dir, "exec sleep 30\n").with_ready_probe(listener.local_addr().unwrap());

    let started = Instant::now();
    mgr.start().await.unwrap();
    assert!(
        started.elapsed() >= STABILITY_WINDOW,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(mgr.state(), ProcessState::Running);
    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn exit_before_ready_carries_last_output() {
    let dir = tempfile::TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut mgr = manager_for(&dir, "echo FATAL >&2\nexit 1\n")
        .with_ready_probe(listener.local_addr().unwrap());

    match mgr.start().await {
        Err(ProcessError::ExitedBeforeReady(msg)) => {
            assert!(msg.contains("code 1"), "{msg}");
            assert!(msg.contains("FATAL"), "{msg}");
        }
        other => panic!("expected ExitedBeforeReady, got {other:?}"),
    }
    assert!(matches!(mgr.state(), ProcessState::Error(_)));
    assert!(mgr.child.is_none());
}

#[tokio::test]
async fn ready_timeout_stops_the_backend() {
    let dir = tempfile::TempDir::new().unwrap();
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut mgr = manager_for(&dir, "exec sleep 30\n")
        .with_log_file(Some(backend_log(dir.path())))
        .with_ready_probe(addr);
    mgr.ready_timeout = Duration::from_millis(1500);
    assert!(mgr.ready_timeout > STABILITY_WINDOW);

    match mgr.start().await {
        Err(e @ ProcessError::ReadyTimeout { .. }) => {
            let text = e.to_string();
            assert!(text.contains(&addr.to_string()), "{text}");
            assert!(text.contains("1.5s"), "{text}");
        }
        other => panic!("expected ReadyTimeout, got {other:?}"),
    }
    assert!(mgr.child.is_none(), "the backend should have been reaped");

    let lines = read_lines(&dir.path().join("backend.log"));
    let exits: Vec<_> = lines.iter().filter(|l| l.contains(" exit ")).collect();
    assert_eq!(exits.len(), 1, "{lines:#?}");
    assert!(
        exits[0].contains("exit requested=false reason=start-failed"),
        "{}",
        exits[0]
    );
}

#[tokio::test]
async fn failed_cleanup_composes_both_errors_until_the_retry() {
    // Both helpers fail their first teardown verb, then succeed: the stop
    // tail must report both failures in one combined error, leave the
    // manager in Error (never Stopped), and reach Stopped on the retry.
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    let fail_once = "#!/bin/sh\n[ -f \"$0.ok\" ] && exit 0\n: > \"$0.ok\"\nexit 1\n";
    let tun_helper = dir.path().join("v2ray-rs-netctl");
    let antidpi_helper = dir.path().join("v2ray-rs-antidpi");
    for path in [&tun_helper, &antidpi_helper] {
        std::fs::write(path, fail_once).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The backend exits immediately: the fixture only needs an Error
    // state with no child, so the stop takes the release-and-retry path.
    let backend = dir.path().join("backend");
    std::fs::write(&backend, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&backend, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();

    let mut mgr = ProcessManager::new(backend, config, dir.path().join("test.pid"), None)
        .with_tun(Some(crate::tun::TunRuntime {
            backend: BackendType::Xray,
            iface: "v2rs-comp0".into(),
            addr_v4: "10.199.0.1/24".into(),
            addr_v6: None,
            helper_path: tun_helper,
            bypass_uid: None,
            capture_dns: false,
            strict: false,
            exclude_routes: Vec::new(),
            device_ifindex: None,
            journal: None,
        }))
        .with_antidpi(Some(AntiDpiRuntime {
            helper_path: antidpi_helper,
            queue: 200,
            strategies: Vec::new(),
            hostlist: None,
        }));
    let _ = mgr.start().await;
    // Mark the obligation a live session would have installed, so the
    // stop tail must remove rules, not skip them.
    mgr.antidpi_session_id = Some("compose-test".into());
    mgr.antidpi_rules_installed = true;
    assert!(matches!(mgr.state(), ProcessState::Error(_)));

    let err = mgr
        .stop()
        .await
        .expect_err("the first stop must report the failed cleanups");
    let ProcessError::CleanupFailed { tun, antidpi } = &err else {
        panic!("expected CleanupFailed, got {err:?}");
    };
    assert!(
        !tun.is_empty() && !antidpi.is_empty(),
        "tun={tun} antidpi={antidpi}"
    );
    // Stopped is never reached with a failed cleanup. The state text keeps
    // the earlier start error when the manager already sat in Error; the
    // returned error always carries the fresh combined failure.
    assert!(
        matches!(mgr.state(), ProcessState::Error(_)),
        "failed cleanup must leave Error, got {:?}",
        mgr.state()
    );

    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[test]
fn is_host_level_classifies_every_variant() {
    let cases: Vec<(ProcessError, bool)> = vec![
        (
            ProcessError::BinaryNotFound(PathBuf::from("/usr/bin/xray")),
            false,
        ),
        (
            ProcessError::ConfigMissing(PathBuf::from("/tmp/config.json")),
            false,
        ),
        (ProcessError::Spawn(std::io::Error::other("denied")), false),
        (ProcessError::Wait(std::io::Error::other("gone")), false),
        (
            ProcessError::Transition(TransitionError::Invalid {
                from: ProcessState::Running,
                to: ProcessState::Stopped,
            }),
            false,
        ),
        (
            ProcessError::TunCapabilityMissing(PathBuf::from("/usr/bin/xray")),
            true,
        ),
        (
            ProcessError::TunCapabilityProbe("getcap not found".into()),
            true,
        ),
        (
            ProcessError::TunMountUnsupported("ignores file capabilities".into()),
            true,
        ),
        (
            ProcessError::AntiDpiMountUnsupported("ignores file capabilities".into()),
            true,
        ),
        (ProcessError::TunDeviceTimeout("tun-test".into()), false),
        (ProcessError::TunHelper("helper exited 1".into()), false),
        (
            ProcessError::TunJournal("TUN journal identity changed".into()),
            true,
        ),
        (
            ProcessError::CleanupFailed {
                tun: "xray-down exited 1".into(),
                antidpi: "remove-rules exited 1".into(),
            },
            true,
        ),
        (ProcessError::TunHelperMissing, true),
        (ProcessError::TunHelperRelogin, true),
        (
            ProcessError::TunHelperCapabilityMissing(PathBuf::from(
                "/usr/local/bin/v2ray-rs-netctl",
            )),
            true,
        ),
        (ProcessError::TunHelperEnrollmentRequired, true),
        (
            ProcessError::TunHelperPermissions(PathBuf::from("/usr/local/bin/v2ray-rs-netctl")),
            true,
        ),
        (
            ProcessError::TunHelperNoExec(PathBuf::from("/usr/local/bin/v2ray-rs-netctl")),
            true,
        ),
        (
            ProcessError::TunHelperAccessProbe {
                path: PathBuf::from("/usr/local/bin/v2ray-rs-netctl"),
                detail: "probe failed".into(),
            },
            true,
        ),
        (
            ProcessError::BackendTooOld {
                backend: BackendType::Xray,
                installed: "25.12.8".into(),
                required: "26.1.13".into(),
            },
            true,
        ),
        (
            ProcessError::BackendTooOld {
                backend: BackendType::SingBox,
                installed: "1.12.4".into(),
                required: "1.13.0".into(),
            },
            true,
        ),
        (ProcessError::ConfigCheck("bad dns config".into()), false),
        (
            ProcessError::ExitedBeforeReady("process exited with code 1: FATAL".into()),
            false,
        ),
        (
            ProcessError::ReadyTimeout {
                addr: SocketAddr::from(([127, 0, 0, 1], 1080)),
                timeout: READY_TIMEOUT,
            },
            false,
        ),
    ];
    for (error, host_level) in cases {
        assert_eq!(error.is_host_level(), host_level, "{error}");
    }
}

#[test]
fn from_helper_access_maps_every_classification() {
    use crate::tun::HelperAccess;

    let helper = Path::new("/usr/local/bin/v2ray-rs-netctl");
    assert!(ProcessError::from_helper_access(helper, HelperAccess::Ready).is_none());
    assert!(matches!(
        ProcessError::from_helper_access(helper, HelperAccess::Missing),
        Some(ProcessError::TunHelperMissing)
    ));
    assert!(matches!(
        ProcessError::from_helper_access(helper, HelperAccess::EnrollmentRequired),
        Some(ProcessError::TunHelperEnrollmentRequired)
    ));
    assert!(matches!(
        ProcessError::from_helper_access(helper, HelperAccess::ReloginRequired),
        Some(ProcessError::TunHelperRelogin)
    ));
    assert!(matches!(
        ProcessError::from_helper_access(helper, HelperAccess::Permissions),
        Some(ProcessError::TunHelperPermissions(path))
        if path == helper
    ));
    assert!(matches!(
        ProcessError::from_helper_access(helper, HelperAccess::NoExec),
        Some(ProcessError::TunHelperNoExec(path))
        if path == helper
    ));
    assert!(matches!(
        ProcessError::from_helper_access(
            helper,
            HelperAccess::ProbeFailed("mount unreadable".into())
        ),
        Some(ProcessError::TunHelperAccessProbe { path, detail })
        if path == helper && detail == "mount unreadable"
    ));
}

/// The nosuid gates only fire when the host can stage the probe on a
/// mount that ignores file capabilities.
fn shm_ignores_caps() -> bool {
    !crate::privilege::file_caps_supported(Path::new("/dev/shm"))
}

fn stage_helper(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let helper = dir.join(name);
    std::fs::write(&helper, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    helper
}

#[tokio::test]
async fn nosuid_antidpi_helper_fails_preflight_before_spawn() {
    if !shm_ignores_caps() {
        eprintln!("skipping: /dev/shm honors file capabilities");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let shm = tempfile::TempDir::new_in("/dev/shm").unwrap();
    let staged = stage_helper(shm.path(), "v2ray-rs-antidpi", "exit 0\n");
    let mut mgr = manager_for(&dir, "exit 0\n")
        .with_backend(BackendType::SingBox)
        .with_antidpi(Some(antidpi_rt(staged.clone())));

    let error = mgr.start().await.unwrap_err();
    let ProcessError::AntiDpiMountUnsupported(text) = &error else {
        panic!("expected AntiDpiMountUnsupported, got {error:?}");
    };
    assert!(text.contains(staged.to_string_lossy().as_ref()), "{text}");
    assert!(text.contains(crate::privilege::ANTIDPI_CAPS), "{text}");
    assert!(error.is_host_level());
    assert!(!dir.path().join("backend.pid").exists());
}

#[tokio::test]
async fn nosuid_route_helper_fails_preflight_before_spawn() {
    if !shm_ignores_caps() {
        eprintln!("skipping: /dev/shm honors file capabilities");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    if !crate::privilege::file_caps_supported(dir.path()) {
        eprintln!("skipping: the temp dir is on a nosuid mount");
        return;
    }
    let shm = tempfile::TempDir::new_in("/dev/shm").unwrap();
    let staged = stage_helper(shm.path(), "v2ray-rs-netctl", "exit 1\n");
    let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exit 1\n"))
        .with_backend(BackendType::Xray)
        .with_tun(Some(xray_on_lo(staged.clone())));

    let error = mgr.start().await.unwrap_err();
    let ProcessError::TunMountUnsupported(text) = &error else {
        panic!("expected TunMountUnsupported, got {error:?}");
    };
    assert!(text.contains(staged.to_string_lossy().as_ref()), "{text}");
    assert!(text.contains(crate::privilege::HELPER_CAPS), "{text}");
}

#[tokio::test]
async fn symlinked_backend_on_nosuid_target_fails_preflight() {
    let exe_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if !shm_ignores_caps() || !crate::privilege::file_caps_supported(&exe_dir) {
        eprintln!("skipping: the test binary directory cannot host the symlink probe");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let shm = tempfile::TempDir::new_in("/dev/shm").unwrap();
    let target = stage_helper(shm.path(), "xray", &format!("{VERSION_STUB}exit 1\n"));
    let link = exe_dir.join(format!("v2ray-rs-nosuid-{}.xray", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    let mut mgr = ProcessManager::new(link.clone(), config, dir.path().join("backend.pid"), None)
        .with_backend(BackendType::Xray)
        .with_tun(Some(xray_on_lo(stage_helper(
            dir.path(),
            "v2ray-rs-netctl",
            "exit 1\n",
        ))));

    let error = mgr.start().await.unwrap_err();
    std::fs::remove_file(&link).unwrap();
    let ProcessError::TunMountUnsupported(text) = &error else {
        panic!("expected TunMountUnsupported, got {error:?}");
    };
    assert!(text.contains(link.to_string_lossy().as_ref()), "{text}");
    std::fs::remove_file(&link).ok();
}
