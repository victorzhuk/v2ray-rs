//! Ephemeral "Real Delay" probe config generation.
//!
//! These generators emit a minimal backend config used by a short-lived,
//! isolated backend instance to measure end-to-end delay through each candidate
//! node, to observe each node's egress, and to run availability checks through
//! each node's own tunnel. The generated config has no user-facing inbound
//! (only a loopback API listener plus one loopback HTTP inbound per tested
//! node), no routing rules beyond inbound-to-outbound mappings, and tags every
//! probed outbound as `probe-<idx>` for stable mapping back to the node index.

use std::collections::HashMap;
use std::net::TcpListener;

use serde_json::{Value, json};

use crate::models::{BackendType, SubscriptionNode};

/// Tag prefix used for every probed outbound so results map back to the node
/// index deterministically.
pub const PROBE_TAG_PREFIX: &str = "probe-";

/// Tag prefix of the per-node loopback HTTP inbound used for requests that must
/// travel through a specific node's tunnel.
pub const EGRESS_TAG_PREFIX: &str = "egress-";

/// Returns the stable outbound tag for the node at `idx`.
#[must_use]
pub fn probe_tag(idx: usize) -> String {
    format!("{PROBE_TAG_PREFIX}{idx}")
}

/// Returns the stable tag of the per-node egress HTTP inbound for the node at
/// `idx`.
#[must_use]
pub fn egress_tag(idx: usize) -> String {
    format!("{EGRESS_TAG_PREFIX}{idx}")
}

/// Allocates `n` distinct loopback ports by binding that many sockets at once,
/// recording each port, and closing all sockets. Distinctness is guaranteed by
/// the simultaneous binds; the OS may reassign a port between close and
/// backend start, which is the same benign race the probe API port accepts.
#[must_use]
pub fn pick_free_loopback_ports(n: usize) -> Option<Vec<u16>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let listeners: Vec<TcpListener> = (0..n)
        .map(|_| TcpListener::bind("127.0.0.1:0"))
        .collect::<Result<_, _>>()
        .ok()?;
    let ports: Vec<u16> = listeners
        .iter()
        .filter_map(|l| l.local_addr().ok().map(|a| a.port()))
        .collect();
    let distinct = ports.iter().collect::<std::collections::HashSet<_>>().len() == n;
    drop(listeners);
    distinct.then_some(ports)
}

/// Extracts the loopback port of every egress inbound present in a generated
/// probe config, keyed by the node index parsed from the `egress-<idx>` tag.
/// Nodes whose outbound a generator dropped have no entry.
#[must_use]
pub fn egress_inbound_ports(config: &Value) -> HashMap<usize, u16> {
    let Some(inbounds) = config.get("inbounds").and_then(Value::as_array) else {
        return HashMap::new();
    };
    inbounds
        .iter()
        .filter_map(|inbound| {
            let tag = inbound.get("tag")?.as_str()?;
            let idx = tag.strip_prefix(EGRESS_TAG_PREFIX)?.parse::<usize>().ok()?;
            let port = inbound
                .get("listen_port")
                .or_else(|| inbound.get("port"))
                .and_then(Value::as_u64)? as u16;
            Some((idx, port))
        })
        .collect()
}

pub trait ProbeConfigGenerator: Send + Sync {
    /// Generates a probe config exposing an API on `127.0.0.1:<api_port>` with
    /// every node in `nodes` present as an outbound tagged `probe-<idx>`, plus
    /// one loopback HTTP inbound per emittable node on `egress_ports[idx]`
    /// routed exclusively to that node's outbound.
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        test_url: &str,
        timeout_ms: u32,
        egress_ports: &[u16],
    ) -> Value;

    fn outbound_tag(&self, idx: usize) -> String {
        probe_tag(idx)
    }
}

/// Returns a probe generator for the backend, or `None` if the backend does not
/// support Real Delay probing (e.g. legacy v2ray-core without observatory).
#[must_use]
pub fn probe_generator_for(backend: BackendType) -> Option<Box<dyn ProbeConfigGenerator>> {
    match backend {
        BackendType::SingBox => Some(Box::new(SingboxProbeGenerator)),
        BackendType::V2ray => Some(Box::new(V2rayProbeGenerator)),
        BackendType::Xray => Some(Box::new(XrayProbeGenerator)),
    }
}

pub struct SingboxProbeGenerator;

