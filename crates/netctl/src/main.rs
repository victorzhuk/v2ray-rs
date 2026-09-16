mod net;
mod validate;

use std::net::IpAddr;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

/// Privileged route helper for v2ray-rs TUN mode. Programs and tears down the
/// xray TUN interface address and split routes, and recovers leftover state
/// after an unclean shutdown. Requires CAP_NET_ADMIN.
#[derive(Parser)]
#[command(name = "v2ray-rs-netctl", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Assign the address(es) and add the split routes for an xray TUN device.
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
    },
    /// Remove the xray policy rules, flush the tunnel table (IPv4 and IPv6) and
    /// delete the xray TUN device (no-op if absent).
    XrayDown {
        #[arg(long)]
        iface: String,
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

#[tokio::main(flavor = "current_thread")]
async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::XrayUp {
            iface,
            addr,
            addr6,
            bypass_uid,
            capture_dns,
            strict,
            exclude,
        } => {
            validate::validate_iface(&iface)?;
            // The named device must be a TUN device the backend already created;
            // refusing anything else stops an unprivileged caller from routing
            // system traffic into lo or a physical NIC.
            if !validate::is_tun_device(&iface) {
                return Err(format!("refusing xray-up on {iface}: not a TUN device"));
            }
            let v4 = validate::parse_cidr(&addr)?;
            let v6 = addr6.as_deref().map(validate::parse_cidr).transpose()?;
            validate::check_exclusion_count(exclude.len())?;
            let handle = net::connect()?;
            net::xray_up(&handle, &iface, v4, v6, bypass_uid, capture_dns, strict).await?;
            net::replace_exclusions(&handle, &exclude).await
        }
        Command::XrayDown { iface } => {
            validate::validate_iface(&iface)?;
            let handle = net::connect()?;
            net::xray_down(&handle, &iface).await
        }
        Command::Recover { iface, backend } => {
            validate::validate_iface(&iface)?;
            let handle = net::connect()?;
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

    use super::{Cli, Command};
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
}
