//! A/B harness: pick one node per subscription flavor from the user's real
//! subscriptions.json, generate an xray config with the app's own generator on
//! test ports (no TUN, no DNS overrides), and write it out for live testing.
//!
//! Usage: repro_ab <subscriptions.json> <out_dir> <sub-name=remark-substr> ...

use v2ray_rs_core::config::{ConfigGenerator, XrayGenerator};
use v2ray_rs_core::models::{AppSettings, Subscription};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (sub_path, out_dir, picks) = (args[0].clone(), args[1].clone(), &args[2..]);

    let raw = std::fs::read_to_string(&sub_path).expect("read subscriptions");
    let subs: Vec<Subscription> = serde_json::from_str(&raw).expect("parse subscriptions");

    let settings = AppSettings {
        socks_port: 2180,
        http_port: 2181,
        ..Default::default()
    };

    for pick in picks {
        let (sub_name, remark_sub) = pick.split_once('=').expect("pick as sub-name=remark");
        let sub = subs
            .iter()
            .find(|s| s.name == sub_name)
            .unwrap_or_else(|| panic!("subscription {sub_name} not found"));
        let entry = sub
            .nodes
            .iter()
            .find(|n| {
                n.enabled
                    && n.node
                        .remark()
                        .is_some_and(|r| r.to_lowercase().contains(&remark_sub.to_lowercase()))
            })
            .unwrap_or_else(|| panic!("no enabled node matching '{remark_sub}' in {sub_name}"));

        let config = XrayGenerator
            .generate(std::slice::from_ref(&entry.node), &[], &settings)
            .unwrap_or_else(|e| panic!("generate {sub_name}: {e}"));

        let mut out = out_dir.clone();
        out.push_str(&format!("ab-{sub_name}.json"));
        std::fs::write(&out, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        println!(
            "{} -> {} ({})",
            sub_name,
            out,
            entry.node.remark().unwrap_or("?")
        );
    }
}
