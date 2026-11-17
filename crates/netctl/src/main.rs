mod net;
mod ownership;
mod validate;

use std::net::IpAddr;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

/// Privileged route helper for v2ray-rs TUN mode. Programs and tears down the
/// xray TUN interface addresses and policy routes, and recovers leftover state
/// after an unclean shutdown. Requires CAP_NET_ADMIN.
#[derive(Parser)]
#[command(name = "v2ray-rs-netctl", version, about)]
struct Cli {
    #[arg(long, global = true, conflicts_with = "force_legacy")]
    journal: Option<std::path::PathBuf>,
    /// Bypass ownership checks only for explicit operator recovery of legacy state.
    #[arg(long, global = true)]
    force_legacy: bool,
    #[arg(long, global = true, requires = "journal")]
    session_id: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Reserve xray routing state before the backend creates a device.
    Reserve {
        #[arg(long)]
        iface: String,
    },
    /// Assign the address(es) and add policy routes for an xray TUN device.
    XrayUp {
        #[arg(long)]
        iface: String,
        #[arg(long)]
        addr: String,
        #[arg(long)]
        addr6: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        bypass_uid: Option<u32>,
        /// Steer port-53 traffic into the tunnel table, so a resolver on the
        /// local subnet is reached through the tunnel like everything else.
        #[arg(long)]
        capture_dns: bool,
        /// Add an unreachable default to the tunnel table (IPv4 and IPv6) and
        /// install the IPv6 policy rules even without --addr6, so traffic fails
        /// closed instead of leaking out the real interface once the device is gone.
        #[arg(long)]
        strict: bool,
        // Any caller can exclude any prefix, even 0.0.0.0/1 plus 128.0.0.0/1,
        // which takes the whole tunnel down no further than xray-down already
        // allows.
        /// Route this prefix via the main table instead of the tunnel. Repeatable.
        #[arg(long = "exclude", value_name = "CIDR", value_parser = validate::parse_exclusion)]
        exclude: Vec<(IpAddr, u8)>,
        /// Kernel ifindex the caller observed for `--iface`. When given and the
        /// live link has a different index, refuse before touching anything.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        ifindex: Option<u32>,
    },
    /// Remove journaled xray routing state and the verified TUN device.
    XrayDown {
        #[arg(long)]
        iface: String,
        /// Kernel ifindex the caller observed for `--iface`; refuses a link
        /// that reused the name. Optional so legacy callers stay valid.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        ifindex: Option<u32>,
    },
    /// Remove leftover TUN state after an unclean shutdown.
    Recover {
        #[arg(long)]
        iface: String,
        #[command(flatten)]
        backend: BackendFlag,
    },
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct BackendFlag {
    #[arg(long)]
    singbox: bool,
    #[arg(long)]
    xray: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("netctl: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Pure, netlink-free checks for the invocation shape, so an invalid call
/// fails before the mutation lock is taken and stays unit-testable.
fn validate_invocation(cli: &Cli, euid: u32) -> Result<(), String> {
    if cli.journal.is_none() && !cli.force_legacy {
        return Err("an ownership journal is required; legacy state needs explicit --force-legacy operator recovery".into());
    }
    if cli.force_legacy && euid != 0 {
        return Err("--force-legacy requires effective UID 0".into());
    }
    if cli.journal.is_some() && cli.session_id.is_none() {
        return Err("journaled commands require --session-id".into());
    }
    if cli.session_id.is_some() && cli.journal.is_none() {
        return Err("session identity requires --journal".into());
    }
    if matches!(cli.command, Command::Reserve { .. }) && cli.journal.is_none() {
        return Err("reserve requires --journal".into());
    }
    let Command::XrayUp { ifindex, .. } = &cli.command else {
        return Ok(());
    };
    if cli.journal.is_some() && ifindex.is_none() {
        return Err("journaled xray-up requires --ifindex".into());
    }
    Ok(())
}

/// Reads the effective UID from the kernel's process identity table; the route
/// helper gains `CAP_NET_ADMIN` without a root effective UID.
fn effective_uid() -> Result<u32, String> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|e| format!("read process identity: {e}"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().nth(1))
        .and_then(|euid| euid.parse().ok())
        .ok_or_else(|| "could not determine effective UID".into())
}

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<(), String> {
    validate_invocation(&cli, effective_uid()?)?;
    let _mutation_lock = ownership::mutation_lock(cli.journal.as_deref())?;
    match cli.command {
        Command::Reserve { iface } => {
            validate::validate_iface(&iface)?;
            let path = cli.journal.as_deref().ok_or("reserve requires --journal")?;
            let handle = net::connect()?;
            ownership::Owner::open(path, &iface, cli.session_id.as_deref().unwrap_or_default())?
                .reserve(&handle)
                .await
        }
        Command::XrayUp {
            iface,
            addr,
            addr6,
            bypass_uid,
            capture_dns,
            strict,
            exclude,
            ifindex,
        } => {
            validate::validate_iface(&iface)?;
            // The named device must be a TUN device the backend already created;
            // refusing anything else stops an unprivileged caller from routing
            // system traffic into lo or a physical NIC.
            if !validate::is_tun_device(&iface) {
                return Err(format!("refusing xray-up on {iface}: not a TUN device"));
            }
            if let Some(expected) = ifindex {
                let live = validate::read_ifindex(&iface)?.ok_or_else(|| {
                    format!("refusing xray-up on {iface}: interface is absent, expected ifindex {expected}")
                })?;
                validate::check_ifindex(&iface, expected, live)?;
            }
            let v4 = validate::parse_cidr(&addr)?;
            let v6 = addr6.as_deref().map(validate::parse_cidr).transpose()?;
            validate::check_exclusion_count(exclude.len())?;
            let handle = net::connect()?;
            let owner = if let Some(path) = &cli.journal {
                let mut owner = ownership::Owner::open(
                    path,
                    &iface,
                    cli.session_id.as_deref().unwrap_or_default(),
                )?;
                owner
                    .attach(
                        &handle,
                        ifindex.ok_or("journaled xray-up requires --ifindex")?,
                    )
                    .await?;
                Some(owner)
            } else {
                None
            };
            let mut context = net::Context { handle, owner };
            net::xray_up(
                &mut context,
                &iface,
                v4,
                v6,
                bypass_uid,
                capture_dns,
                strict,
            )
            .await?;
            net::replace_exclusions(&mut context, &exclude).await?;
            if let Some(owner) = context.owner.as_mut() {
                owner.active()?;
            }
            Ok(())
        }
        Command::XrayDown { iface, ifindex } => {
            validate::validate_iface(&iface)?;
            if let Some(expected) = ifindex
                && let Some(live) = validate::read_ifindex(&iface)?
            {
                validate::check_ifindex(&iface, expected, live)?;
            }
            let handle = net::connect()?;
            if let Some(path) = &cli.journal {
                ownership::Owner::open(path, &iface, cli.session_id.as_deref().unwrap_or_default())?
                    .cleanup(&handle)
                    .await
            } else {
                net::xray_down(&handle, &iface).await
            }
        }
        Command::Recover { iface, backend } => {
            validate::validate_iface(&iface)?;
            let handle = net::connect()?;
            if let Some(path) = &cli.journal {
                if backend.singbox {
                    return Err("sing-box ownership cannot be established; explicit operator recovery is required".into());
                }
                return ownership::Owner::open(
                    path,
                    &iface,
                    cli.session_id.as_deref().unwrap_or_default(),
                )?
                .cleanup(&handle)
                .await;
            }
            if backend.singbox {
                net::recover_singbox(&handle, &iface).await
            } else {
                net::recover_xray(&handle, &iface).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{Cli, Command, validate_invocation};
    use clap::Parser;

    fn parse(extra: &[&str]) -> Cli {
        let mut args = vec![
            "v2ray-rs-netctl",
            "xray-up",
            "--iface",
            "xtun0",
            "--addr",
            "172.19.0.1/30",
        ];
        args.extend_from_slice(extra);
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn xray_up_parses_strict_flag() {
        assert!(matches!(
            parse(&["--strict"]).command,
            Command::XrayUp { strict: true, .. }
        ));
        assert!(matches!(
            parse(&[]).command,
            Command::XrayUp { strict: false, .. }
        ));
    }

    #[test]
    fn xray_subcommands_parse_optional_ifindex() {
        let up = parse(&["--ifindex", "17"]);
        assert!(matches!(
            up.command,
            Command::XrayUp {
                ifindex: Some(17),
                ..
            }
        ));
        assert!(matches!(
            parse(&[]).command,
            Command::XrayUp { ifindex: None, .. }
        ));

        let down = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "xray-down",
            "--iface",
            "xtun0",
            "--ifindex",
            "42",
        ])
        .unwrap();
        assert!(matches!(
            down.command,
            Command::XrayDown {
                ifindex: Some(42),
                ..
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["v2ray-rs-netctl", "xray-down", "--iface", "xtun0"])
                .unwrap()
                .command,
            Command::XrayDown { ifindex: None, .. }
        ));
    }

    #[test]
    fn xray_up_parses_repeated_exclude() {
        let Command::XrayUp { exclude, .. } =
            parse(&["--exclude", "10.1.2.3/8", "--exclude", "2001:db8::1/32"]).command
        else {
            panic!("expected xray-up");
        };
        let want: Vec<(IpAddr, u8)> = vec![
            ("10.0.0.0".parse().unwrap(), 8),
            ("2001:db8::".parse().unwrap(), 32),
        ];
        assert_eq!(exclude, want);
    }

    #[test]
    fn xray_up_refuses_invalid_exclude_naming_it() {
        let err = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "xray-up",
            "--iface",
            "xtun0",
            "--addr",
            "172.19.0.1/30",
            "--exclude",
            "10.0.0.0/33",
        ])
        .err()
        .expect("invalid exclusion must be refused");
        assert!(err.to_string().contains("10.0.0.0/33"), "{err}");
    }

    #[test]
    fn invocation_without_journal_or_force_legacy_is_refused() {
        let err = validate_invocation(&parse(&[]), 0).unwrap_err();
        assert!(err.contains("--force-legacy"), "{err}");
    }

    #[test]
    fn force_legacy_requires_root() {
        let legacy = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "recover",
            "--xray",
            "--iface",
            "xtun0",
            "--force-legacy",
        ])
        .unwrap();
        let err = validate_invocation(&legacy, 1000).unwrap_err();
        assert!(err.contains("--force-legacy"), "{err}");
        assert!(err.contains("UID 0"), "{err}");
        assert!(validate_invocation(&legacy, 0).is_ok());
    }

