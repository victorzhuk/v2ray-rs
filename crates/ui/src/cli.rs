use std::path::PathBuf;

use clap::Parser;
use v2ray_rs_core::cli::CliPaths;

#[derive(Debug, Parser)]
#[command(name = "v2ray-rs", version, about = "V2Ray/XRay proxy GUI")]
pub struct CliArgs {
    #[arg(long = "profile")]
    pub profile: Option<String>,

    #[arg(long = "config-dir")]
    pub config_dir: Option<PathBuf>,

    #[arg(long = "data-dir")]
    pub data_dir: Option<PathBuf>,

    #[arg(long = "cache-dir")]
    pub cache_dir: Option<PathBuf>,

    #[arg(long = "runtime-dir")]
    pub runtime_dir: Option<PathBuf>,

    #[arg(long = "state-dir")]
    pub state_dir: Option<PathBuf>,

    #[arg(long = "reset-instance")]
    pub reset_instance: bool,

    #[arg(long = "install-icons")]
    pub install_icons: bool,

    #[arg(long = "i-understand")]
    pub i_understand: bool,
}

impl CliArgs {
    pub fn paths(&self) -> CliPaths {
        CliPaths {
            config_dir: self.config_dir.clone(),
            data_dir: self.data_dir.clone(),
            cache_dir: self.cache_dir.clone(),
            runtime_dir: self.runtime_dir.clone(),
            state_dir: self.state_dir.clone(),
            install_icons: self.install_icons,
        }
    }
}
