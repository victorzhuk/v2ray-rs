use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use v2ray_rs_core::models::{ProxyNode, TransportSettings};
use v2ray_rs_subscription::parse_json_subscription;

const BUNDLE_SAMPLE: &str = include_str!("fixtures/bundle_sample.json");

#[test]
fn bundle_fixture_yields_24_nodes_with_expected_transport_mix() {
    let import = parse_json_subscription(BUNDLE_SAMPLE).expect("bundle format detected");

    assert_eq!(import.result.errors.len(), 0, "{:?}", import.result.errors);
    assert_eq!(import.result.nodes.len(), 24);

    let mut xhttp = 0;
    let mut tls = 0;
    let mut reality = 0;
    for node in &import.result.nodes {
        let ProxyNode::Vless(cfg) = &node.node else {
            panic!("expected all fixture nodes to be vless");
        };
        if matches!(cfg.transport, TransportSettings::Xhttp(_)) {
            xhttp += 1;
        }
        if let Some(t) = &cfg.tls {
            if t.reality {
                reality += 1;
            } else {
                tls += 1;
            }
        }
    }

    assert_eq!(xhttp, 5, "expected 5 xhttp nodes");
    assert_eq!(tls, 1, "expected 1 plain-tls node");
    assert_eq!(reality, 5, "expected 5 reality nodes");

    let profile = import.profile.expect("profile extracted from first entry");
    assert_eq!(profile.rules.len(), 4, "{:?}", profile.rules);
    assert!(
        profile
            .skipped
            .iter()
            .any(|warning| warning.contains("dns-out")),
        "{:?}",
        profile.skipped
    );
    assert!(
        profile
            .skipped
            .iter()
            .any(|warning| warning.contains("no supported match condition")),
        "{:?}",
        profile.skipped
    );
    let dns = profile.dns.expect("dns profile extracted");
    assert_eq!(dns.servers.len(), 4);
    assert!(dns.enabled);
    assert!(dns.use_custom_rules);
}

#[test]
fn base64_wrapped_bundle_is_detected() {
    let encoded = STANDARD.encode(BUNDLE_SAMPLE);
    let import = parse_json_subscription(&encoded).expect("base64-wrapped bundle detected");
    assert_eq!(import.result.nodes.len(), 24);
}

#[test]
fn base64_uri_list_is_not_a_bundle() {
    let uris = "vless://550e8400-e29b-41d4-a716-446655440000@example.com:443#Test\nss://YWVzLTI1Ni1nY206c2VjcmV0@ss.example.com:8388#SS";
    let encoded = STANDARD.encode(uris);
    assert!(
        parse_json_subscription(&encoded).is_none(),
        "a base64 URI list must fall through to the URI parser, not the JSON importer"
    );
}

#[test]
fn bare_object_single_config_is_accepted() {
    let single = serde_json::json!({
        "remarks": "Solo Node",
        "dns": { "servers": ["1.1.1.1"] },
        "routing": { "rules": [] },
        "inbounds": [],
        "outbounds": [
            {
                "tag": "proxy",
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": "solo.example.net",
                        "port": 443,
                        "users": [{"id": "00000000-0000-4000-8000-000000000099", "encryption": "none"}]
                    }]
                },
                "streamSettings": { "network": "tcp" }
            },
            {"tag": "direct", "protocol": "freedom", "settings": {}}
        ]
    });

    let import =
        parse_json_subscription(&single.to_string()).expect("bare object accepted as a bundle");
    assert_eq!(import.result.nodes.len(), 1);
    assert_eq!(import.result.nodes[0].node.address(), "solo.example.net");
}

#[test]
fn unsupported_transport_lands_in_errors_not_nodes() {
    let bundle = serde_json::json!([
        {
            "remarks": "Bad Transport",
            "dns": {},
            "routing": { "rules": [] },
            "inbounds": [],
            "outbounds": [
                {
                    "tag": "proxy",
                    "protocol": "vless",
                    "settings": {
                        "vnext": [{
                            "address": "kcp.example.net",
                            "port": 443,
                            "users": [{"id": "00000000-0000-4000-8000-000000000098", "encryption": "none"}]
                        }]
                    },
                    "streamSettings": { "network": "kcp" }
                },
                {"tag": "direct", "protocol": "freedom", "settings": {}}
            ]
        }
    ]);

    let import = parse_json_subscription(&bundle.to_string()).expect("bundle format detected");
    assert_eq!(import.result.nodes.len(), 0);
    assert_eq!(import.result.errors.len(), 1);
    assert!(import.result.errors[0].1.to_string().contains("kcp"));
}

fn vless_outbound(tag: &str, address: &str, stream: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "tag": tag,
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": address,
                "port": 443,
                "users": [{"id": "00000000-0000-4000-8000-000000000097", "encryption": "none"}]
            }]
        },
        "streamSettings": stream
    })
}

