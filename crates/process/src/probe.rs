use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep};

use crate::spawn::spawn_with_etxtbsy_retry;

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("failed to spawn probe backend: {0}")]
    Spawn(std::io::Error),
    #[error("probe API did not become reachable within {0:?}")]
    ApiNotReady(Duration),
    #[error("probe backend exited early: {0}")]
    ExitedEarly(String),
    #[error("probe backend was killed")]
    Killed,
}

/// An isolated, short-lived backend instance used solely for Real Delay probes.
///
/// Unlike [`crate::ProcessManager`], `ProbeRunner` does not publish process
/// state events, does not auto-restart on crash, and does not write a PID file.
/// On `Drop` it sends `SIGKILL` to any surviving child to prevent orphans.
pub struct ProbeRunner {
    binary: PathBuf,
    config_path: PathBuf,
    api_port: u16,
    child: Option<Child>,
}

impl ProbeRunner {
    pub fn new(binary: PathBuf, config_path: PathBuf, api_port: u16) -> Self {
        Self {
            binary,
            config_path,
            api_port,
            child: None,
        }
    }

    #[must_use]
    pub fn api_port(&self) -> u16 {
        self.api_port
    }

    /// Spawns the ephemeral backend with the generated probe config. Sets
    /// `PR_SET_PDEATHSIG` on Linux so the kernel terminates the probe if this
    /// process (v2ray-rs) dies.
    pub async fn start(&mut self) -> Result<(), ProbeError> {
        let binary = self.binary.clone();
        let config_path = self.config_path.clone();
        let child = spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary);
            cmd.arg("run")
                .arg("-c")
                .arg(&config_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // SAFETY: set_pdeathsig only sets a process attribute and does not
            // allocate or touch shared state in the forked child.
            unsafe {
                cmd.as_std_mut().pre_exec(|| {
                    nix::sys::prctl::set_pdeathsig(Signal::SIGTERM)
                        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
                    Ok(())
                });
            }
            cmd
        })
        .await
        .map_err(ProbeError::Spawn)?;
        self.child = Some(child);
        Ok(())
    }

    /// Polls the loopback API port until it accepts a connection or `timeout`
    /// elapses. If the child exits during the wait, returns `ExitedEarly` with
    /// captured stderr.
    pub async fn wait_ready(&mut self, timeout: Duration) -> Result<(), ProbeError> {
        let deadline = Instant::now() + timeout;
        let mut backoff = Duration::from_millis(25);
        loop {
            if let Some(child) = self.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                let stderr = self.drain_stderr().await;
                return Err(ProbeError::ExitedEarly(format!(
                    "exit status {status}: {stderr}"
                )));
            }

            if TcpStream::connect(("127.0.0.1", self.api_port))
                .await
                .is_ok()
            {
                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(ProbeError::ApiNotReady(timeout));
            }

            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_millis(200));
        }
    }

    /// Stops the ephemeral backend: SIGTERM, then SIGKILL after 5 seconds.
    pub async fn stop(&mut self) -> Result<(), ProbeError> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if let Some(pid) = child.id() {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        }
        if tokio::time::timeout(STOP_TIMEOUT, child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    async fn drain_stderr(&mut self) -> String {
        if let Some(child) = self.child.as_mut()
            && let Some(mut stderr) = child.stderr.take()
        {
            let mut buf = Vec::new();
            let _ = tokio::time::timeout(Duration::from_millis(500), stderr.read_to_end(&mut buf))
                .await;
            return String::from_utf8_lossy(&buf).trim().to_string();
        }
        String::new()
    }
}

impl Drop for ProbeRunner {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Some(pid) = child.id() {
                log::warn!("ProbeRunner dropped with live child (pid {pid}); sending SIGKILL");
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            }
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // A fake backend that ignores its args and stays alive without ever opening
    // the API port.
    fn fake_sleeper_binary(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("fake-backend");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\nexec sleep 30").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn pid_alive(pid: i32) -> bool {
        kill(Pid::from_raw(pid), None).is_ok()
    }

    #[tokio::test]
    async fn start_wait_ready_times_out_then_stop() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_sleeper_binary(dir.path());
        let config = dir.path().join("probe.json");
        std::fs::write(&config, "{}").unwrap();

        // Bind a port then drop it to get an unused loopback port.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut runner = ProbeRunner::new(binary, config, port);
        runner.start().await.unwrap();
        let pid = {
            // child is private; verify liveness via stop afterwards
            assert!(runner.child.is_some());
            runner.child.as_ref().unwrap().id().unwrap() as i32
        };
        assert!(pid_alive(pid));

        let result = runner.wait_ready(Duration::from_millis(300)).await;
        assert!(matches!(result, Err(ProbeError::ApiNotReady(_))));

        runner.stop().await.unwrap();
        // Give the kernel a moment to reap.
        sleep(Duration::from_millis(100)).await;
        assert!(!pid_alive(pid));
    }

    #[tokio::test]
    async fn drop_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_sleeper_binary(dir.path());
        let config = dir.path().join("probe.json");
        std::fs::write(&config, "{}").unwrap();

        let mut runner = ProbeRunner::new(binary, config, 1);
        runner.start().await.unwrap();
        let pid = runner.child.as_ref().unwrap().id().unwrap() as i32;
        assert!(pid_alive(pid));

        drop(runner);
        sleep(Duration::from_millis(150)).await;
        assert!(!pid_alive(pid));
    }
}