impl ProbeConfigGenerator for SingboxProbeGenerator {
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        _test_url: &str,
        _timeout_ms: u32,
        egress_ports: &[u16],
    ) -> Value {
        // A node whose transport sing-box can't emit (xhttp) just can't be
        // probed on this backend; drop it rather than fail the whole batch.
        // Safe because results are matched back by parsing the index out of
        // the outbound tag, not by position.
        let built: Vec<(usize, Value)> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                crate::config::singbox::build_singbox_outbound(&node.node, &probe_tag(i))
                    .ok()
                    .map(|outbound| (i, outbound))
            })
            .collect();

        let mut inbounds: Vec<Value> = Vec::new();
        let mut route_rules: Vec<Value> = Vec::new();
        for (i, _outbound) in &built {
            let Some(&port) = egress_ports.get(*i) else {
                continue;
            };
            inbounds.push(json!({
                "type": "http",
                "tag": egress_tag(*i),
                "listen": "127.0.0.1",
                "listen_port": port,
            }));
            route_rules.push(json!({
                "inbound": [egress_tag(*i)],
                "outbound": probe_tag(*i),
            }));
        }

        json!({
            "log": { "level": "warn" },
            "outbounds": built.into_iter().map(|(_, ob)| ob).collect::<Vec<_>>(),
            "inbounds": inbounds,
            "experimental": {
                "clash_api": {
                    "external_controller": format!("127.0.0.1:{api_port}"),
                }
            },
            "route": {
                "rules": route_rules,
            }
        })
    }
}

pub struct XrayProbeGenerator;

impl ProbeConfigGenerator for XrayProbeGenerator {
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        test_url: &str,
        timeout_ms: u32,
        egress_ports: &[u16],
    ) -> Value {
        let outbounds: Vec<Value> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| crate::config::xray::build_xray_outbound(&node.node, &probe_tag(i)))
            .collect();

        let (egress_inbounds, egress_rules) =
            egress_inbounds_and_rules(nodes, egress_ports, |_| true);

        let mut inbounds = vec![json!({
            "tag": "api-in",
            "listen": "127.0.0.1",
            "port": api_port,
            "protocol": "dokodemo-door",
            "settings": { "address": "127.0.0.1" },
        })];
        inbounds.extend(egress_inbounds);

        let mut routing_rules = vec![json!({
            "type": "field",
            "inboundTag": ["api-in"],
            "outboundTag": "api",
        })];
        routing_rules.extend(egress_rules);

        json!({
            "log": { "loglevel": "warning" },
            "outbounds": outbounds,
            "inbounds": inbounds,
            "observatory": {
                "subjectSelector": [PROBE_TAG_PREFIX],
                "probeUrl": test_url,
                "probeInterval": "500ms",
            },
            "burstObservatory": {
                "subjectSelector": [PROBE_TAG_PREFIX],
                "pingConfig": {
                    "destination": test_url,
                    "interval": "500ms",
                    "timeout": format!("{timeout_ms}ms"),
                    "sampling": 1,
                    "httpMethod": "HEAD",
                }
            },
            "stats": {},
            "api": {
                "tag": "api",
                "services": ["StatsService", "ObservatoryService"],
            },
            "routing": {
                "rules": routing_rules,
            }
        })
    }
}

/// Builds the per-node loopback HTTP inbounds and their inbound-to-outbound
/// routing rules in the xray/v2ray shape, one per node index in `emittable`
/// whose outbound the generator actually emits.
fn egress_inbounds_and_rules(
    nodes: &[&SubscriptionNode],
    egress_ports: &[u16],
    emittable: impl Fn(usize) -> bool,
) -> (Vec<Value>, Vec<Value>) {
    let mut inbounds = Vec::new();
    let mut rules = Vec::new();
    for (i, _) in nodes.iter().enumerate() {
        if !emittable(i) {
            continue;
        }
        let Some(&port) = egress_ports.get(i) else {
            continue;
        };
        inbounds.push(json!({
            "tag": egress_tag(i),
            "listen": "127.0.0.1",
            "port": port,
            "protocol": "http",
            "settings": {},
        }));
        rules.push(json!({
            "type": "field",
            "inboundTag": [egress_tag(i)],
            "outboundTag": probe_tag(i),
        }));
    }
    (inbounds, rules)
}

pub struct V2rayProbeGenerator;

