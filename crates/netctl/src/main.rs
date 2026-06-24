mod net;
mod validate;

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
    },
    /// Remove the xray TUN device (no-op if absent).
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
        } => {
            validate::validate_iface(&iface)?;
            let v4 = validate::parse_cidr(&addr)?;
            let v6 = addr6.as_deref().map(validate::parse_cidr).transpose()?;
            let handle = net::connect()?;
            net::xray_up(&handle, &iface, v4, v6, bypass_uid).await
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
