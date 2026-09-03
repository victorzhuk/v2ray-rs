use std::io::Write;
use std::process::Command;

use v2ray_rs_core::config::{ConfigGenerator, XrayGenerator};
use v2ray_rs_core::models::{
    AppSettings, ConnectionNodeRef, DnsHijackMode, DnsProtocol, DnsServerConfig, HostOverride,
    ProxyNode, RoutingRule, RuleAction, RuleMatch, ShadowsocksConfig, TlsSettings,
    TransportSettings, VlessConfig, WsSettings,
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
    check_with_nodes(name, settings, rules, &[ss_node()]);
}

/// `xray run -test` starts the servers, so every case needs listen ports and a
/// TUN name of its own — otherwise it collides with a running instance of the
/// app, or with an earlier case, and fails for reasons that have nothing to do
/// with the config being checked.
fn isolate(name: &str, settings: &AppSettings) -> AppSettings {
    let slot: u16 = name
        .bytes()
        .fold(0u16, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u16))
        % 400;
    let mut isolated = settings.clone();
    isolated.socks_port = 39000 + slot;
    isolated.http_port = 39400 + slot;
    isolated.tun.interface_name = format!("xchk{slot}");
    isolated
}

fn check_with_nodes(
    name: &str,
    settings: &AppSettings,
    rules: &[RoutingRule],
    nodes: &[ProxyNode],
) {
    let settings = &isolate(name, settings);
    let config = XrayGenerator
        .generate(nodes, rules, settings)
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

    let mut tun_pinned = AppSettings::default();
    tun_pinned.tun.enabled = true;
    tun_pinned.dns.hosts = vec![HostOverride {
        domain: "ss.example.com".to_string(),
        ip: "203.0.113.9".to_string(),
    }];
    cases.push(("tun-derived-dns-with-pinned-hosts", tun_pinned));

    let mut tun_direct_detour = AppSettings::default();
    tun_direct_detour.tun.enabled = true;
    tun_direct_detour.dns.enabled = true;
    tun_direct_detour.dns.servers = vec![
        DnsServerConfig {
            tag: "remote".to_string(),
            protocol: DnsProtocol::Doh,
            address: "dns.adguard.com".to_string(),
            port: None,
            detour: None,
        },
        DnsServerConfig {
            tag: "domestic".to_string(),
            protocol: DnsProtocol::Udp,
            address: "77.88.8.8".to_string(),
            port: None,
            detour: Some("direct".to_string()),
        },
    ];
    cases.push(("tun-user-dns-direct-detour", tun_direct_detour));

    for (name, settings) in &cases {
        check(name, settings);
    }
}

fn ws_node() -> ProxyNode {
    ProxyNode::Vless(VlessConfig {
        address: "ws.example.com".into(),
        port: 443,
        uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        encryption: Some("none".into()),
        flow: None,
        transport: TransportSettings::Ws(WsSettings {
            path: "/ws".into(),
            host: Some("cdn.example.com".into()),
            headers: Default::default(),
        }),
        tls: Some(TlsSettings {
            server_name: Some("ws.example.com".into()),
            ..Default::default()
        }),
        remark: Some("Pinned WS".into()),
    })
}

#[test]
fn pinned_node_and_ws_transport_options_pass_xray_test() {
    if !xray_available() {
        eprintln!("xray not found in PATH, skipping");
        return;
    }

    let pinned = ConnectionNodeRef::Manual {
        node_id: uuid::Uuid::new_v4(),
    };
    let rules = vec![
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: "api.z.ai".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node: Some(pinned),
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        },
    ];

    let settings = AppSettings {
        ws_heartbeat_secs: 30,
        ..Default::default()
    };

    check_with_nodes(
        "pinned-node-with-ws-heartbeat",
        &settings,
        &rules,
        &[ss_node(), ws_node()],
    );
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
            via_node: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainKeyword {
                keyword: "sina".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainFull {
                domain: "ads.example.com".into(),
            },
            action: RuleAction::Block,
            enabled: true,
            group: None,
            via_node: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Protocol {
                name: "bittorrent".into(),
            },
            action: RuleAction::Block,
            enabled: true,
            group: None,
            via_node: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Port {
                spec: "80,443,1000-2000".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        },
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Network {
                spec: "tcp,udp".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        },
    ];

    check_with_rules("new-rule-kinds", &AppSettings::default(), &rules);
}
