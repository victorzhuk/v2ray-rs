use std::collections::HashMap;

use thiserror::Error;
use v2ray_rs_core::models::{ProxyNode, TransportSettings, TlsSettings, WsSettings, GrpcSettings, H2Settings};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unsupported URI scheme: {0}")]
    UnsupportedScheme(String),
    #[error("invalid URI format: {0}")]
    InvalidFormat(String),
}

pub fn parse_uri(uri: &str) -> Result<ProxyNode, ParseError> {
    let scheme = uri
        .split("://")
        .next()
        .unwrap_or("")
        .to_lowercase();

    match scheme.as_str() {
        "vless" => parse_vless(uri),
        "vmess" => parse_vmess(uri),
        "ss" => parse_ss(uri),
        "trojan" => parse_trojan(uri),
        other => Err(ParseError::UnsupportedScheme(other.to_owned())),
    }
}

fn parse_url_transport(params: &HashMap<String, String>) -> TransportSettings {
    match params.get("type").map(|s| s.as_str()) {
        Some("ws") => {
            let path = params.get("path").cloned().unwrap_or_default();
            let host = params.get("host").cloned();
            TransportSettings::Ws(WsSettings {
                path,
                host,
                headers: Default::default(),
            })
        }
        Some("grpc") => {
            let service_name = params.get("serviceName").cloned().unwrap_or_default();
            TransportSettings::Grpc(GrpcSettings {
                service_name,
                multi_mode: false,
            })
        }
        Some("h2") => {
            let host = params
                .get("host")
                .map(|h| vec![h.clone()])
                .unwrap_or_default();
            let path = params.get("path").cloned().unwrap_or_default();
            TransportSettings::H2(H2Settings { host, path })
        }
        _ => TransportSettings::Tcp,
    }
}

fn parse_url_tls(params: &HashMap<String, String>) -> Option<TlsSettings> {
    match params.get("security").map(|s| s.as_str()) {
        Some("tls") | Some("reality") => {
            let server_name = params.get("sni").cloned();
            let alpn = params
                .get("alpn")
                .map(|a| a.split(',').map(|s| s.to_owned()).collect())
                .unwrap_or_default();
            let fingerprint = params.get("fp").cloned();
            Some(TlsSettings {
                server_name,
                alpn,
                verify: true,
                fingerprint,
            })
        }
        _ => None,
    }
}

fn parse_vless(uri: &str) -> Result<ProxyNode, ParseError> {
    use v2ray_rs_core::models::VlessConfig;

    let url = url::Url::parse(uri).map_err(|e| ParseError::InvalidFormat(e.to_string()))?;

    let uuid = url.username().to_owned();
    if uuid.is_empty() {
        return Err(ParseError::InvalidFormat("missing UUID".into()));
    }

    let address = url
        .host_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing host".into()))?
        .to_owned();
    let port = url
        .port()
        .ok_or_else(|| ParseError::InvalidFormat("missing port".into()))?;

    let remark = percent_decode_fragment(url.fragment());

    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let transport = parse_url_transport(&params);
    let tls = parse_url_tls(&params);

    let flow = params.get("flow").cloned();
    let encryption = params.get("encryption").cloned();

    Ok(ProxyNode::Vless(VlessConfig {
        address,
        port,
        uuid,
        encryption,
        flow,
        transport,
        tls,
        remark,
    }))
}

