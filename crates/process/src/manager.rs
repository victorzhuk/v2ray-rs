use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::time::sleep;

use crate::log_buffer::{LogBuffer, LogLine};
use crate::pid::PidFile;
use crate::state::{ProcessEvent, ProcessState, StateManager, TransitionError};
use crate::tun::{self, TunRuntime};
use v2ray_rs_core::models::ConnectionMetadata;

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);
const MAX_CRASHES: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("binary not found: {0}")]
    BinaryNotFound(PathBuf),
    #[error("config file missing: {0}")]
    ConfigMissing(PathBuf),
    #[error("spawn process: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("wait process: {0}")]
    Wait(std::io::Error),
    #[error("{0}")]
    Transition(#[from] TransitionError),
    #[error("backend {0} lacks CAP_NET_ADMIN required for TUN mode; grant TUN privileges first")]
    TunCapabilityMissing(PathBuf),
    #[error("could not verify TUN capabilities: {0}")]
    TunCapabilityProbe(String),
    #[error("TUN device {0} did not appear")]
    TunDeviceTimeout(String),
    #[error("TUN route helper failed: {0}")]
    TunHelper(String),
}

pub struct ProcessManager {
    state: StateManager,
    log_buffer: Arc<Mutex<LogBuffer>>,
    pid_file: PidFile,
    child: Option<Child>,
    binary_path: PathBuf,
    config_path: PathBuf,
    geodata_dir: Option<PathBuf>,
    crash_times: Vec<Instant>,
    auto_restart: bool,
    log_handles: Vec<tokio::task::JoinHandle<()>>,
    current_connection: Option<ConnectionMetadata>,
    tun: Option<TunRuntime>,
}

