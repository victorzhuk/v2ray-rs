use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use v2ray_rs_core::config::{ConfigWriter, ResolvedNodeSet, SingboxGenerator};
use v2ray_rs_core::models::{
    AppSettings, BackendLogLevel, BackendType, DnsConfig, DnsHijackMode, DnsProtocol,
    DnsServerConfig, FakeIpConfig, HostOverride, ProxyNode, RoutingRule, RuleAction, RuleMatch,
    ShadowsocksConfig,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::AppProfile;

fn sing_box_available() -> bool {
    let available = Command::new("sing-box")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        available || std::env::var_os("V2RAY_RS_REQUIRE_BACKENDS").is_none(),
        "required backend sing-box is unavailable"
    );
    available
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

/// `sing-box check` validates the schema but starts no services, so it accepts
/// configurations the daemon then refuses to run - a DNS server detoured to the
/// empty direct outbound is one. This starts the real thing and fails on any
/// FATAL. The TUN inbound is dropped because creating the device needs
/// CAP_NET_ADMIN, which the test runner does not have; everything the DNS plane
/// is built from survives that edit.
fn check_starts(name: &str, settings: &AppSettings, slot: u16) {
    let mut config = SingboxGenerator
        .generate(&[ss_node()], &[], settings)
        .unwrap_or_else(|err| panic!("{name}: generate failed: {err}"));

    if let Some(inbounds) = config["inbounds"].as_array_mut() {
        inbounds.retain(|i| i["type"] != "tun");
        for (i, inbound) in inbounds.iter_mut().enumerate() {
            inbound["listen_port"] = serde_json::json!(38000 + slot * 10 + i as u16);
        }
    }
    let json = serde_json::to_string_pretty(&config).unwrap();

    let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    file.write_all(json.as_bytes()).unwrap();
    file.flush().unwrap();

    let mut child = Command::new("sing-box")
        .arg("run")
        .arg("-c")
        .arg(file.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(1500));
    let running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        running && !log.contains("FATAL"),
        "{name}: sing-box refused to start\nlog: {log}\nconfig: {json}",
    );
}

#[test]
fn generated_singbox_configs_start() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let mut cases: Vec<(&str, AppSettings)> = Vec::new();

    // No DNS section at all. The daemon refuses to start when one exists
    // without a resolver, so the absence of the whole section is worth pinning.
    cases.push(("plain", AppSettings::default()));

    let mut pinned_no_dns = AppSettings::default();
    pinned_no_dns.dns.hosts = vec![HostOverride {
        domain: "ss.example.com".to_string(),
        ip: "203.0.113.9".to_string(),
    }];
    cases.push(("pinned-without-dns-section", pinned_no_dns));

    let mut detour_direct = AppSettings::default();
    detour_direct.dns.enabled = true;
    detour_direct.dns.servers = vec![DnsServerConfig {
        tag: "domestic".to_string(),
        protocol: DnsProtocol::Udp,
        address: "77.88.8.8".to_string(),
        port: None,
        detour: Some("direct".to_string()),
    }];
    cases.push(("detour-direct", detour_direct));

    let mut detour_proxy = AppSettings::default();
    detour_proxy.dns.enabled = true;
    detour_proxy.dns.servers = vec![DnsServerConfig {
        tag: "remote".to_string(),
        protocol: DnsProtocol::Doh,
        address: "1.1.1.1".to_string(),
        port: None,
        detour: Some("proxy".to_string()),
    }];
    cases.push(("detour-proxy", detour_proxy));

    let mut tun_derived = AppSettings::default();
    tun_derived.tun.enabled = true;
    tun_derived.dns.hosts = vec![HostOverride {
        domain: "ss.example.com".to_string(),
        ip: "203.0.113.9".to_string(),
    }];
    cases.push(("tun-derived-pinned", tun_derived));

    let mut tun_dns = AppSettings::default();
    tun_dns.tun.enabled = true;
    tun_dns.dns.enabled = true;
    tun_dns.dns.servers = dns_servers();
    cases.push(("tun-dns", tun_dns));

    for (slot, (name, settings)) in cases.iter().enumerate() {
        check_starts(name, settings, slot as u16);
    }
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

    let mut log_debug = AppSettings::default();
    log_debug.logging.backend_level = BackendLogLevel::Debug;
    log_debug.logging.connection_log = true;
    cases.push(("log-debug", log_debug));

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