fn parse_vmess(uri: &str) -> Result<ProxyNode, ParseError> {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use v2ray_rs_core::models::{VmessConfig, TransportSettings, TlsSettings, WsSettings, GrpcSettings, H2Settings};

    let encoded = uri
        .strip_prefix("vmess://")
        .ok_or_else(|| ParseError::InvalidFormat("missing vmess:// prefix".into()))?;

    let decoded = STANDARD
        .decode(encoded.trim())
        .map_err(|e| ParseError::InvalidFormat(format!("base64 decode failed: {e}")))?;
    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| ParseError::InvalidFormat(format!("invalid JSON: {e}")))?;

    let address = json["add"]
        .as_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing 'add' field".into()))?
        .to_owned();
    let port = json["port"]
        .as_u64()
        .or_else(|| json["port"].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| ParseError::InvalidFormat("missing 'port' field".into()))?
        as u16;
    let uuid = json["id"]
        .as_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing 'id' field".into()))?
        .to_owned();
    let remark = json["ps"].as_str().map(|s| s.to_owned());

    let transport = match json["net"].as_str() {
        Some("ws") => {
            let path = json["path"].as_str().unwrap_or("").to_owned();
            let host = json["host"].as_str().map(|s| s.to_owned());
            TransportSettings::Ws(WsSettings {
                path,
                host,
                headers: Default::default(),
            })
        }
        Some("grpc") => {
            let service_name = json["path"].as_str().unwrap_or("").to_owned();
            TransportSettings::Grpc(GrpcSettings {
                service_name,
                multi_mode: false,
            })
        }
        Some("h2") => {
            let host = json["host"]
                .as_str()
                .map(|h| vec![h.to_owned()])
                .unwrap_or_default();
            let path = json["path"].as_str().unwrap_or("").to_owned();
            TransportSettings::H2(H2Settings { host, path })
        }
        _ => TransportSettings::Tcp,
    };

    let tls = if json["tls"].as_str() == Some("tls") {
        let server_name = json["sni"]
            .as_str()
            .or_else(|| json["host"].as_str())
            .map(|s| s.to_owned());
        Some(TlsSettings {
            server_name,
            alpn: vec![],
            verify: true,
            fingerprint: None,
        })
    } else {
        None
    };

    Ok(ProxyNode::Vmess(VmessConfig {
        address,
        port,
        uuid,
        alter_id: json["aid"].as_u64().unwrap_or(0) as u32,
        security: json["scy"]
            .as_str()
            .unwrap_or("auto")
            .to_owned(),
        transport,
        tls,
        remark,
    }))
}

fn parse_ss(uri: &str) -> Result<ProxyNode, ParseError> {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use v2ray_rs_core::models::ShadowsocksConfig;

    let without_scheme = uri
        .strip_prefix("ss://")
        .ok_or_else(|| ParseError::InvalidFormat("missing ss:// prefix".into()))?;

    let (userinfo_part, host_part) = without_scheme
        .split_once('@')
        .ok_or_else(|| ParseError::InvalidFormat("missing '@' separator".into()))?;

    let decoded = URL_SAFE_NO_PAD
        .decode(userinfo_part.trim())
        .or_else(|_| STANDARD.decode(userinfo_part.trim()))
        .map_err(|e| ParseError::InvalidFormat(format!("base64 decode failed: {e}")))?;
    let userinfo = String::from_utf8(decoded)
        .map_err(|e| ParseError::InvalidFormat(format!("invalid UTF-8: {e}")))?;

    let (method, password) = userinfo
        .split_once(':')
        .ok_or_else(|| ParseError::InvalidFormat("missing method:password".into()))?;

    let (host_port, fragment) = host_part.split_once('#').unzip();
    let host_port = host_port.unwrap_or(host_part);

    let (address, port_str) = host_port
        .rsplit_once(':')
        .ok_or_else(|| ParseError::InvalidFormat("missing host:port".into()))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| ParseError::InvalidFormat("invalid port".into()))?;

    let remark = percent_decode_fragment(fragment);

    Ok(ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: address.to_owned(),
        port,
        method: method.to_owned(),
        password: password.to_owned(),
        remark,
    }))
}

fn parse_trojan(uri: &str) -> Result<ProxyNode, ParseError> {
    use v2ray_rs_core::models::TrojanConfig;

    let url = url::Url::parse(uri).map_err(|e| ParseError::InvalidFormat(e.to_string()))?;

    let password = url.username().to_owned();
    if password.is_empty() {
        return Err(ParseError::InvalidFormat("missing password".into()));
    }

    let address = url
        .host_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing host".into()))?
        .to_owned();
    let port = url
        .port()
        .ok_or_else(|| ParseError::InvalidFormat("missing port".into()))?;

    let remark = percent_decode_fragment(url.fragment());

    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let transport = parse_url_transport(&params);
    let tls = parse_url_tls(&params).or_else(|| {
        if port == 443 {
            Some(TlsSettings {
                server_name: Some(address.clone()),
                alpn: vec![],
                verify: true,
                fingerprint: None,
            })
        } else {
            None
        }
    });

    Ok(ProxyNode::Trojan(TrojanConfig {
        address,
        port,
        password,
        transport,
        tls,
        remark,
    }))
}

pub struct ImportResult {
    pub nodes: Vec<v2ray_rs_core::models::SubscriptionNode>,
    pub errors: Vec<(String, ParseError)>,
}

pub fn parse_subscription_uris(uris: &[String]) -> ImportResult {
    let mut nodes = Vec::new();
    let mut errors = Vec::new();

    for uri in uris {
        match parse_uri(uri) {
            Ok(proxy_node) => {
                nodes.push(v2ray_rs_core::models::SubscriptionNode {
                    node: proxy_node,
                    enabled: true,
                    last_latency_ms: None,
                });
            }
            Err(e) => {
                errors.push((uri.clone(), e));
            }
        }
    }

    ImportResult { nodes, errors }
}

