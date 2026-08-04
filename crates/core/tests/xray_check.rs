use std::io::Write;
use std::process::Command;

use v2ray_rs_core::config::{ConfigGenerator, XrayGenerator};
use v2ray_rs_core::models::{
    AppSettings, DnsHijackMode, DnsProtocol, DnsServerConfig, ProxyNode, RoutingRule, RuleAction,
    RuleMatch, ShadowsocksConfig,
};

fn xray_available() -> bool {
    Command::new("xray")
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

fn check(name: &str, settings: &AppSettings) {
    check_with_rules(name, settings, &[]);
}

fn check_with_rules(name: &str, settings: &AppSettings, rules: &[RoutingRule]) {
    let config = XrayGenerator
        .generate(&[ss_node()], rules, settings)
        .unwrap_or_else(|err| panic!("{name}: generate failed: {err}"));
    let json = serde_json::to_string_pretty(&config).unwrap();

    let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    file.write_all(json.as_bytes()).unwrap();
    file.flush().unwrap();

    let output = Command::new("xray")
        .arg("run")
        .arg("-test")
        .arg("-c")
        .arg(file.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{name}: xray run -test failed\nstdout: {}\nstderr: {}\nconfig: {json}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn generated_xray_configs_pass_xray_test() {
    if !xray_available() {
        eprintln!("xray not found in PATH, skipping");
        return;
    }

    let mut cases: Vec<(&str, AppSettings)> = Vec::new();

    cases.push(("plain", AppSettings::default()));

    let mut tun_only = AppSettings::default();
    tun_only.tun.enabled = true;
    cases.push(("tun-derived-dns-hijack", tun_only));

    let mut tun_native = AppSettings::default();
    tun_native.tun.enabled = true;
    tun_native.tun.dns_hijack = DnsHijackMode::Native;
    cases.push(("tun-derived-dns-native", tun_native));

    let mut tun_user_dns = AppSettings::default();
    tun_user_dns.tun.enabled = true;
    tun_user_dns.dns.enabled = true;
    tun_user_dns.dns.servers = vec![
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
            address: "77.88.8.8".to_string(),
            port: None,
            detour: None,
        },
    ];
    cases.push(("tun-user-dns-hijack", tun_user_dns));

    for (name, settings) in &cases {
        check(name, settings);
    }
}

#[test]
fn new_rule_kinds_pass_xray_test() {
    if !xray_available() {
        eprintln!("xray not found in PATH, skipping");
        return;
    }

    let rules = vec![
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: "example.com".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainKeyword {
                keyword: "sina".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainFull {
                domain: "ads.example.com".into(),
            },
            action: RuleAction::Block,
            enabled: true,
            group: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Protocol {
                name: "bittorrent".into(),
            },
            action: RuleAction::Block,
            enabled: true,
            group: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Port {
                spec: "80,443,1000-2000".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Network {
                spec: "tcp,udp".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
        },
    ];

    check_with_rules("new-rule-kinds", &AppSettings::default(), &rules);
}
