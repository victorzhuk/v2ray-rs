//! `v2ray-rs-antidpi` CLI: installs the NFQUEUE rules, runs the engine loop,
//! tears everything down on exit. See `docs/antidpi-registry.md`.

use clap::{Parser, Subcommand};
use std::time::Duration;
use v2ray_rs_antidpi::engine::{Engine, Rotation};
use v2ray_rs_antidpi::error::{Error, Result};
use v2ray_rs_antidpi::fw;
use v2ray_rs_antidpi::state;
use v2ray_rs_antidpi::strategy::DEFAULT_CHAINS;

#[derive(Parser)]
#[command(
    name = "v2ray-rs-antidpi",
    about = "DPI-circumvention packet helper (NFQUEUE desync strategies)",
    after_help = "See docs/antidpi-registry.md for strategy syntax and semantics."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Validate strategy strings and privilege requirements, touch nothing.
    Check {
        /// Strategy chain strings (repeatable).
        #[arg(short, long = "strategy", default_values_t = DEFAULT_CHAINS.iter().map(|s| s.to_string()).collect::<Vec<_>>())]
        strategies: Vec<String>,
    },
    /// Install firewall rules, run the queue loop, remove rules on exit.
    Run {
        /// NFQUEUE number.
        #[arg(short, long, default_value_t = fw::DEFAULT_QUEUE)]
        queue: u16,
        /// Strategy chain strings (repeatable; rotated on failure).
        #[arg(short, long = "strategy", default_values_t = DEFAULT_CHAINS.iter().map(|s| s.to_string()).collect::<Vec<_>>())]
        strategies: Vec<String>,
        /// Hostlist file: one domain per line; fire only for these hosts.
        #[arg(long)]
        hosts: Option<std::path::PathBuf>,
        /// Circular rotation: failures within the window before rotating.
        #[arg(long, default_value = "fails=2:time=300")]
        rotation: String,
        /// Skip firewall rule installation (rules managed externally).
        #[arg(long)]
        no_fw: bool,
        /// Session UUID of the controller-managed install: the run only
        /// verifies this session's journal and never installs or removes.
        #[arg(long)]
        session_id: Option<uuid::Uuid>,
    },
    /// Install only the firewall rules (no engine).
    InstallRules {
        #[arg(short, long, default_value_t = fw::DEFAULT_QUEUE)]
        queue: u16,
        /// Strategy chain strings (repeatable) to validate up front.
        #[arg(short, long = "strategy")]
        strategies: Vec<String>,
        /// Hostlist file to validate up front (readability + parse).
        #[arg(long)]
        hosts: Option<std::path::PathBuf>,
        /// Owning controller as PID:START; default: this process's parent.
        #[arg(long)]
        owner: Option<String>,
        /// Session UUID recorded in the journal; default: a fresh UUID v4.
        #[arg(long)]
        session_id: Option<uuid::Uuid>,
    },
    /// Remove the firewall rules recorded for the owner (or sweep a dead owner).
    RemoveRules {
        /// Remove state recorded by a provably dead owner; refuses a live
        /// owner and unowned tables. Offline recovery only: incompatible
        /// with --session-id.
        #[arg(long, conflicts_with = "session_id")]
        sweep: bool,
        /// Owning controller as PID:START; default: this process's parent.
        #[arg(long)]
        owner: Option<String>,
        /// Session UUID recorded by the installing controller; default: the
        /// owner's journal session.
        #[arg(long)]
        session_id: Option<uuid::Uuid>,
    },
}

fn parse_rotation(s: &str) -> Result<Rotation> {
    let mut fails = 2;
    let mut time = 300;
    for part in s.split(':') {
        match part
            .split_once('=')
            .ok_or_else(|| Error::Strategy(format!("bad rotation '{s}'")))?
        {
            ("fails", v) => {
                fails = v
                    .parse()
                    .map_err(|_| Error::Strategy("bad rotation fails".into()))?;
                if fails == 0 {
                    return Err(Error::Strategy("rotation fails must be at least 1".into()));
                }
            }
            ("time", v) => {
                time = v
                    .parse()
                    .map_err(|_| Error::Strategy("bad rotation time".into()))?
            }
            _ => return Err(Error::Strategy(format!("bad rotation part '{part}'"))),
        }
    }
    Ok(Rotation {
        fails_threshold: fails,
        window: Duration::from_secs(time),
    })
}

