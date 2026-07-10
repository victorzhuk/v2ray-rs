use std::io::Write;
use std::process::Command;

use v2ray_rs_core::config::{ConfigGenerator, SingboxGenerator};
use v2ray_rs_core::models::{
    AppSettings, DnsHijackMode, DnsProtocol, DnsServerConfig, FakeIpConfig, HostOverride,
    ProxyNode, ShadowsocksConfig,
};

fn sing_box_available() -> bool {
    Command::new("sing-box")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ss_node() -> ProxyNode {
    ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: "ss.example.com".into(),
        port: 8388,
        method: "aes-256-gcm".into(),
        password: "secret".into(),
        remark: Some("Test SS".into()),
    })
}

fn dns_servers() -> Vec<DnsServerConfig> {
    vec![
        DnsServerConfig {
            tag: "remote".to_string(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".to_string(),
            port: None,
            detour: None,
        },
        DnsServerConfig {
            tag: "domestic".to_string(),
            protocol: DnsProtocol::Udp,
            address: "223.5.5.5".to_string(),
            port: None,
            detour: None,
        },
    ]
}

fn check(name: &str, settings: &AppSettings) {
    let config = SingboxGenerator
        .generate(&[ss_node()], &[], settings)
        .unwrap_or_else(|err| panic!("{name}: generate failed: {err}"));
    let json = serde_json::to_string_pretty(&config).unwrap();

    let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    file.write_all(json.as_bytes()).unwrap();
    file.flush().unwrap();

    let output = Command::new("sing-box")
        .arg("check")
        .arg("-c")
        .arg(file.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{name}: sing-box check failed\nstderr: {}\nconfig: {json}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn generated_singbox_configs_pass_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let mut cases: Vec<(&str, AppSettings)> = Vec::new();

    cases.push(("plain", AppSettings::default()));

    let mut tun_only = AppSettings::default();
    tun_only.tun.enabled = true;
    cases.push(("tun-only", tun_only));

    let mut dns_only = AppSettings::default();
    dns_only.dns.enabled = true;
    dns_only.dns.servers = dns_servers();
    cases.push(("dns-only", dns_only));

    let mut tun_dns_hijack = AppSettings::default();
    tun_dns_hijack.tun.enabled = true;
    tun_dns_hijack.dns.enabled = true;
    tun_dns_hijack.dns.servers = dns_servers();
    tun_dns_hijack.tun.dns_hijack = DnsHijackMode::Hijack;
    cases.push(("tun-dns-hijack", tun_dns_hijack));

    let mut tun_dns_native = AppSettings::default();
    tun_dns_native.tun.enabled = true;
    tun_dns_native.dns.enabled = true;
    tun_dns_native.dns.servers = dns_servers();
    tun_dns_native.tun.dns_hijack = DnsHijackMode::Native;
    cases.push(("tun-dns-native", tun_dns_native));

    let mut tun_dns_disabled = AppSettings::default();
    tun_dns_disabled.tun.enabled = true;
    tun_dns_disabled.dns.enabled = true;
    tun_dns_disabled.dns.servers = dns_servers();
    tun_dns_disabled.tun.dns_hijack = DnsHijackMode::Disabled;
    cases.push(("tun-dns-disabled-hijack", tun_dns_disabled));

    let mut fakeip = AppSettings::default();
    fakeip.dns.enabled = true;
    fakeip.dns.servers = dns_servers();
    fakeip.dns.fakeip = FakeIpConfig {
        enabled: true,
        inet4_range: "198.18.0.0/15".to_string(),
        inet6_range: "fc00::/18".to_string(),
    };
    cases.push(("fakeip", fakeip));

    let mut tun_fakeip = AppSettings::default();
    tun_fakeip.tun.enabled = true;
    tun_fakeip.dns.enabled = true;
    tun_fakeip.dns.servers = dns_servers();
    tun_fakeip.dns.fakeip = FakeIpConfig {
        enabled: true,
        inet4_range: "198.18.0.0/15".to_string(),
        inet6_range: "fc00::/18".to_string(),
    };
    cases.push(("tun-fakeip", tun_fakeip));

    let mut hosts = AppSettings::default();
    hosts.dns.enabled = true;
    hosts.dns.servers = dns_servers();
    hosts.dns.hosts = vec![
        HostOverride {
            domain: "example.com".to_string(),
            ip: "192.0.2.1".to_string(),
        },
        HostOverride {
            domain: "test.local".to_string(),
            ip: "192.0.2.2".to_string(),
        },
    ];
    cases.push(("hosts", hosts));

    let mut tun_hosts_hijack = AppSettings::default();
    tun_hosts_hijack.tun.enabled = true;
    tun_hosts_hijack.dns.enabled = true;
    tun_hosts_hijack.dns.servers = dns_servers();
    tun_hosts_hijack.dns.hosts = vec![HostOverride {
        domain: "example.com".to_string(),
        ip: "192.0.2.1".to_string(),
    }];
    cases.push(("tun-hosts-hijack", tun_hosts_hijack));

    let mut detour_legacy = AppSettings::default();
    detour_legacy.dns.enabled = true;
    let mut servers = dns_servers();
    servers[0].detour = Some("proxy-0".to_string());
    detour_legacy.dns.servers = servers;
    cases.push(("detour-legacy-sentinel", detour_legacy));

    let mut detour_proxy = AppSettings::default();
    detour_proxy.dns.enabled = true;
    let mut servers = dns_servers();
    servers[0].detour = Some("proxy".to_string());
    detour_proxy.dns.servers = servers;
    cases.push(("detour-proxy-sentinel", detour_proxy));

    let mut detour_direct = AppSettings::default();
    detour_direct.dns.enabled = true;
    let mut servers = dns_servers();
    servers[0].detour = Some("direct".to_string());
    detour_direct.dns.servers = servers;
    cases.push(("detour-direct", detour_direct));

    let mut tun_exclusions = AppSettings::default();
    tun_exclusions.tun.enabled = true;
    tun_exclusions.dns.enabled = true;
    tun_exclusions.dns.servers = dns_servers();
    tun_exclusions.tun.exclude_processes = vec!["cloudflared".to_string()];
    tun_exclusions.tun.exclude_domains = vec!["example.com".to_string()];
    cases.push(("tun-exclusions", tun_exclusions));

    let mut single_doh_server = AppSettings::default();
    single_doh_server.dns.enabled = true;
    single_doh_server.dns.servers = vec![DnsServerConfig {
        tag: "remote".to_string(),
        protocol: DnsProtocol::Doh,
        address: "1.1.1.1".to_string(),
        port: None,
        detour: None,
    }];
    cases.push(("single-doh-server", single_doh_server));

    for (name, settings) in &cases {
        check(name, settings);
    }
}