/// Under TUN with DNS enabled, a proxy-detoured DoH server leaves no IP-literal
/// direct candidate for `route.default_domain_resolver`, so the generator
/// injects the `sys-dns-bootstrap` HTTPS resolver. `sing-box check` must accept
/// those exact field names, and a `local` resolver is forbidden there: the OS
/// resolver itself routes through the tunnel, so it would resolve in a circle.
#[test]
fn tun_dns_bootstrap_passes_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let mut settings = AppSettings::default();
    settings.dns.enabled = true;
    settings.dns.servers = vec![
        DnsServerConfig {
            tag: "remote".to_string(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".to_string(),
            port: None,
            detour: Some("proxy".to_string()),
        },
        DnsServerConfig {
            tag: "remote-host".to_string(),
            protocol: DnsProtocol::Doh,
            address: "dns.google".to_string(),
            port: None,
            detour: Some("direct".to_string()),
        },
    ];
    settings.tun.enabled = true;

    check_with_rules("tun-dns-bootstrap", &settings, &[], |config| {
        assert_eq!(
            config["route"]["default_domain_resolver"], "sys-dns-bootstrap",
            "route.default_domain_resolver must point at the bootstrap: {config}"
        );
        let servers = config["dns"]["servers"].as_array().unwrap();
        let hostname_server = servers
            .iter()
            .find(|s| s["tag"] == "remote-host")
            .unwrap_or_else(|| panic!("remote-host missing from dns.servers: {config}"));
        assert_eq!(
            hostname_server["domain_resolver"], "sys-dns-bootstrap",
            "hostname-addressed server must resolve via the bootstrap: {config}"
        );
        let bootstrap = servers
            .iter()
            .find(|s| s["tag"] == "sys-dns-bootstrap")
            .unwrap_or_else(|| panic!("sys-dns-bootstrap missing from dns.servers: {config}"));
        assert_eq!(bootstrap["type"], "https");
        assert_eq!(bootstrap["server"], "1.1.1.1");
        assert_eq!(bootstrap["server_port"], 443);
        assert_eq!(bootstrap["path"], "/dns-query");
        assert!(
            bootstrap.get("detour").is_none(),
            "sys-dns-bootstrap must dial direct"
        );
        assert!(
            servers.iter().all(|s| s["type"] != "local"),
            "local resolver inside the tunnel resolves through itself"
        );
    });
}

/// Auto-derived DNS rules must only name server tags that actually exist in
/// dns.servers: with no server tagged `domestic`, a Direct GeoSite rule used to
/// produce a rule pointing at a missing server.
#[test]
fn missing_domestic_tag_derived_rules_pass_sing_box_check() {
    if !sing_box_available() {
        eprintln!("sing-box not found in PATH, skipping");
        return;
    }

    let mut settings = AppSettings::default();
    settings.dns.enabled = true;
    settings.dns.use_custom_rules = false;
    settings.dns.servers = vec![
        DnsServerConfig {
            tag: "remote".to_string(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".to_string(),
            port: None,
            detour: None,
        },
        DnsServerConfig {
            tag: "lan".to_string(),
            protocol: DnsProtocol::Udp,
            address: "223.5.5.5".to_string(),
            port: None,
            detour: None,
        },
    ];
    let rules = vec![RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::GeoSite {
            category: "category-ru".into(),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    }];

    check_with_rules("missing domestic tag", &settings, &rules, |config| {
        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        assert!(
            !dns_rules.iter().any(|r| r["server"] == "domestic"),
            "no dns rule may reference the absent 'domestic' tag: {dns_rules:?}"
        );
    });
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
        .write_config(
            &ResolvedNodeSet::from_nodes(&[ss_node()]).unwrap(),
            &rules,
            &settings,
            &[],
        )
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

const ROUNDTRIP_PASSWORD: &str = "roundtrip-pass";
const ROUNDTRIP_BODY: &str = "roundtrip-ok-body";

/// Takes `count` distinct loopback ports from the OS, then drops the
/// listeners: parallel or repeated runs must never collide on fixed ports.
fn ephemeral_ports(count: usize) -> Vec<u16> {
    let listeners: Vec<std::net::TcpListener> = (0..count)
        .map(|_| {
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback port")
        })
        .collect();
    listeners
        .iter()
        .map(|l| l.local_addr().expect("listener local address").port())
        .collect()
}

fn python3_available() -> bool {
    let available = Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        available || std::env::var_os("V2RAY_RS_REQUIRE_BACKENDS").is_none(),
        "required backend python3 is unavailable"
    );
    available
}

/// Kills the backend on drop, so a failing assert never leaks listeners into
/// later tests sharing the process.
struct ManagedChild(Option<std::process::Child>);

impl ManagedChild {
    /// Kills the backend and returns everything it wrote to its pipes.
    fn drain(mut self) -> String {
        let Some(child) = self.0.as_mut() else {
            return String::new();
        };
        let _ = child.kill();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let _ = child.wait();
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stdout {
            let _ = pipe.read_to_end(&mut bytes);
        }
        if let Some(mut pipe) = stderr {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn config_tempfile(config: &serde_json::Value) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    file.write_all(&serde_json::to_vec(config).unwrap())
        .unwrap();
    file.flush().unwrap();
    file
}

fn wait_listening(port: u16, deadline: Instant, config: impl Fn() -> String) {
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "nothing listened on 127.0.0.1:{port} in time\nconfig: {}",
        config()
    );
}

fn target_http_script(target_port: u16) -> String {
    format!(
        r#"import http.server

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"{ROUNDTRIP_BODY}"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass

http.server.HTTPServer(("127.0.0.1", {target_port}), H).serve_forever()
"#
    )
}

/// Speaks HTTP-proxy to the mixed inbound: absolute-form GET, then reads until
/// the server closes the connection. The read deadline is what makes a
/// resolver-recursion deadlock fail the test instead of hanging it.
fn proxied_get(proxy_port: u16, target: &str, deadline: Instant) -> Result<String, String> {
    let addr: SocketAddr = format!("127.0.0.1:{proxy_port}")
        .parse()
        .expect("parse loopback address");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("connect to mixed inbound: {e}"))?;
    stream
        .set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(200)),
        ))
        .map_err(|e| format!("set read timeout: {e}"))?;
    let request = format!(
        "GET http://{target}/probe HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("send request: {e}"))?;

    let mut response = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("read timed out waiting for the proxied response".into());
        }
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("read response: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&response).into_owned())
}

