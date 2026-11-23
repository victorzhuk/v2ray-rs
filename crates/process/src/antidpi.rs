//! Anti-DPI engine lifecycle: binary resolution, argument construction, and
//! the one-shot rule invocations. The engine is an external binary
//! (`v2ray-rs-antidpi`); this module never links it. See
//! `docs/antidpi-registry.md`.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::process::Child;

use crate::tun::{HELPER_TIMEOUT, HelperRun, run_helper};

/// Helper binary resolved with the same bundled → relocated → `$PATH` ordering
/// as the route helper, so AppImage installs land on the relocated copy and
/// distribution installs on `/usr/bin`.
pub const ANTIDPI_BIN: &str = "v2ray-rs-antidpi";

/// How long the stop path waits after SIGTERM before SIGKILL on the engine
/// child. Mirrors the backend's own stop budget (`manager::STOP_TIMEOUT`).
pub(crate) const ANTIDPI_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the start path waits for the engine's ready sentinel after the
/// spawn. The engine prints it once its NFQUEUE bind succeeded, so it is the
/// earliest point at which the session may report Connected.
pub(crate) const ANTIDPI_READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything the process manager needs to drive the anti-DPI engine for a
/// connection.
#[derive(Debug, Clone)]
pub struct AntiDpiRuntime {
    pub helper_path: PathBuf,
    pub queue: u16,
    /// Strategy chains passed one argument per string. An empty list omits the
    /// `--strategy` flags entirely, so the helper falls back to its built-in
    /// default chains.
    pub strategies: Vec<String>,
    pub hostlist: Option<PathBuf>,
}

/// Resolves the anti-DPI helper for execution.
pub fn antidpi_path() -> PathBuf {
    crate::tun::resolve_bin(ANTIDPI_BIN)
}

/// Absolute path to the anti-DPI helper beside the running executable, if
/// present. This is the source a privileged grant copies from, never an
/// execution target.
pub fn bundled_antidpi_path() -> Option<PathBuf> {
    crate::tun::bundled_bin(ANTIDPI_BIN)
}

/// Absolute path the grant installs the anti-DPI helper to.
pub fn relocated_antidpi_path() -> PathBuf {
    Path::new(crate::tun::RELOCATE_DIR).join(ANTIDPI_BIN)
}

/// Arguments for the one-shot `install-rules` invocation that must run before
/// any backend traffic flows. The helper validates every strategy chain and
/// the hostlist before touching the kernel, so passing them here moves that
/// validation ahead of the backend spawn.
pub(crate) fn antidpi_install_args(
    rt: &AntiDpiRuntime,
    owner: Option<&str>,
    session_id: &str,
) -> Vec<String> {
    let mut args = vec![
        "install-rules".to_string(),
        "--queue".to_string(),
        rt.queue.to_string(),
    ];
    for strategy in &rt.strategies {
        args.extend(["--strategy".to_string(), strategy.clone()]);
    }
    if let Some(hosts) = &rt.hostlist {
        args.extend(["--hosts".to_string(), hosts.display().to_string()]);
    }
    if let Some(owner) = owner {
        args.extend(["--owner".to_string(), owner.to_string()]);
    }
    args.extend(["--session-id".to_string(), session_id.to_string()]);
    args
}

/// Arguments for the owner-qualified `remove-rules` invocation. The owner
/// pins the removal to the mutations this process recorded and the session id
/// to this manager's journal, so a foreign session's rules are never deleted.
/// Leftovers without a journal are offline recovery's job (`--sweep`), never
/// an app call.
pub(crate) fn antidpi_remove_args(owner: Option<&str>, session_id: &str) -> Vec<String> {
    let mut args = vec!["remove-rules".to_string()];
    if let Some(owner) = owner {
        args.extend(["--owner".to_string(), owner.to_string()]);
    }
    args.extend(["--session-id".to_string(), session_id.to_string()]);
    args
}

/// Exact stdout line the long-lived engine prints once its NFQUEUE bind
/// succeeded; the manager gates startup success on it.
pub(crate) fn ready_sentinel(queue: u16) -> String {
    format!("v2ray-rs-antidpi: ready queue={queue}")
}

