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
    #[error("{0}")]
    Transition(#[from] TransitionError),
}

pub struct ProcessManager {
    state: StateManager,
    log_buffer: Arc<Mutex<LogBuffer>>,
    pid_file: PidFile,
    child: Option<Child>,
    binary_path: PathBuf,
    config_path: PathBuf,
    crash_times: Vec<Instant>,
    auto_restart: bool,
    log_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl ProcessManager {
    pub fn new(binary_path: PathBuf, config_path: PathBuf, pid_path: PathBuf) -> Self {
        Self {
            state: StateManager::new(),
            log_buffer: Arc::new(Mutex::new(LogBuffer::new())),
            pid_file: PidFile::new(pid_path),
            child: None,
            binary_path,
            config_path,
            crash_times: Vec::new(),
            auto_restart: true,
            log_handles: Vec::new(),
        }
    }

    pub fn state(&self) -> ProcessState {
        self.state.state()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe()
    }

    pub fn log_buffer(&self) -> &Arc<Mutex<LogBuffer>> {
        &self.log_buffer
    }

    pub fn set_auto_restart(&mut self, enabled: bool) {
        self.auto_restart = enabled;
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        if !self.binary_path.exists() {
            return Err(ProcessError::BinaryNotFound(self.binary_path.clone()));
        }
        if !self.config_path.exists() {
            return Err(ProcessError::ConfigMissing(self.config_path.clone()));
        }

        self.state.transition(ProcessState::Starting)?;

        match self.spawn_process().await {
            Ok(()) => {
                self.state.transition(ProcessState::Running)?;
                Ok(())
            }
            Err(e) => {
                let _ = self.state.transition(ProcessState::Error(e.to_string()));
                Err(e)
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), ProcessError> {
        if self.child.is_none() {
            return Ok(());
        }

        self.state.transition(ProcessState::Stopping)?;
        self.graceful_stop().await;
        self.state.transition(ProcessState::Stopped)?;
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

    pub async fn wait_and_handle_exit(&mut self) -> Option<i32> {
        let child = self.child.as_mut()?;
        let status = child.wait().await.ok()?;
        let exit_code = status.code();

        self.child = None;
        self.pid_file.remove().ok();

        self.state.emit(ProcessEvent::ProcessExited { exit_code });

        if self.state.state() == ProcessState::Running {
            self.handle_unexpected_exit(exit_code).await;
        }

        exit_code
    }

    async fn spawn_process(&mut self) -> Result<(), ProcessError> {
        let mut child = Command::new(&self.binary_path)
            .arg("run")
            .arg("-c")
            .arg(&self.config_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        if let Some(pid) = child.id() {
            self.pid_file.write(pid).ok();
        }

        self.capture_output(&mut child);
        self.child = Some(child);
        Ok(())
    }

    fn capture_output(&mut self, child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            let tx = self.state.sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let log_line = LogLine::stdout(&line);
                    if let Ok(mut buf) = buffer.lock() {
                        buf.push(log_line.clone());
                    }
                    let _ = tx.send(ProcessEvent::LogLine(log_line));
                }
            }));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = self.state.sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let log_line = LogLine::stderr(&line);
                    if let Ok(mut buf) = buffer.lock() {
                        buf.push(log_line.clone());
                    }
                    let _ = tx.send(ProcessEvent::LogLine(log_line));
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

        for handle in self.log_handles.drain(..) {
            handle.abort();
        }

        self.child = None;
    }

    async fn handle_unexpected_exit(&mut self, exit_code: Option<i32>) {
        let msg = match exit_code {
            Some(code) => format!("process exited with code {code}"),
            None => "process killed by signal".into(),
        };

        let is_signal_exit =
            exit_code.is_none() || matches!(exit_code, Some(130) | Some(137) | Some(143));

        if !is_signal_exit {
            self.crash_times.push(Instant::now());
            self.crash_times.retain(|t| t.elapsed() < CRASH_WINDOW);

            if self.crash_times.len() >= MAX_CRASHES {
                let _ = self.state.transition(ProcessState::Error(format!(
                    "{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}"
                )));
                return;
            }
        }

        if !self.auto_restart || is_signal_exit {
            let _ = self.state.transition(if is_signal_exit {
                ProcessState::Stopped
            } else {
                ProcessState::Error(msg)
            });
            return;
        }

        let _ = self.state.transition(ProcessState::Stopped);
        sleep(CRASH_RESTART_DELAY).await;

        if let Err(e) = self.start().await {
            let _ = self
                .state
                .transition(ProcessState::Error(format!("restart failed: {e}")));
        }
    }
}