/// Upper bound for a hostlist: 4 MiB, plus one detection byte so an
/// oversized file is rejected instead of silently truncated.
const HOSTLIST_MAX_BYTES: u64 = 4 * 1024 * 1024;

fn load_hosts(path: &std::path::Path) -> Result<Vec<String>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // Nonblocking open: opening a named pipe for read must not hang here.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Strategy(format!(
            "hostlist {} is not a regular file",
            path.display()
        )));
    }
    let mut limited = file.take(HOSTLIST_MAX_BYTES + 1);
    let mut data = Vec::new();
    limited.read_to_end(&mut data)?;
    if data.len() as u64 > HOSTLIST_MAX_BYTES {
        return Err(Error::Strategy(format!(
            "hostlist {} exceeds the 4 MiB limit",
            path.display()
        )));
    }
    let data = String::from_utf8(data)
        .map_err(|_| Error::Strategy("hostlist is not valid UTF-8".into()))?;
    Ok(data
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// Parses strategy chains and the hostlist. Pure validation: no netlink, no
/// procfs writes — every caller runs this before its first kernel mutation.
fn validate_inputs(
    strategies: &[String],
    hosts: Option<&std::path::Path>,
) -> Result<(Vec<v2ray_rs_antidpi::strategy::Chain>, Option<Vec<String>>)> {
    let mut chains = Vec::new();
    for s in strategies {
        chains.push(v2ray_rs_antidpi::strategy::parse_chain(s)?);
    }
    let hosts_list = match hosts {
        Some(p) => Some(load_hosts(p)?),
        None => None,
    };
    Ok((chains, hosts_list))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();
    let cli = Cli::parse();
    let code = match cli.command {
        Cmd::Check { strategies } => cmd_check(&strategies),
        Cmd::Run {
            queue,
            strategies,
            hosts,
            rotation,
            no_fw,
            session_id,
        } => cmd_run(
            queue,
            &strategies,
            hosts.as_deref(),
            &rotation,
            no_fw,
            session_id,
        ),
        Cmd::InstallRules {
            queue,
            strategies,
            hosts,
            owner,
            session_id,
        } => cmd_install_rules(
            queue,
            &strategies,
            hosts.as_deref(),
            owner.as_deref(),
            session_id,
        ),
        Cmd::RemoveRules {
            sweep,
            owner,
            session_id,
        } => cmd_remove_rules(sweep, session_id, owner.as_deref()),
    };
    if let Err(e) = code {
        eprintln!("v2ray-rs-antidpi: {e}");
        std::process::exit(1);
    }
}

fn cmd_check(strategies: &[String]) -> Result<()> {
    for s in strategies {
        v2ray_rs_antidpi::strategy::parse_chain(s).map(|_| ())?;
        println!("ok: {s}");
    }
    // Privilege probe: raw socket + SO_MARK.
    match v2ray_rs_antidpi::engine::Engine::probe_caps() {
        Ok(()) => println!("ok: caps (cap_net_raw + cap_net_admin)"),
        Err(e) => {
            println!("missing: {e}");
            return Err(Error::RawSend("insufficient capabilities".into()));
        }
    }
    // Strictly observational: report unmet kernel prerequisites, change
    // nothing.
    let unmet = fw::unmet_sysctls();
    for (key, want) in &unmet {
        println!("unmet: {key}={want}");
    }
    if !unmet.is_empty() {
        return Err(Error::Firewall(
            "kernel prerequisites are missing; nothing was modified".into(),
        ));
    }
    Ok(())
}

/// Maps the session flag and `--no-fw` onto the verification call: managed
/// and installed runs require this session's journal before the queue bind,
/// while a standalone `--no-fw` run allows external rules only when no
/// journal exists. A standalone install gets a fresh UUID v4 session.
fn verify_plan(session_id: Option<uuid::Uuid>, no_fw: bool) -> (Option<uuid::Uuid>, bool) {
    match (session_id, no_fw) {
        (Some(session), _) => (Some(session), true),
        (None, true) => (None, false),
        (None, false) => (Some(uuid::Uuid::new_v4()), true),
    }
}

/// True when this helper no longer runs under the controller that spawned
/// it: re-parented to init (ppid 0/1) or under a different pid. The pid is
/// only compared here; journal ownership additionally pins the start time.
fn parent_gone(parent_pid: u32, current_ppid: i32) -> bool {
    current_ppid <= 1 || current_ppid != parent_pid as i32
}

fn current_ppid() -> i32 {
    unsafe { libc::getppid() }
}

/// The controller identity every ownership decision must agree on: the
/// standalone install's resolved parent, or the actual parent captured
/// before any journal verification for managed runs. Externally managed
/// runs have none.
fn controller_identity(
    owner: &Option<state::ProcessIdentity>,
    session: Option<uuid::Uuid>,
) -> Result<Option<state::ProcessIdentity>> {
    match (owner, session) {
        (Some(owner), _) => Ok(Some(owner.clone())),
        (None, Some(_)) => Ok(Some(state::parent_identity().map_err(|e| {
            Error::Firewall(format!("cannot resolve the owning controller: {e}"))
        })?)),
        (None, None) => Ok(None),
    }
}

/// Verification, readiness publication, and the queue loop: everything a
/// standalone install must undo if it fails after installing. The run
/// guard spans journal verification, the queue bind, and readiness
/// publication: the transaction lock is still held when the READY sentinel
/// is written and is released immediately after it, before the engine
/// loop starts. Teardown serialization does not depend on this guard —
/// the stop path takes its own transaction lock.
fn run_verified_engine(
    queue: u16,
    chains: Vec<v2ray_rs_antidpi::strategy::Chain>,
    hosts_list: Option<Vec<String>>,
    rot: Rotation,
    session: Option<uuid::Uuid>,
    require_journal: bool,
    controller: &Option<state::ProcessIdentity>,
) -> Result<()> {
    let guard = fw::verify_active_for_run(queue, session, controller.as_ref(), require_journal)?;
    // Signal-driven teardown: SIGINT/SIGTERM flip the flag; the queue loop
    // polls it. A lost controller counts as shutdown too.
    unsafe {
        set_signal_handlers();
    }
    let mut engine = Engine::new(chains, Some(rot), hosts_list)?;
    log::info!("running (pid {})", std::process::id());
    engine.run_interruptible_with_ready(
        queue,
        || {
            SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed)
                || controller
                    .as_ref()
                    .is_some_and(|p| parent_gone(p.pid, current_ppid()))
        },
        |queue_num| {
            if controller
                .as_ref()
                .is_some_and(|p| parent_gone(p.pid, current_ppid()))
            {
                return Err(Error::Firewall(
                    "the owning controller exited before the engine became ready".into(),
                ));
            }
            Engine::publish_ready(queue_num)?;
            drop(guard);
            Ok(())
        },
    )
}