impl ProbeConfigGenerator for V2rayProbeGenerator {
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        test_url: &str,
        timeout_ms: u32,
        egress_ports: &[u16],
    ) -> Value {
        let outbounds: Vec<Value> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| crate::config::v2ray::v2ray_supports(&node.node))
            .map(|(i, node)| {
                crate::config::v2ray::build_family_outbound(
                    &node.node,
                    &probe_tag(i),
                    crate::config::v2ray::V2rayFamilyBackend::V2ray,
                )
            })
            .collect();

        let (egress_inbounds, egress_rules) = egress_inbounds_and_rules(nodes, egress_ports, |i| {
            nodes
                .get(i)
                .is_some_and(|n| crate::config::v2ray::v2ray_supports(&n.node))
        });

        let mut inbounds = vec![json!({
            "tag": "api-in",
            "listen": "127.0.0.1",
            "port": api_port,
            "protocol": "dokodemo-door",
            "settings": { "address": "127.0.0.1" },
        })];
        inbounds.extend(egress_inbounds);

        let mut routing_rules = vec![json!({
            "type": "field",
            "inboundTag": ["api-in"],
            "outboundTag": "api",
        })];
        routing_rules.extend(egress_rules);

        json!({
            "log": { "loglevel": "warning" },
            "outbounds": outbounds,
            "inbounds": inbounds,
            "burstObservatory": {
                "subjectSelector": [PROBE_TAG_PREFIX],
                "pingConfig": {
                    "destination": test_url,
                    "interval": "500ms",
                    "timeout": format!("{timeout_ms}ms"),
                    "sampling": 1,
                }
            },
            "stats": {},
            "api": {
                "tag": "api",
                "services": ["StatsService", "ObservatoryService"],
            },
            "routing": {
                "rules": routing_rules,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProxyNode;
    use crate::models::{
        ShadowsocksConfig, SubscriptionNode, TlsSettings, TransportSettings, TrojanConfig,
        VlessConfig, VmessConfig,
    };

    fn mixed_nodes() -> Vec<SubscriptionNode> {
        vec![
            SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
                address: "vless.example.com".into(),
                port: 443,
                uuid: "uuid-1".into(),
                encryption: None,
                flow: Some("xtls-rprx-vision".into()),
                transport: TransportSettings::Tcp,
                tls: Some(TlsSettings {
                    server_name: Some("vless.example.com".into()),
                    ..Default::default()
                }),
                remark: Some("VLESS".into()),
            })),
            SubscriptionNode::new(ProxyNode::Vmess(VmessConfig {
                address: "vmess.example.com".into(),
                port: 443,
                uuid: "uuid-2".into(),
                alter_id: 0,
                security: "auto".into(),
                transport: TransportSettings::Tcp,
                tls: None,
                remark: Some("VMess".into()),
            })),
            SubscriptionNode::new(ProxyNode::Trojan(TrojanConfig {
                address: "trojan.example.com".into(),
                port: 443,
                password: "secret".into(),
                transport: TransportSettings::Tcp,
                tls: Some(TlsSettings::default()),
                remark: Some("Trojan".into()),
            })),
            SubscriptionNode::new(ProxyNode::Shadowsocks(ShadowsocksConfig {
                address: "ss.example.com".into(),
                port: 8388,
                method: "aes-256-gcm".into(),
                password: "pass".into(),
                remark: Some("SS".into()),
            })),
        ]
    }

    #[test]
    fn probe_generator_for_backends() {
        assert!(probe_generator_for(BackendType::SingBox).is_some());
        assert!(probe_generator_for(BackendType::Xray).is_some());
        assert!(probe_generator_for(BackendType::V2ray).is_some());
    }

    #[test]
    fn probe_configs_keep_fixed_log_objects() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let expected = [
            (BackendType::SingBox, json!({ "level": "warn" })),
            (BackendType::Xray, json!({ "loglevel": "warning" })),
            (BackendType::V2ray, json!({ "loglevel": "warning" })),
        ];
        for (backend, log) in expected {
            let config = probe_generator_for(backend).unwrap().generate(
                &refs,
                19_080,
                "https://www.gstatic.com/generate_204",
                5000,
                &[],
            );
            assert_eq!(config["log"], log, "{backend:?}");
        }
    }

    #[test]
    fn singbox_probe_config_shape() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let generator = SingboxProbeGenerator;
        let config = generator.generate(
            &refs,
            19_080,
            "https://www.gstatic.com/generate_204",
            5000,
            &[],
        );

        // No user-facing inbounds: without egress ports, none are emitted.
        assert!(
            config
                .get("inbounds")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        );

        // API bound to loopback only.
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"],
            "127.0.0.1:19080"
        );

        // Every node present as an outbound with the expected probe tag.
        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 4);
        for (i, ob) in outbounds.iter().enumerate() {
            assert_eq!(ob["tag"], probe_tag(i));
        }
        assert_eq!(outbounds[0]["type"], "vless");
        assert_eq!(outbounds[1]["type"], "vmess");
        assert_eq!(outbounds[2]["type"], "trojan");
        assert_eq!(outbounds[3]["type"], "shadowsocks");
    }

    #[test]
    fn xray_probe_config_shape() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let generator = XrayProbeGenerator;
        let test_url = "https://cp.cloudflare.com/generate_204";
        let config = generator.generate(&refs, 19_090, test_url, 4000, &[]);

        // API listener bound to loopback only; no egress inbounds requested.
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["listen"], "127.0.0.1");
        assert_eq!(inbounds[0]["port"], 19_090);

        // Observatory probes the configured URL.
        assert_eq!(config["observatory"]["probeUrl"], test_url);
        assert_eq!(
            config["burstObservatory"]["pingConfig"]["destination"],
            test_url
        );

        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 4);
        for (i, ob) in outbounds.iter().enumerate() {
            assert_eq!(ob["tag"], probe_tag(i));
        }
        assert_eq!(outbounds[0]["protocol"], "vless");
        assert_eq!(outbounds[1]["protocol"], "vmess");

        // Valid JSON.
        let s = serde_json::to_string(&config).unwrap();
        let _: Value = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn xray_probe_config_has_http_method() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let generator = XrayProbeGenerator;
        let test_url = "https://cp.cloudflare.com/generate_204";
        let config = generator.generate(&refs, 19_091, test_url, 5000, &[]);

        // pingConfig includes httpMethod
        assert_eq!(
            config["burstObservatory"]["pingConfig"]["httpMethod"],
            "HEAD"
        );
    }

    #[test]
    fn v2ray_probe_config_shape() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let generator = V2rayProbeGenerator;
        let test_url = "https://www.gstatic.com/generate_204";
        let config = generator.generate(&refs, 19_092, test_url, 5000, &[]);

        // API listener bound to loopback only; no egress inbounds requested.
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["listen"], "127.0.0.1");
        assert_eq!(inbounds[0]["port"], 19_092);

        // No user-facing inbounds (only API).
        assert_eq!(inbounds[0]["protocol"], "dokodemo-door");

        // burstObservatory is present.
        assert!(config.get("burstObservatory").is_some());
        assert_eq!(
            config["burstObservatory"]["pingConfig"]["destination"],
            test_url
        );

        // No httpMethod in pingConfig (v2ray doesn't support it).
        assert!(
            config["burstObservatory"]["pingConfig"]
                .get("httpMethod")
                .is_none()
        );

        // Every node present as an outbound with the expected probe tag.
        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 4);
        for (i, ob) in outbounds.iter().enumerate() {
            assert_eq!(ob["tag"], probe_tag(i));
        }
        assert_eq!(outbounds[0]["protocol"], "vless");
        assert_eq!(outbounds[1]["protocol"], "vmess");
        assert_eq!(outbounds[2]["protocol"], "trojan");
        assert_eq!(outbounds[3]["protocol"], "shadowsocks");

        // Valid JSON.
        let s = serde_json::to_string(&config).unwrap();
        let _: Value = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn v2ray_probe_skips_refused_nodes() {
        let reality = SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
            address: "reality.example.com".into(),
            port: 443,
            uuid: "uuid-r".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                reality: true,
                public_key: Some("pbk".into()),
                ..Default::default()
            }),
            remark: Some("REALITY".into()),
        }));
        let tls = mixed_nodes().swap_remove(0);
        let refs = vec![&reality, &tls];
        let config = V2rayProbeGenerator.generate(
            &refs,
            19_093,
            "https://www.gstatic.com/generate_204",
            5000,
            &[40_093, 40_094],
        );

        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 1);
        assert_eq!(outbounds[0]["tag"], probe_tag(1));
        assert_eq!(outbounds[0]["protocol"], "vless");

        // No egress inbound for the dropped REALITY outbound: its routing rule
        // would reference an outbound that does not exist.
        let egress_inbounds: Vec<&Value> = config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|ib| {
                ib["tag"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with(EGRESS_TAG_PREFIX)
            })
            .collect();
        assert_eq!(egress_inbounds.len(), 1);
        assert_eq!(egress_inbounds[0]["tag"], egress_tag(1));
    }

    #[test]
    fn pick_free_loopback_ports_yields_distinct_ports() {
        let ports = pick_free_loopback_ports(3).expect("port allocation succeeds");
        assert_eq!(ports.len(), 3);
        let unique: std::collections::HashSet<u16> = ports.iter().copied().collect();
        assert_eq!(unique.len(), 3, "ports must be distinct: {ports:?}");
        assert!(pick_free_loopback_ports(0).unwrap().is_empty());
    }

    #[test]
    fn egress_inbound_ports_parses_both_generator_shapes() {
        let config = json!({
            "inbounds": [
                { "tag": "api-in", "port": 19000 },
                { "tag": "egress-0", "listen": "127.0.0.1", "port": 19001 },
                { "tag": "egress-2", "listen": "127.0.0.1", "listen_port": 19003 },
                { "tag": "probe-0", "port": 1234 }
            ]
        });
        let ports = egress_inbound_ports(&config);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[&0], 19_001);
        assert_eq!(ports[&2], 19_003);
        assert!(egress_inbound_ports(&json!({ "log": {} })).is_empty());
    }

    #[test]
    fn xray_egress_inbounds_bind_loopback_and_route_to_own_outbound() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let config = XrayProbeGenerator.generate(
            &refs,
            19_094,
            "https://x/204",
            4000,
            &[40_001, 40_002, 40_003, 40_004],
        );

        let inbounds = config["inbounds"].as_array().unwrap();
        // api-in + one egress inbound per node.
        assert_eq!(inbounds.len(), 1 + 4);
        for i in 0..4 {
            let inbound = inbounds
                .iter()
                .find(|ib| ib["tag"] == egress_tag(i))
                .unwrap_or_else(|| panic!("missing egress inbound {i}"));
            assert_eq!(inbound["listen"], "127.0.0.1");
            assert_eq!(inbound["port"], 40_001 + i as u64);
            assert_eq!(inbound["protocol"], "http");
        }

        let rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1 + 4);
        for i in 0..4 {
            let rule = rules
                .iter()
                .find(|r| r["inboundTag"] == json!([egress_tag(i)]))
                .unwrap_or_else(|| panic!("missing egress route rule {i}"));
            assert_eq!(rule["outboundTag"], probe_tag(i));
        }
        // The API rule is present and first.
        assert_eq!(rules[0]["inboundTag"], json!(["api-in"]));
        assert_eq!(rules[0]["outboundTag"], "api");

        assert_eq!(egress_inbound_ports(&config)[&3], 40_004);
    }

    #[test]
    fn v2ray_egress_inbounds_bind_loopback_and_route_to_own_outbound() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let config = V2rayProbeGenerator.generate(
            &refs,
            19_095,
            "https://x/204",
            4000,
            &[40_011, 40_012, 40_013, 40_014],
        );

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1 + 4);
        assert_eq!(inbounds[0]["protocol"], "dokodemo-door");
        for i in 0..4 {
            let inbound = inbounds
                .iter()
                .find(|ib| ib["tag"] == egress_tag(i))
                .unwrap_or_else(|| panic!("missing egress inbound {i}"));
            assert_eq!(inbound["listen"], "127.0.0.1");
            assert_eq!(inbound["port"], 40_011 + i as u64);
            assert_eq!(inbound["protocol"], "http");
        }

        let rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1 + 4);
        assert_eq!(rules[0]["inboundTag"], json!(["api-in"]));
    }

    #[test]
    fn singbox_egress_inbounds_bind_loopback_and_route_to_own_outbound() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let config = SingboxProbeGenerator.generate(
            &refs,
            19_096,
            "https://x/204",
            4000,
            &[40_021, 40_022, 40_023, 40_024],
        );

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 4);
        for (i, inbound) in inbounds.iter().enumerate() {
            assert_eq!(inbound["tag"], egress_tag(i));
            assert_eq!(inbound["listen"], "127.0.0.1");
            assert_eq!(inbound["listen_port"], 40_021 + i as u64);
            assert_eq!(inbound["type"], "http");
        }

        let rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 4);
        for (i, rule) in rules.iter().enumerate() {
            assert_eq!(rule["inbound"], json!([egress_tag(i)]));
            assert_eq!(rule["outbound"], probe_tag(i));
        }

        assert_eq!(egress_inbound_ports(&config)[&2], 40_023);
    }

    #[test]
    fn egress_ports_shorter_than_nodes_are_tolerated() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let config = XrayProbeGenerator.generate(&refs, 19_097, "https://x/204", 4000, &[40_031]);
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1 + 1);
        assert_eq!(inbounds[1]["tag"], egress_tag(0));
    }
}