#[test]
fn allow_insecure_json_outbound_is_skipped_with_warning() {
    let bundle = serde_json::json!([
        {
            "remarks": "Insecure",
            "outbounds": [
                vless_outbound(
                    "proxy",
                    "insecure.example.net",
                    serde_json::json!({
                        "network": "tcp",
                        "security": "tls",
                        "tlsSettings": {"allowInsecure": true}
                    })
                )
            ]
        },
        {
            "remarks": "Valid",
            "outbounds": [
                vless_outbound(
                    "proxy",
                    "valid.example.net",
                    serde_json::json!({
                        "network": "tcp",
                        "security": "tls",
                        "tlsSettings": {"allowInsecure": false}
                    })
                )
            ]
        }
    ]);

    let import = parse_json_subscription(&bundle.to_string()).expect("bundle detected");
    assert_eq!(import.result.nodes.len(), 1);
    assert_eq!(import.result.nodes[0].node.address(), "valid.example.net");
    assert_eq!(import.result.errors.len(), 1);
    assert!(
        import.result.errors[0]
            .1
            .to_string()
            .contains("allowInsecure")
    );
}

#[test]
fn multi_axis_and_dns_outbound_rules_are_dropped_whole() {
    let bundle = serde_json::json!({
        "remarks": "Rules",
        "dns": {},
        "inbounds": [],
        "outbounds": [
            vless_outbound(
                "proxy",
                "rules.example.net",
                serde_json::json!({"network": "tcp"})
            ),
            {"tag": "direct", "protocol": "freedom", "settings": {}},
            {"tag": "block", "protocol": "blackhole", "settings": {}},
            {"tag": "dns-out", "protocol": "dns"}
        ],
        "routing": {
            "rules": [
                {"type": "field", "outboundTag": "dns-out", "port": "53"},
                {"type": "field", "outboundTag": "block", "network": "udp", "port": "443"},
                {"type": "field", "outboundTag": "block", "domain": ["regexp:.*"], "ip": ["geoip:private"]},
                {"type": "field", "outboundTag": "direct", "ip": ["geoip:private"], "network": "tcp,udp"},
                {"type": "field", "outboundTag": "direct", "ip": ["999.1.1.1/24"]},
                {"type": "field", "outboundTag": "proxy", "domain": ["domain:bad_domain.com"]}
            ]
        }
    });

    let import = parse_json_subscription(&bundle.to_string()).expect("bundle detected");
    let profile = import.profile.expect("profile extracted");
    assert_eq!(profile.rules.len(), 1, "{:?}", profile.rules);
    assert_eq!(profile.rules[0].action, v2ray_rs_core::models::RuleAction::Direct);
    assert!(matches!(
        profile.rules[0].match_condition,
        v2ray_rs_core::models::RuleMatch::GeoIp { .. }
    ));
    for expected in [
        "dns-out",
        "combines 2 match conditions",
        "regexp:",
        "invalid ip matcher",
        "invalid domain pattern",
    ] {
        assert!(
            profile
                .skipped
                .iter()
                .any(|warning| warning.contains(expected)),
            "missing warning {expected:?}: {:?}",
            profile.skipped
        );
    }
}

#[test]
fn invalid_dns_servers_and_hosts_are_dropped_with_warnings() {
    let bundle = serde_json::json!({
        "remarks": "DNS",
        "outbounds": [
            vless_outbound(
                "proxy",
                "dns.example.net",
                serde_json::json!({"network": "tcp"})
            )
        ],
        "dns": {
            "servers": [
                "1.1.1.1",
                "999.999.999.999",
                {"address": "bad host", "domains": ["domain:example.com"]},
                {"address": "8.8.8.8", "domains": ["domain:bad_domain.com"]}
            ],
            "hosts": {
                "ok.example.com": "1.2.3.4",
                "bad.example.com": "not-an-ip"
            }
        },
        "routing": {"rules": []}
    });

    let import = parse_json_subscription(&bundle.to_string()).expect("bundle detected");
    let profile = import.profile.expect("profile extracted");
    let dns = profile.dns.expect("dns profile kept");
    assert_eq!(dns.servers.len(), 2, "{:?}", dns.servers);
    assert_eq!(dns.rules.len(), 0, "{:?}", dns.rules);
    assert_eq!(dns.hosts.len(), 1, "{:?}", dns.hosts);
    assert_eq!(dns.hosts[0].domain, "ok.example.com");
    for expected in ["invalid address 'bad host'", "invalid entry", "invalid domain matcher"] {
        assert!(
            profile
                .skipped
                .iter()
                .any(|warning| warning.contains(expected)),
            "missing warning {expected:?}: {:?}",
            profile.skipped
        );
    }
}