    #[test]
    fn journaled_invocation_requires_session_id() {
        let cli = parse(&["--journal", "journal.json", "--ifindex", "3"]);
        let err = validate_invocation(&cli, 0).unwrap_err();
        assert!(err.contains("--session-id"), "{err}");
    }

    #[test]
    fn force_legacy_reserve_without_journal_is_refused() {
        let cli = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "reserve",
            "--iface",
            "xtun0",
            "--force-legacy",
        ])
        .unwrap();
        let err = validate_invocation(&cli, 0).unwrap_err();
        assert!(err.contains("reserve requires --journal"), "{err}");
    }

    #[test]
    fn journaled_xray_up_requires_ifindex() {
        let cli = parse(&["--journal", "journal.json", "--session-id", "sid"]);
        let err = validate_invocation(&cli, 0).unwrap_err();
        assert!(err.contains("--ifindex"), "{err}");
        let cli = parse(&[
            "--journal",
            "journal.json",
            "--session-id",
            "sid",
            "--ifindex",
            "3",
        ]);
        assert!(validate_invocation(&cli, 0).is_ok());
    }

    #[test]
    fn journaled_and_legacy_invocations_pass_validation() {
        let journaled = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "recover",
            "--xray",
            "--iface",
            "xtun0",
            "--journal",
            "journal.json",
            "--session-id",
            "sid",
        ])
        .unwrap();
        assert!(validate_invocation(&journaled, 0).is_ok());
        let legacy = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "xray-down",
            "--iface",
            "xtun0",
            "--force-legacy",
        ])
        .unwrap();
        assert!(validate_invocation(&legacy, 0).is_ok());
    }

    #[test]
    fn journal_and_force_legacy_conflict_at_clap_level() {
        assert!(
            Cli::try_parse_from([
                "v2ray-rs-netctl",
                "xray-up",
                "--iface",
                "xtun0",
                "--addr",
                "172.19.0.1/30",
                "--journal",
                "journal.json",
                "--force-legacy",
            ])
            .is_err()
        );
    }

    #[test]
    fn session_id_without_journal_is_refused() {
        // Top-level flag ordering: clap refuses it via `requires = "journal"`.
        assert!(
            Cli::try_parse_from([
                "v2ray-rs-netctl",
                "--session-id",
                "sid",
                "xray-down",
                "--iface",
                "xtun0",
                "--force-legacy",
            ])
            .is_err()
        );

        // A global flag repeated after the subcommand slips past clap; the
        // validation boundary refuses that shape instead.
        let cli = Cli::try_parse_from([
            "v2ray-rs-netctl",
            "xray-down",
            "--iface",
            "xtun0",
            "--force-legacy",
            "--session-id",
            "sid",
        ])
        .unwrap();
        let err = validate_invocation(&cli, 0).unwrap_err();
        assert!(err.contains("--journal"), "{err}");
    }
}