fn cmd_run(
    queue: u16,
    strategies: &[String],
    hosts: Option<&std::path::Path>,
    rotation: &str,
    no_fw: bool,
    session_id: Option<uuid::Uuid>,
) -> Result<()> {
    // Everything that can fail without touching the kernel fails here.
    let (chains, hosts_list) = validate_inputs(strategies, hosts)?;
    if chains.is_empty() {
        return Err(Error::Strategy("no strategies given".into()));
    }
    let rot = parse_rotation(rotation)?;
    // A managed run verifies the controller's install and leaves installing
    // and removing to the controller. A standalone run resolves its parent
    // as the owner and installs unless the rules are managed externally.
    let owner = match (session_id, no_fw) {
        (Some(_), _) | (None, true) => None,
        (None, false) => Some(
            state::resolve_owner(None)
                .map_err(|e| Error::Firewall(format!("cannot resolve the owning process: {e}")))?,
        ),
    };
    let (session, require_journal) = verify_plan(session_id, no_fw);
    let controller = controller_identity(&owner, session)?;

    if let (Some(owner), Some(session)) = (owner.clone(), session) {
        // Observational prerequisites: report unmet sysctls, never write
        // namespace policy from an automatic path.
        for (key, want) in fw::unmet_sysctls() {
            log::warn!("kernel prerequisite {key}={want} is unmet; continuing without writing it");
        }
        let owner_pid = owner.pid;
        fw::install(queue, fw::DEFAULT_FWMARK, owner.clone(), session)?;
        log::info!("rules installed (owner pid {owner_pid})");
        // A post-install failure must not leave the just-installed rules
        // behind: remove once, naming both causes when removal also fails.
        let outcome = run_verified_engine(
            queue,
            chains,
            hosts_list,
            rot,
            Some(session),
            require_journal,
            &controller,
        );
        return match outcome {
            Ok(()) => {
                match fw::remove(owner, Some(session)) {
                    Ok(true) => log::info!("rules removed"),
                    Ok(false) => log::info!("no state recorded; nothing to remove"),
                    Err(e) => return Err(Error::Firewall(format!("rule removal failed: {e}"))),
                }
                Ok(())
            }
            Err(e) => match fw::remove(owner, Some(session)) {
                Ok(_) => Err(e),
                Err(re) => Err(Error::Firewall(format!(
                    "{e} and rule removal failed: {re}"
                ))),
            },
        };
    }
    run_verified_engine(
        queue,
        chains,
        hosts_list,
        rot,
        session,
        require_journal,
        &controller,
    )
}

