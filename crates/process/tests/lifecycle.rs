use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tempfile::TempDir;
use v2ray_rs_process::{ProcessManager, ProcessState};

fn setup_dir() -> TempDir {
    TempDir::new().unwrap()
}

fn create_script(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    let mut f = File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.sync_all().unwrap();
    drop(f);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn create_config(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("config.json");
    fs::write(&path, "{}").unwrap();
    path
}

fn pid_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test.pid")
}

#[tokio::test]
async fn start_and_stop() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nwhile true; do sleep 1; done\n");
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));

    assert_eq!(mgr.state(), ProcessState::Stopped);

    mgr.start().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);

    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn restart_transitions() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nwhile true; do sleep 1; done\n");
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));

    mgr.start().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);

    mgr.restart().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Running);

    mgr.stop().await.unwrap();
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn binary_not_found() {
    let dir = setup_dir();
    let config = create_config(&dir);
    let binary = dir.path().join("nonexistent");

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    let result = mgr.start().await;

    assert!(result.is_err());
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn config_missing() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nsleep 60\n");
    let config = dir.path().join("nonexistent.json");

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    let result = mgr.start().await;

    assert!(result.is_err());
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn pid_file_written_on_start() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nwhile true; do sleep 1; done\n");
    let config = create_config(&dir);
    let pid = pid_path(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid.clone());
    mgr.start().await.unwrap();

    assert!(pid.exists());

    mgr.stop().await.unwrap();

    assert!(!pid.exists());
}

#[tokio::test]
async fn log_capture() {
    let dir = setup_dir();
    let binary = create_script(
        &dir,
        "backend",
        "#!/bin/sh\necho 'hello stdout'\necho 'hello stderr' >&2\nsleep 60\n",
    );
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    mgr.start().await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    {
        let buf = mgr.log_buffer().lock().unwrap();
        let lines = buf.last_n(10);
        assert!(!lines.is_empty(), "should have captured log lines");
    }

    mgr.stop().await.unwrap();
}

#[tokio::test]
async fn shutdown_disables_auto_restart() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nwhile true; do sleep 1; done\n");
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    mgr.start().await.unwrap();

    mgr.shutdown().await;
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn stop_when_already_stopped() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nsleep 60\n");
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    let result = mgr.stop().await;

    assert!(result.is_ok());
    assert_eq!(mgr.state(), ProcessState::Stopped);
}

#[tokio::test]
async fn crash_detection() {
    let dir = setup_dir();
    let binary = create_script(&dir, "backend", "#!/bin/sh\nexit 1\n");
    let config = create_config(&dir);

    let mut mgr = ProcessManager::new(binary, config, pid_path(&dir));
    mgr.set_auto_restart(false);
    mgr.start().await.unwrap();

    let exit_code = mgr.wait_and_handle_exit().await;
    assert_eq!(exit_code, Some(1));

    match mgr.state() {
        ProcessState::Error(_) => {}
        other => panic!("expected Error state, got {other:?}"),
    }
}