/// Arguments for the long-lived engine child. `--no-fw` is the contract with
/// the helper: the app owns rule installation, so the engine never touches the
/// firewall itself and stop stays deterministic.
///
/// Every element — including each strategy chain and the hostlist path — is
/// pushed as one entry of the argv vector handed to `Command::args()`. Nothing
/// is ever interpolated into a shell string: a strategy containing quotes or
/// metacharacters reaches the helper as a single argument, exactly like the
/// TUN exclusion CIDRs in `tun::xray_up_args`.
pub(crate) fn antidpi_run_args(rt: &AntiDpiRuntime, session_id: &str) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--queue".to_string(),
        rt.queue.to_string(),
        "--no-fw".to_string(),
    ];
    for strategy in &rt.strategies {
        args.extend(["--strategy".to_string(), strategy.clone()]);
    }
    if let Some(hosts) = &rt.hostlist {
        args.extend(["--hosts".to_string(), hosts.display().to_string()]);
    }
    args.extend(["--session-id".to_string(), session_id.to_string()]);
    args
}

/// Runs `antidpi install-rules --queue N --strategy ... [--hosts ...]
/// --owner ... --session-id ...`. Bounded by `HELPER_TIMEOUT` like every
/// other helper invocation.
pub(crate) async fn antidpi_install_rules(rt: &AntiDpiRuntime, session_id: &str) -> HelperRun {
    run_helper(
        &rt.helper_path,
        &antidpi_install_args(rt, crate::pid::owner_identity().as_deref(), session_id),
        HELPER_TIMEOUT,
    )
    .await
}

/// Runs `antidpi remove-rules --owner ... --session-id ...`.
pub(crate) async fn antidpi_remove_rules(rt: &AntiDpiRuntime, session_id: &str) -> HelperRun {
    run_helper(
        &rt.helper_path,
        &antidpi_remove_args(crate::pid::owner_identity().as_deref(), session_id),
        HELPER_TIMEOUT,
    )
    .await
}

/// Outcome of the engine child's termination or self-exit. Cleanup is always
/// the owner- and session-qualified removal, so the death mode changes
/// nothing about it.
#[derive(Debug, Clone)]
pub struct EngineExit {
    pub status: Option<ExitStatus>,
    /// A wait failure is not an exit status: the monitor could not observe
    /// the child, so the exit is unconfirmed even when the channel delivers.
    pub wait_failed: bool,
}

