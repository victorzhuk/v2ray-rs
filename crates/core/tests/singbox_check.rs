use std::io::Write;
use std::process::Command;

use v2ray_rs_core::config::{ConfigGenerator, ConfigWriter, SingboxGenerator};
use v2ray_rs_core::models::{
    AppSettings, BackendType, DnsHijackMode, DnsProtocol, DnsServerConfig, FakeIpConfig,
    HostOverride, ProxyNode, RoutingRule, RuleAction, RuleMatch, ShadowsocksConfig,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::AppProfile;

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
    check_with_rules(name, settings, &[], |_| {});
}

fn check_with_rules(
    name: &str,
    settings: &AppSettings,
    rules: &[RoutingRule],
    patch: impl FnOnce(&mut serde_json::Value),
) {
    let mut config = SingboxGenerator
        .generate(&[ss_node()], rules, settings)
        .unwrap_or_else(|err| panic!("{name}: generate failed: {err}"));
    patch(&mut config);
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

#[test]
fn new_rule_kinds_pass_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
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

    check_with_rules("new-rule-kinds", &AppSettings::default(), &rules, |_| {});
}

/// Public providers commonly address DoH servers by hostname
/// (https://dns.google/dns-query, https://cloudflare-dns.com/dns-query) rather
/// than IP literal. sing-box can't resolve such a server using itself as the
/// bootstrap resolver - `sing-box check` used to reject this with "missing
/// domain resolver for domain server address".
#[test]
fn hostname_only_dns_servers_pass_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let mut settings = AppSettings::default();
    settings.dns.enabled = true;
    settings.dns.servers = vec![
        DnsServerConfig {
            tag: "google".to_string(),
            protocol: DnsProtocol::Doh,
            address: "dns.google".to_string(),
            port: None,
            detour: None,
        },
        DnsServerConfig {
            tag: "cloudflare".to_string(),
            protocol: DnsProtocol::Doh,
            address: "cloudflare-dns.com".to_string(),
            port: None,
            detour: None,
        },
    ];

    check("hostname-only-dns-servers", &settings);
}

/// v2fly/xray bundle RFC 1918 private ranges as GeoIP "private"; SagerNet's
/// sing-geoip mirror ships no such .srs, so treating it like any other
/// country code makes sing-box try to download a file that 404s. Must become
/// `ip_is_private` instead, which needs no rule-set at all.
#[test]
fn geoip_private_route_rule_passes_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let rules = vec![RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::GeoIp {
            country_code: "private".into(),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    }];

    check_with_rules("geoip-private", &AppSettings::default(), &rules, |_| {});
}

/// Rule-set config with the experimental cache_file the writer injects — the
/// exact shape a GeoIP/GeoSite routing setup produces after
/// fix-singbox-ruleset-offline-start.
#[test]
fn ruleset_config_with_cache_file_passes_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let rules = vec![
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
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoSite {
                category: "yandex".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        },
    ];

    let mut tun_settings = AppSettings::default();
    tun_settings.tun.enabled = true;

    let cache_dir = tempfile::TempDir::new().unwrap();
    let cache_path = cache_dir.path().join("sing-box-cache.db");
    for (name, settings) in [
        ("rulesets-plain", AppSettings::default()),
        ("rulesets-tun", tun_settings),
    ] {
        check_with_rules(name, &settings, &rules, |config| {
            config["experimental"]["cache_file"] = serde_json::json!({
                "enabled": true,
                "path": cache_path,
            });
        });
    }
}

/// A config generated through the writer with a cached `.srs` rule-set on
/// disk resolves the matching `route.rule_set` entry to `local`, letting
/// sing-box start without fetching it over the network.
#[test]
fn local_ruleset_config_passes_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    let rule_sets_dir = paths.geodata_dir().join("rule-sets");
    std::fs::create_dir_all(&rule_sets_dir).unwrap();

    let source = tmp.path().join("geoip-ru.json");
    std::fs::write(
        &source,
        br#"{"version":1,"rules":[{"ip_cidr":["1.1.1.1/32"]}]}"#,
    )
    .unwrap();
    let compile = Command::new("sing-box")
        .args(["rule-set", "compile"])
        .arg(&source)
        .arg("-o")
        .arg(rule_sets_dir.join("geoip-ru.srs"))
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "rule-set compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let mut settings = AppSettings::default();
    settings.backend.backend_type = BackendType::SingBox;
    let rules = vec![RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::GeoIp {
            country_code: "RU".into(),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    }];

    let writer = ConfigWriter::new(&settings, &paths);
    let path = writer
        .write_config(&[ss_node()], &rules, &settings)
        .unwrap();
    let json = std::fs::read_to_string(&path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&json).unwrap();

    let rule_sets = config["route"]["rule_set"].as_array().unwrap();
    let entry = rule_sets.iter().find(|r| r["tag"] == "geoip-ru").unwrap();
    assert_eq!(entry["type"], "local");

    let output = Command::new("sing-box")
        .arg("check")
        .arg("-c")
        .arg(&path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "local ruleset config: sing-box check failed\nstderr: {}\nconfig: {json}",
        String::from_utf8_lossy(&output.stderr),
    );
}