impl ProcessManager {
    pub fn new(
        binary_path: PathBuf,
        config_path: PathBuf,
        pid_path: PathBuf,
        geodata_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            state: StateManager::new(),
            log_buffer: Arc::new(Mutex::new(LogBuffer::new())),
            pid_file: PidFile::new(pid_path),
            child: None,
            binary_path,
            config_path,
            geodata_dir,
            crash_times: Vec::new(),
            auto_restart: true,
            log_handles: Vec::new(),
            current_connection: None,
            tun: None,
        }
    }

    /// Attaches TUN runtime details so start/stop become TUN-aware.
    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {
        self.tun = tun;
        self
    }

    pub fn state(&self) -> ProcessState {
        self.state.state()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe()
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe_logs()
    }

    pub fn log_buffer(&self) -> &Arc<Mutex<LogBuffer>> {
        &self.log_buffer
    }

    pub fn set_auto_restart(&mut self, enabled: bool) {
        self.auto_restart = enabled;
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        self.start_with_connection(None).await
    }

    pub async fn start_with_connection(
        &mut self,
        connection: Option<ConnectionMetadata>,
    ) -> Result<(), ProcessError> {
        if !self.binary_path.exists() {
            self.state
                .transition(ProcessState::Starting, connection.clone())?;
            let error = ProcessError::BinaryNotFound(self.binary_path.clone());
            let _ = self
                .state
                .transition(ProcessState::Error(error.to_string()), None);
            return Err(error);
        }
        if !self.config_path.exists() {
            self.state
                .transition(ProcessState::Starting, connection.clone())?;
            let error = ProcessError::ConfigMissing(self.config_path.clone());
            let _ = self
                .state
                .transition(ProcessState::Error(error.to_string()), None);
            return Err(error);
        }

        if self.tun.is_some() {
            let binary = self.binary_path.clone();
            let probe =
                tokio::task::spawn_blocking(move || crate::privilege::has_net_admin(&binary)).await;
            let cap = match probe {
                Ok(inner) => inner,
                Err(join) => Err(crate::privilege::PrivilegeError::Probe(
                    self.binary_path.clone(),
                    join.to_string(),
                )),
            };
            match cap {
                Ok(true) => {}
                other => {
                    self.state
                        .transition(ProcessState::Starting, connection.clone())?;
                    let error = match other {
                        Ok(false) => ProcessError::TunCapabilityMissing(self.binary_path.clone()),
                        Err(e) => ProcessError::TunCapabilityProbe(e.to_string()),
                        Ok(true) => unreachable!(),
                    };
                    let _ = self
                        .state
                        .transition(ProcessState::Error(error.to_string()), None);
                    return Err(error);
                }
            }
        }

        if connection.is_some() {
            self.current_connection = connection.clone();
        }

        self.state
            .transition(ProcessState::Starting, connection.clone())?;

        match self.spawn_process().await {
            Ok(()) => {
                self.state.transition(ProcessState::Running, connection)?;
                Ok(())
            }
            Err(e) => {
                let _ = self
                    .state
                    .transition(ProcessState::Error(e.to_string()), None);
                Err(e)
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), ProcessError> {
        if self.child.is_none() {
            return Ok(());
        }

        self.state.transition(ProcessState::Stopping, None)?;
        self.graceful_stop().await;

        // SIGTERM already let xray close its TUN fd (the kernel drops the
        // device-scoped routes); run the helper teardown as a safeguard.
        self.teardown_tun().await;

        self.state.transition(ProcessState::Stopped, None)?;
        self.pid_file.remove().ok();
        Ok(())
    }

    pub async fn restart(&mut self) -> Result<(), ProcessError> {
        if self.child.is_some() {
            self.stop().await?;
        }
        self.start().await
    }

    pub async fn shutdown(&mut self) {
        if self.child.is_some() {
            self.auto_restart = false;
            let _ = self.stop().await;
        }
    }

    pub fn check_orphaned(&self) -> std::io::Result<bool> {
        self.pid_file.check_and_kill_orphaned()
    }

    pub async fn wait_and_handle_exit(&mut self) -> Result<Option<i32>, ProcessError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let status = match child.wait().await {
            Ok(status) => status,
            Err(err) => {
                self.cleanup_after_exit();
                let error = ProcessError::Wait(err);
                let _ = self
                    .state
                    .transition(ProcessState::Error(error.to_string()), None);
                return Err(error);
            }
        };
        let exit_code = status.code();

        self.cleanup_after_exit();

        self.state.emit(ProcessEvent::ProcessExited { exit_code });

        if self.state.state() == ProcessState::Running {
            self.handle_unexpected_exit(exit_code).await;
        }

        Ok(exit_code)
    }

    async fn spawn_process(&mut self) -> Result<(), ProcessError> {
        let mut child = self.try_spawn().await?;

        if let Some(pid) = child.id()
            && let Err(err) = self
                .pid_file
                .write(pid, &self.binary_path, &self.config_path)
        {
            log::warn!("failed to write pid ownership record: {err}");
        }

        self.capture_output(&mut child);
        self.child = Some(child);

        // xray creates the device but does not program routes on Linux: wait for
        // the device, then drive the privileged helper. sing-box self-routes.
        if let Some(rt) = self.tun.clone()
            && rt.needs_helper()
        {
            if !tun::wait_for_device(&rt.iface, tun::DEVICE_TIMEOUT).await {
                self.graceful_stop().await;
                self.teardown_tun().await;
                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));
            }
            match tun::xray_up(&rt).await {
                Ok(true) => {}
                Ok(false) => {
                    self.graceful_stop().await;
                    self.teardown_tun().await;
                    return Err(ProcessError::TunHelper("xray-up reported failure".into()));
                }
                Err(e) => {
                    self.graceful_stop().await;
                    self.teardown_tun().await;
                    return Err(ProcessError::TunHelper(e.to_string()));
                }
            }
        }

        Ok(())
    }

    // Retry on ETXTBSY which can occur on overlayfs (Docker containers)
    // when a binary is written and immediately executed
    async fn try_spawn(&self) -> Result<Child, std::io::Error> {
        let binary_path = self.binary_path.clone();
        let config_path = self.config_path.clone();
        let geodata_dir = self.geodata_dir.clone();
        crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary_path);
            cmd.arg("run")
                .arg("-c")
                .arg(&config_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            if let Some(dir) = &geodata_dir {
                cmd.env("V2RAY_LOCATION_ASSET", dir);
                cmd.env("XRAY_LOCATION_ASSET", dir);
            }
            cmd
        })
        .await
    }

    fn capture_output(&mut self, child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let log_line = LogLine::stdout(&line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let log_line = LogLine::stderr(&line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }
    }

    async fn graceful_stop(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };

        if let Some(pid) = child.id() {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        }

        let wait_result = tokio::time::timeout(STOP_TIMEOUT, child.wait()).await;

        if wait_result.is_err() {
            child.kill().await.ok();
            child.wait().await.ok();
        }

        self.cleanup_after_exit();
    }

    async fn handle_unexpected_exit(&mut self, exit_code: Option<i32>) {
        let msg = match exit_code {
            Some(code) => format!("process exited with code {code}"),
            None => "process killed by signal".into(),
        };

        // Reaching here means the backend died while we still expected it
        // Running: stop() moves state to Stopping before killing, so a requested
        // stop never lands here. Every exit at this point is an unrequested
        // crash — including a signal death (OOM, segfault, external kill), which
        // reports exit_code == None on Unix and must not be mistaken for a clean
        // stop.
        self.crash_times.push(Instant::now());
        self.crash_times.retain(|t| t.elapsed() < CRASH_WINDOW);

        // Roll back any TUN routing state the dead backend left behind before we
        // relaunch or give up: xray_up installs host-wide policy rules that
        // outlive the device and is not idempotent across a dirty restart.
        self.teardown_tun().await;

        if !self.auto_restart {
            let _ = self.state.transition(ProcessState::Error(msg), None);
            return;
        }

        if self.crash_times.len() >= MAX_CRASHES {
            let _ = self.state.transition(
                ProcessState::Error(format!(
                    "{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}"
                )),
                None,
            );
            return;
        }

        sleep(CRASH_RESTART_DELAY * self.crash_times.len() as u32).await;

        if let Err(e) = self
            .start_with_connection(self.current_connection.clone())
            .await
        {
            let _ = self
                .state
                .transition(ProcessState::Error(format!("restart failed: {e}")), None);
        }
    }

    async fn teardown_tun(&self) {
        if let Some(rt) = self.tun.clone()
            && rt.needs_helper()
        {
            let _ = tun::xray_down(&rt).await;
        }
    }

    fn cleanup_after_exit(&mut self) {
        self.child = None;
        self.pid_file.remove().ok();
        for handle in self.log_handles.drain(..) {
            handle.abort();
        }
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            if let Some(pid) = child.id() {
                log::warn!("ProcessManager dropped with live child (pid {pid}); sending SIGKILL");
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
                let _ = child.try_wait();
            }
            self.pid_file.remove().ok();
            for handle in self.log_handles.drain(..) {
                handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tun::TunRuntime;
    use v2ray_rs_core::models::BackendType;

    #[tokio::test]
    async fn tun_start_refuses_without_capability() {
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
            helper_path: PathBuf::from("v2ray-rs-netctl"),
            bypass_uid: None,
        }));

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(
                result,
                Err(ProcessError::TunCapabilityMissing(_))
                    | Err(ProcessError::TunCapabilityProbe(_))
            ),
            "expected a TUN capability error, got {result:?}"
        );
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none(), "no backend should have been spawned");
    }
}