static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// SIGINT/SIGTERM only flip the shutdown flag; the queue loop polls it.
unsafe fn set_signal_handlers() {
    unsafe {
        let h = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
    }
}

fn cmd_install_rules(
    queue: u16,
    strategies: &[String],
    hosts: Option<&std::path::Path>,
    owner: Option<&str>,
    session_id: Option<uuid::Uuid>,
) -> Result<()> {
    // Everything that needs no privilege is validated before any kernel
    // mutation; unmet kernel prerequisites are reported, not written.
    validate_inputs(strategies, hosts)?;
    let owner = state::resolve_owner(owner).map_err(|e| Error::Firewall(format!("owner: {e}")))?;
    let session = session_id.unwrap_or_else(uuid::Uuid::new_v4);
    for (key, want) in fw::unmet_sysctls() {
        log::warn!("kernel prerequisite {key}={want} is unmet; continuing without writing it");
    }
    let owner_pid = owner.pid;
    fw::install(queue, fw::DEFAULT_FWMARK, owner, session)?;
    log::info!("rules installed for owner pid {owner_pid} (session {session})");
    Ok(())
}

fn cmd_remove_rules(
    sweep: bool,
    session_id: Option<uuid::Uuid>,
    owner: Option<&str>,
) -> Result<()> {
    if sweep {
        if fw::sweep()? {
            log::info!("swept the state of a dead anti-DPI owner");
        } else {
            log::info!("nothing to sweep");
        }
        return Ok(());
    }
    let owner = state::resolve_owner(owner).map_err(|e| Error::Firewall(format!("owner: {e}")))?;
    if fw::remove(owner, session_id)? {
        log::info!("rules removed");
    } else {
        log::info!("no state recorded; use --sweep to clear leftovers");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, Cmd, DEFAULT_CHAINS, load_hosts, parent_gone, parse_rotation, validate_inputs,
        verify_plan,
    };
    use clap::Parser;
    use std::path::Path;

    #[test]
    fn rotation_rejects_a_zero_failure_threshold() {
        assert!(parse_rotation("fails=0:time=300").is_err());
        assert!(parse_rotation("fails=1:time=300").is_ok());
    }

    #[test]
    fn invalid_strategy_is_rejected_before_any_kernel_call() {
        assert!(validate_inputs(&["".to_owned()], None).is_err());
        assert!(validate_inputs(&["not-a-known-attack".to_owned()], None).is_err());
    }

    #[test]
    fn missing_hostlist_is_rejected_before_any_kernel_call() {
        assert!(
            validate_inputs(&[], Some(Path::new("/nonexistent/v2ray-rs-test-hostlist"))).is_err()
        );
    }

    #[test]
    fn default_chains_and_a_temporary_hostlist_validate() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts");
        std::fs::write(&hosts, "# comment\nexample.com\n\n").unwrap();
        let default_strategies: Vec<String> =
            DEFAULT_CHAINS.iter().map(|s| s.to_string()).collect();
        let (chains, hosts_list) = validate_inputs(&default_strategies, Some(&hosts)).unwrap();
        assert_eq!(chains.len(), DEFAULT_CHAINS.len());
        assert_eq!(hosts_list, Some(vec!["example.com".to_owned()]));
    }

    #[test]
    fn run_and_install_rules_parse_the_session_flag() {
        let id = uuid::Uuid::new_v4();
        let cli = Cli::try_parse_from([
            "v2ray-rs-antidpi",
            "run",
            "--no-fw",
            "--session-id",
            &id.to_string(),
        ])
        .unwrap();
        match cli.command {
            Cmd::Run {
                no_fw, session_id, ..
            } => {
                assert!(no_fw);
                assert_eq!(session_id, Some(id));
            }
            _ => panic!("expected run"),
        }
        let cli = Cli::try_parse_from([
            "v2ray-rs-antidpi",
            "install-rules",
            "--session-id",
            &id.to_string(),
        ])
        .unwrap();
        match cli.command {
            Cmd::InstallRules { session_id, .. } => assert_eq!(session_id, Some(id)),
            _ => panic!("expected install-rules"),
        }
    }

    #[test]
    fn remove_rules_keeps_the_session_optional_without_sweep() {
        let cli = Cli::try_parse_from(["v2ray-rs-antidpi", "remove-rules"]).unwrap();
        match cli.command {
            Cmd::RemoveRules {
                sweep, session_id, ..
            } => {
                assert!(!sweep);
                assert_eq!(session_id, None);
            }
            _ => panic!("expected remove-rules"),
        }
        let id = uuid::Uuid::new_v4();
        let cli = Cli::try_parse_from([
            "v2ray-rs-antidpi",
            "remove-rules",
            "--session-id",
            &id.to_string(),
        ])
        .unwrap();
        match cli.command {
            Cmd::RemoveRules { session_id, .. } => assert_eq!(session_id, Some(id)),
            _ => panic!("expected remove-rules"),
        }
    }

    #[test]
    fn sweep_refuses_a_session_id() {
        let id = uuid::Uuid::new_v4();
        let err = match Cli::try_parse_from([
            "v2ray-rs-antidpi",
            "remove-rules",
            "--sweep",
            "--session-id",
            &id.to_string(),
        ]) {
            Err(err) => err,
            Ok(_) => panic!("--sweep with --session-id must be refused at parse time"),
        };
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "{err}"
        );
    }

    #[test]
    fn verify_plan_matches_the_session_and_journal_contract() {
        let id = uuid::Uuid::new_v4();
        // Managed runs verify the controller's session against its journal.
        assert_eq!(verify_plan(Some(id), false), (Some(id), true));
        assert_eq!(verify_plan(Some(id), true), (Some(id), true));
        // Standalone --no-fw: external rules allowed only without a journal.
        assert_eq!(verify_plan(None, true), (None, false));
        // Standalone install: fresh session, journal required.
        let (session, require_journal) = verify_plan(None, false);
        assert_ne!(session, Some(id));
        assert!(session.is_some());
        assert!(require_journal);
    }

    #[test]
    fn parent_gone_flags_reparenting_and_lost_parents() {
        assert!(!parent_gone(4242, 4242));
        assert!(parent_gone(4242, 4243));
        assert!(parent_gone(4242, 1));
        assert!(parent_gone(4242, 0));
    }

    #[test]
    fn oversized_hostlist_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts");
        std::fs::write(&hosts, vec![b'a'; 4 * 1024 * 1024 + 1]).unwrap();
        assert!(load_hosts(&hosts).is_err());
    }

    #[test]
    fn exactly_four_mebibytes_of_hostlist_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts");
        std::fs::write(&hosts, vec![b'a'; 4 * 1024 * 1024]).unwrap();
        assert!(load_hosts(&hosts).is_ok());
    }

    #[test]
    fn non_regular_hostlists_are_rejected() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        assert!(load_hosts(dir.path()).is_err());
        let fifo = dir.path().join("pipe");
        let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(load_hosts(&fifo).is_err());
    }
}