/// Terminates the engine child the way the backend is stopped: SIGTERM first,
/// a bounded wait, then SIGKILL. The engine's own handler flushes its flow
/// state on the TERM.
pub(crate) async fn terminate_engine(mut child: Child) -> EngineExit {
    if let Some(pid) = child.id() {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }
    let mut status = tokio::time::timeout(ANTIDPI_STOP_TIMEOUT, child.wait())
        .await
        .ok()
        .and_then(Result::ok);
    if status.is_none() {
        child.kill().await.ok();
        status = child.wait().await.ok();
    }
    EngineExit {
        status,
        wait_failed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "5e8f4c46-6d95-4aa6-9c3e-6569e1a1f6cd";

    fn rt() -> AntiDpiRuntime {
        AntiDpiRuntime {
            helper_path: PathBuf::from("/usr/bin/v2ray-rs-antidpi"),
            queue: 200,
            strategies: Vec::new(),
            hostlist: None,
        }
    }

    #[test]
    fn install_args_pin_the_queue_and_session() {
        assert_eq!(
            antidpi_install_args(&rt(), None, SESSION),
            ["install-rules", "--queue", "200", "--session-id", SESSION,]
        );
        let mut custom = rt();
        custom.queue = 5;
        assert_eq!(
            antidpi_install_args(&custom, None, SESSION),
            ["install-rules", "--queue", "5", "--session-id", SESSION]
        );
    }

    #[test]
    fn install_args_carry_strategies_hosts_and_owner() {
        let mut custom = rt();
        custom.strategies = vec![
            "fake:blob=quic:repeats=11".to_string(),
            "multisplit:pos=2,midsld:seqovl=1".to_string(),
        ];
        custom.hostlist = Some(PathBuf::from("/etc/v2ray-rs/hosts.txt"));
        assert_eq!(
            antidpi_install_args(&custom, Some("4242:12345"), SESSION),
            [
                "install-rules",
                "--queue",
                "200",
                "--strategy",
                "fake:blob=quic:repeats=11",
                "--strategy",
                "multisplit:pos=2,midsld:seqovl=1",
                "--hosts",
                "/etc/v2ray-rs/hosts.txt",
                "--owner",
                "4242:12345",
                "--session-id",
                SESSION,
            ]
        );
        // One argv element per value: the owner identity is never joined with
        // its flag.
        let args = antidpi_install_args(&rt(), Some("1:2"), SESSION);
        assert!(args.windows(2).any(|w| w[0] == "--owner" && w[1] == "1:2"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--session-id" && w[1] == SESSION)
        );
    }

    #[test]
    fn remove_args_are_owner_and_session_qualified() {
        assert_eq!(
            antidpi_remove_args(None, SESSION),
            ["remove-rules", "--session-id", SESSION]
        );
        assert_eq!(
            antidpi_remove_args(Some("7:99"), SESSION),
            ["remove-rules", "--owner", "7:99", "--session-id", SESSION]
        );
    }

    #[test]
    fn run_args_pass_each_strategy_as_one_argument() {
        let mut custom = rt();
        custom.queue = 7;
        custom.strategies = vec![
            "fake:blob=quic:repeats=11".to_string(),
            "multisplit:pos=2,midsld:seqovl=1".to_string(),
        ];
        assert_eq!(
            antidpi_run_args(&custom, SESSION),
            [
                "run",
                "--queue",
                "7",
                "--no-fw",
                "--strategy",
                "fake:blob=quic:repeats=11",
                "--strategy",
                "multisplit:pos=2,midsld:seqovl=1",
                "--session-id",
                SESSION,
            ]
        );
    }

    #[test]
    fn run_args_omit_strategies_and_hosts_when_unset() {
        // An empty strategy list delegates to the helper's built-in default
        // chains: no `--strategy` flags may reach the command line.
        let args = antidpi_run_args(&rt(), SESSION);
        assert!(!args.contains(&"--strategy".to_string()));
        assert!(!args.contains(&"--hosts".to_string()));
        assert_eq!(
            args,
            ["run", "--queue", "200", "--no-fw", "--session-id", SESSION]
        );
    }

    #[test]
    fn run_args_pass_the_hostlist_path_verbatim() {
        let mut custom = rt();
        custom.hostlist = Some(PathBuf::from("/etc/v2ray-rs/hosts.txt"));
        assert_eq!(
            antidpi_run_args(&custom, SESSION),
            [
                "run",
                "--queue",
                "200",
                "--no-fw",
                "--hosts",
                "/etc/v2ray-rs/hosts.txt",
                "--session-id",
                SESSION,
            ]
        );
    }

    #[tokio::test]
    async fn engine_child_terminates_on_sigterm() {
        // `sleep` ignores nothing and dies on TERM: the SIGTERM branch reaps.
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id().expect("fresh child has a pid");

        let exit = terminate_engine(child).await;
        let status = exit.status.expect("wait succeeded");
        assert_eq!(status.code(), None, "killed by signal, not code");
        assert!(
            nix::sys::signal::kill(Pid::from_raw(pid as i32), None)
                == Err(nix::errno::Errno::ESRCH),
            "engine must be gone after termination"
        );
    }

    #[tokio::test]
    async fn engine_that_ignores_sigterm_is_killed_after_the_stop_budget() {
        // Trap and ignore SIGTERM, so the SIGKILL fallback has to fire.
        let script = "trap '' TERM\nwhile :; do sleep 1; done\n";
        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .spawn()
            .unwrap();

        let started = std::time::Instant::now();
        // Let the child install its TERM trap before probing the fallback.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let exit = terminate_engine(child).await;
        assert!(started.elapsed() >= ANTIDPI_STOP_TIMEOUT);
        assert!(started.elapsed() < ANTIDPI_STOP_TIMEOUT + Duration::from_secs(2));
        assert_eq!(exit.status.and_then(|s| s.code()), None);
    }
}
