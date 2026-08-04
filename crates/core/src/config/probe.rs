//! Ephemeral "Real Delay" probe config generation.
//!
//! These generators emit a minimal backend config used by a short-lived,
//! isolated backend instance to measure end-to-end delay through each candidate
//! node. The generated config has **no** user-facing inbound (only a loopback
//! API listener), no routing rules, and tags every probed outbound as
//! `probe-<idx>` for stable mapping back to the node index.

use serde_json::{Value, json};

use crate::models::{BackendType, SubscriptionNode};

/// Tag prefix used for every probed outbound so results map back to the node
/// index deterministically.
pub const PROBE_TAG_PREFIX: &str = "probe-";

/// Returns the stable outbound tag for the node at `idx`.
#[must_use]
pub fn probe_tag(idx: usize) -> String {
    format!("{PROBE_TAG_PREFIX}{idx}")
}

pub trait ProbeConfigGenerator: Send + Sync {
    /// Generates a probe config exposing an API on `127.0.0.1:<api_port>` with
    /// every node in `nodes` present as an outbound tagged `probe-<idx>`.
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        test_url: &str,
        timeout_ms: u32,
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
    ) -> Value {
        // A node whose transport sing-box can't emit (xhttp) just can't be
        // probed on this backend; drop it rather than fail the whole batch.
        // Safe because results are matched back by parsing the index out of
        // the outbound tag, not by position.
        let outbounds: Vec<Value> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                crate::config::singbox::build_singbox_outbound(&node.node, &probe_tag(i)).ok()
            })
            .collect();

        json!({
            "log": { "level": "warn" },
            "outbounds": outbounds,
            "experimental": {
                "clash_api": {
                    "external_controller": format!("127.0.0.1:{api_port}"),
                }
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
    ) -> Value {
        let outbounds: Vec<Value> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| crate::config::xray::build_xray_outbound(&node.node, &probe_tag(i)))
            .collect();

        json!({
            "log": { "loglevel": "warning" },
            "outbounds": outbounds,
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
            "inbounds": [{
                "tag": "api-in",
                "listen": "127.0.0.1",
                "port": api_port,
                "protocol": "dokodemo-door",
                "settings": { "address": "127.0.0.1" },
            }],
            "routing": {
                "rules": [{
                    "type": "field",
                    "inboundTag": ["api-in"],
                    "outboundTag": "api",
                }]
            }
        })
    }
}

pub struct V2rayProbeGenerator;

impl ProbeConfigGenerator for V2rayProbeGenerator {
    fn generate(
        &self,
        nodes: &[&SubscriptionNode],
        api_port: u16,
        test_url: &str,
        timeout_ms: u32,
    ) -> Value {
        let outbounds: Vec<Value> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| crate::config::v2ray::build_family_outbound(&node.node, &probe_tag(i)))
            .collect();

        json!({
            "log": { "loglevel": "warning" },
            "outbounds": outbounds,
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
            "inbounds": [{
                "tag": "api-in",
                "listen": "127.0.0.1",
                "port": api_port,
                "protocol": "dokodemo-door",
                "settings": { "address": "127.0.0.1" },
            }],
            "routing": {
                "rules": [{
                    "type": "field",
                    "inboundTag": ["api-in"],
                    "outboundTag": "api",
                }]
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
    fn singbox_probe_config_shape() {
        let nodes = mixed_nodes();
        let refs: Vec<&SubscriptionNode> = nodes.iter().collect();
        let generator = SingboxProbeGenerator;
        let config =
            generator.generate(&refs, 19_080, "https://www.gstatic.com/generate_204", 5000);

        // No user-facing inbounds.
        assert!(config.get("inbounds").is_none());

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
        let config = generator.generate(&refs, 19_090, test_url, 4000);

        // API listener bound to loopback only.
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
        let config = generator.generate(&refs, 19_091, test_url, 5000);

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
        let config = generator.generate(&refs, 19_092, test_url, 5000);

        // API listener bound to loopback only.
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
}