fn percent_decode_fragment(fragment: Option<&str>) -> Option<String> {
    fragment.map(|f| {
        url::form_urlencoded::parse(f.as_bytes())
            .next()
            .map(|(k, _)| k.into_owned())
            .unwrap_or_else(|| f.to_owned())
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use v2ray_rs_core::models::TransportSettings;

    #[test]
    fn test_parse_vless_basic() {
        let uri = "vless://550e8400-e29b-41d4-a716-446655440000@example.com:443#Test";
        let result = parse_uri(uri).unwrap();

        match result {
            ProxyNode::Vless(cfg) => {
                assert_eq!(cfg.uuid, "550e8400-e29b-41d4-a716-446655440000");
                assert_eq!(cfg.address, "example.com");
                assert_eq!(cfg.port, 443);
                assert_eq!(cfg.remark, Some("Test".to_string()));
                assert_eq!(cfg.transport, TransportSettings::Tcp);
                assert_eq!(cfg.tls, None);
            }
            _ => panic!("expected VLESS config"),
        }
    }

    #[test]
    fn test_parse_vless_with_ws_tls() {
        let uri = "vless://uuid@example.com:443?type=ws&host=example.com&path=/ws&security=tls&sni=example.com&fp=chrome&alpn=h2,http/1.1&flow=xtls-rprx-vision&encryption=none#Test";
        let result = parse_uri(uri).unwrap();

        match result {
            ProxyNode::Vless(cfg) => {
                assert_eq!(cfg.uuid, "uuid");
                assert_eq!(cfg.encryption, Some("none".to_string()));
                assert_eq!(cfg.flow, Some("xtls-rprx-vision".to_string()));

                match cfg.transport {
                    TransportSettings::Ws(ws) => {
                        assert_eq!(ws.path, "/ws");
                        assert_eq!(ws.host, Some("example.com".to_string()));
                    }
                    _ => panic!("expected WS transport"),
                }

                let tls = cfg.tls.unwrap();
                assert_eq!(tls.server_name, Some("example.com".to_string()));
                assert_eq!(tls.alpn, vec!["h2", "http/1.1"]);
                assert_eq!(tls.fingerprint, Some("chrome".to_string()));
                assert!(tls.verify);
            }
            _ => panic!("expected VLESS config"),
        }
    }

    #[test]
    fn test_parse_vless_with_grpc() {
        let uri = "vless://uuid@example.com:443?type=grpc&serviceName=MyService&security=tls";
        let result = parse_uri(uri).unwrap();

        match result {
            ProxyNode::Vless(cfg) => {
                match cfg.transport {
                    TransportSettings::Grpc(grpc) => {
                        assert_eq!(grpc.service_name, "MyService");
                    }
                    _ => panic!("expected GRPC transport"),
                }
                assert!(cfg.tls.is_some());
            }
            _ => panic!("expected VLESS config"),
        }
    }

    #[test]
    fn test_parse_vmess_basic() {
        let vmess_json = r#"{"add":"example.com","port":"443","id":"uuid","aid":0,"ps":"Test"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(vmess_json);
        let uri = format!("vmess://{}", encoded);

        let result = parse_uri(&uri).unwrap();

        match result {
            ProxyNode::Vmess(cfg) => {
                assert_eq!(cfg.address, "example.com");
                assert_eq!(cfg.port, 443);
                assert_eq!(cfg.uuid, "uuid");
                assert_eq!(cfg.alter_id, 0);
                assert_eq!(cfg.remark, Some("Test".to_string()));
            }
            _ => panic!("expected VMess config"),
        }
    }

    #[test]
    fn test_parse_vmess_with_ws_tls() {
        let vmess_json = r#"{"add":"example.com","port":"443","id":"uuid","net":"ws","host":"example.com","path":"/ws","tls":"tls","sni":"example.com","ps":"Test"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(vmess_json);
        let uri = format!("vmess://{}", encoded);

        let result = parse_uri(&uri).unwrap();

        match result {
            ProxyNode::Vmess(cfg) => {
                match cfg.transport {
                    TransportSettings::Ws(ws) => {
                        assert_eq!(ws.path, "/ws");
                        assert_eq!(ws.host, Some("example.com".to_string()));
                    }
                    _ => panic!("expected WS transport"),
                }

                let tls = cfg.tls.unwrap();
                assert_eq!(tls.server_name, Some("example.com".to_string()));
            }
            _ => panic!("expected VMess config"),
        }
    }

    #[test]
    fn test_parse_ss_sip002() {
        let userinfo = "aes-256-gcm:password";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(userinfo);
        let uri = format!("ss://{}@example.com:8388#Test", encoded);

        let result = parse_uri(&uri).unwrap();

        match result {
            ProxyNode::Shadowsocks(cfg) => {
                assert_eq!(cfg.address, "example.com");
                assert_eq!(cfg.port, 8388);
                assert_eq!(cfg.method, "aes-256-gcm");
                assert_eq!(cfg.password, "password");
                assert_eq!(cfg.remark, Some("Test".to_string()));
            }
            _ => panic!("expected Shadowsocks config"),
        }
    }

    #[test]
    fn test_parse_trojan_basic() {
        let uri = "trojan://password@example.com:443#Test";
        let result = parse_uri(uri).unwrap();

        match result {
            ProxyNode::Trojan(cfg) => {
                assert_eq!(cfg.password, "password");
                assert_eq!(cfg.address, "example.com");
                assert_eq!(cfg.port, 443);
                assert_eq!(cfg.remark, Some("Test".to_string()));
                assert!(cfg.tls.is_some());
            }
            _ => panic!("expected Trojan config"),
        }
    }

    #[test]
    fn test_parse_trojan_with_tls() {
        let uri = "trojan://password@example.com:443?security=tls&sni=example.com&alpn=h2#Test";
        let result = parse_uri(uri).unwrap();

        match result {
            ProxyNode::Trojan(cfg) => {
                let tls = cfg.tls.unwrap();
                assert_eq!(tls.server_name, Some("example.com".to_string()));
                assert_eq!(tls.alpn, vec!["h2"]);
            }
            _ => panic!("expected Trojan config"),
        }
    }

    #[test]
    fn test_parse_unknown_scheme() {
        let uri = "http://foo";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::UnsupportedScheme(scheme)) => {
                assert_eq!(scheme, "http");
            }
            _ => panic!("expected UnsupportedScheme error"),
        }
    }

    #[test]
    fn test_parse_vless_missing_uuid() {
        let uri = "vless://@host:443";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("UUID"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_vless_missing_port() {
        let uri = "vless://uuid@host";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("port"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_vmess_invalid_base64() {
        let uri = "vmess://not-base64!@#$%";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("base64"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_vmess_missing_fields() {
        let vmess_json = r#"{"port":"443","id":"uuid"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(vmess_json);
        let uri = format!("vmess://{}", encoded);

        let result = parse_uri(&uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("add"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_ss_missing_at() {
        let uri = "ss://base64stuff";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("@"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_trojan_missing_password() {
        let uri = "trojan://@host:443";
        let result = parse_uri(uri);

        match result {
            Err(ParseError::InvalidFormat(msg)) => {
                assert!(msg.contains("password"));
            }
            _ => panic!("expected InvalidFormat error"),
        }
    }

    #[test]
    fn test_parse_uri_dispatches_correctly() {
        let vless_uri = "vless://uuid@host:443";
        let vmess_json = r#"{"add":"host","port":"443","id":"uuid"}"#;
        let vmess_uri = format!("vmess://{}", base64::engine::general_purpose::STANDARD.encode(vmess_json));
        let ss_userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("method:pass");
        let ss_uri = format!("ss://{}@host:8388", ss_userinfo);
        let trojan_uri = "trojan://pass@host:443";

        assert!(matches!(parse_uri(vless_uri), Ok(ProxyNode::Vless(_))));
        assert!(matches!(parse_uri(&vmess_uri), Ok(ProxyNode::Vmess(_))));
        assert!(matches!(parse_uri(&ss_uri), Ok(ProxyNode::Shadowsocks(_))));
        assert!(matches!(parse_uri(trojan_uri), Ok(ProxyNode::Trojan(_))));
    }

    #[test]
    fn test_parse_subscription_uris_partial_success() {
        let vmess_json = r#"{"add":"example.com","port":"443","id":"uuid"}"#;
        let vmess_uri = format!("vmess://{}", base64::engine::general_purpose::STANDARD.encode(vmess_json));

        let uris = vec![
            "vless://uuid@host:443".to_string(),
            vmess_uri,
            "http://invalid".to_string(),
            "ss://malformed".to_string(),
            "trojan://pass@host:443".to_string(),
        ];

        let result = parse_subscription_uris(&uris);

        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.errors.len(), 2);

        assert!(result.nodes.iter().all(|n| n.enabled));

        let error_schemes: Vec<_> = result.errors.iter().map(|(uri, _)| {
            uri.split("://").next().unwrap()
        }).collect();
        assert!(error_schemes.contains(&"http"));
        assert!(error_schemes.contains(&"ss"));
    }
}