/// The regression this DNS plane exists to prevent: with DNS enabled and TUN
/// off, a hostname-addressed proxy node ("localhost" here) used to be resolved
/// through the proxy-detoured DoH server itself - the "remote" -> "proxy-0"
/// dial loop - so every proxied connection hung forever while `sing-box check`
/// and inbound startup succeeded. The generated config must resolve the node
/// through the direct "domestic" server, and a real request must complete.
#[test]
fn loopback_proxy_roundtrip_succeeds_with_default_dns() {
    if !sing_box_available() || !python3_available() {
        eprintln!("sing-box or python3 not found in PATH, skipping");
        return;
    }

    let settings = AppSettings {
        dns: DnsConfig {
            enabled: true,
            ..DnsConfig::default()
        },
        ..Default::default()
    };
    let ports = ephemeral_ports(4);
    let mixed_port = ports[0];
    let ss_port = ports[1];
    let target_port = ports[2];
    // The second generated inbound (plain HTTP) takes ports[3].
    let spare_port = ports[3];

    let node = ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: "localhost".into(),
        port: ss_port,
        method: "aes-256-gcm".into(),
        password: ROUNDTRIP_PASSWORD.into(),
        remark: Some("Roundtrip SS".into()),
    });
    let mut config = SingboxGenerator
        .generate(&[node], &[], &settings)
        .unwrap_or_else(|err| panic!("generate failed: {err}"));

    assert_eq!(
        config["route"]["default_domain_resolver"], "domestic",
        "resolver must be the direct IP-literal server, never the proxy-detoured one"
    );
    assert!(
        config["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["tag"] != "sys-dns-bootstrap"),
        "the direct 'domestic' server makes the bootstrap resolver unnecessary"
    );

    if let Some(inbounds) = config["inbounds"].as_array_mut() {
        for (i, inbound) in inbounds.iter_mut().enumerate() {
            inbound["listen_port"] =
                serde_json::json!(if i == 0 { mixed_port } else { spare_port });
        }
    }
    let json = serde_json::to_string_pretty(&config).unwrap();

    let mut script = tempfile::Builder::new().suffix(".py").tempfile().unwrap();
    script
        .write_all(target_http_script(target_port).as_bytes())
        .unwrap();
    script.flush().unwrap();

    let fixture = config_tempfile(&serde_json::json!({
        "log": {"level": "warning"},
        "inbounds": [{
            "type": "shadowsocks",
            "tag": "ss-in",
            "listen": "127.0.0.1",
            "listen_port": ss_port,
            "method": "aes-256-gcm",
            "password": ROUNDTRIP_PASSWORD,
        }],
        "outbounds": [{"type": "direct", "tag": "direct"}],
        "route": {"final": "direct"},
    }));
    let generated = config_tempfile(&config);

    let check = Command::new("sing-box")
        .args(["check", "-c"])
        .arg(generated.path())
        .output()
        .expect("run sing-box check on the generated config");
    assert!(
        check.status.success(),
        "generated round-trip config failed sing-box check\nstderr: {}\nconfig: {json}",
        String::from_utf8_lossy(&check.stderr),
    );

    let _target = ManagedChild(Some(
        Command::new("python3")
            .arg(script.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loopback HTTP target"),
    ));
    let _ss_server = ManagedChild(Some(
        Command::new("sing-box")
            .args(["run", "-c"])
            .arg(fixture.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loopback shadowsocks server"),
    ));
    let client = ManagedChild(Some(
        Command::new("sing-box")
            .args(["run", "-c"])
            .arg(generated.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sing-box under test"),
    ));

    let deadline = Instant::now() + Duration::from_secs(15);
    let config_dump = || json.clone();
    wait_listening(ss_port, deadline, config_dump);
    wait_listening(mixed_port, deadline, config_dump);

    let target = format!("127.0.0.1:{target_port}");
    match proxied_get(mixed_port, &target, deadline) {
        Ok(response) => {
            let status = response.split("\r\n").next().unwrap_or_default();
            assert!(
                status.starts_with("HTTP/") && status.contains(" 200 "),
                "unexpected proxied response: {response}"
            );
            assert!(
                response.contains(ROUNDTRIP_BODY),
                "proxied response body missing: {response}"
            );
        }
        Err(err) => {
            let logs = client.drain();
            panic!("proxied round-trip failed: {err}\nsing-box log: {logs}\nconfig: {json}");
        }
    }
}
