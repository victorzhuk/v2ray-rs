use std::collections::HashMap;

use thiserror::Error;
use v2ray_rs_core::models::{
    GrpcSettings, H2Settings, ProxyNode, TlsSettings, TransportSettings, VmessConfig, WsSettings,
    XhttpSettings,
};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unsupported URI scheme: {0}")]
    UnsupportedScheme(String),
    #[error("invalid URI format: {0}")]
    InvalidFormat(String),
}

/// Backends expect a bare IPv6 literal in stored addresses; URI authorities and
/// JSON profiles may carry the RFC 3986 bracketed form.
pub(crate) fn normalize_host(host: &str) -> String {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'))
        && inner.parse::<std::net::Ipv6Addr>().is_ok()
    {
        return inner.to_string();
    }
    host.to_string()
}

fn strip_scheme<'a>(uri: &'a str, scheme: &str) -> Result<&'a str, ParseError> {
    let (actual, rest) = uri
        .split_once("://")
        .ok_or_else(|| ParseError::InvalidFormat(format!("missing {scheme}:// prefix")))?;
    if !actual.eq_ignore_ascii_case(scheme) {
        return Err(ParseError::InvalidFormat(format!(
            "missing {scheme}:// prefix"
        )));
    }
    Ok(rest)
}

fn percent_decode_component(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

pub fn parse_uri(uri: &str) -> Result<ProxyNode, ParseError> {
    let scheme = uri.split("://").next().unwrap_or("").to_lowercase();

    match scheme.as_str() {
        "vless" => parse_vless(uri),
        "vmess" => parse_vmess(uri),
        "ss" => parse_ss(uri),
        "trojan" => parse_trojan(uri),
        other => Err(ParseError::UnsupportedScheme(other.to_owned())),
    }
}

fn parse_url_transport(params: &HashMap<String, String>) -> Result<TransportSettings, ParseError> {
    let transport = match params.get("type").map(String::as_str) {
        None | Some("") | Some("tcp") => TransportSettings::Tcp,
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
        Some("h2") | Some("http") => {
            let host = params
                .get("host")
                .map(|h| vec![h.clone()])
                .unwrap_or_default();
            let path = params.get("path").cloned().unwrap_or_default();
            TransportSettings::H2(H2Settings { host, path })
        }
        Some("xhttp") | Some("splithttp") => TransportSettings::Xhttp(XhttpSettings {
            path: params.get("path").cloned().unwrap_or_default(),
            host: params.get("host").cloned(),
            mode: params
                .get("mode")
                .filter(|mode| !mode.is_empty())
                .cloned()
                .unwrap_or_else(|| "auto".to_string()),
        }),
        Some(other) => {
            return Err(ParseError::InvalidFormat(format!(
                "unsupported transport: {other}"
            )));
        }
    };
    Ok(transport)
}

fn parse_url_tls(params: &HashMap<String, String>) -> Result<Option<TlsSettings>, ParseError> {
    if params
        .get("allowInsecure")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    {
        return Err(ParseError::InvalidFormat(
            "allowInsecure disables TLS certificate verification".into(),
        ));
    }
    match params.get("security").map(|s| s.as_str()) {
        Some("tls") | Some("reality") => {
            let reality = params.get("security").map(|s| s.as_str()) == Some("reality");
            let server_name = params.get("sni").cloned();
            let alpn = params
                .get("alpn")
                .map(|a| a.split(',').map(|s| s.to_owned()).collect())
                .unwrap_or_default();
            let fingerprint = params.get("fp").cloned();
            let public_key = params.get("pbk").cloned();
            let short_id = params.get("sid").cloned();
            let spider_x = params.get("spx").cloned();
            Ok(Some(TlsSettings {
                server_name,
                alpn,
                verify: true,
                fingerprint,
                reality,
                public_key,
                short_id,
                spider_x,
            }))
        }
        _ => Ok(None),
    }
}

fn parse_vless(uri: &str) -> Result<ProxyNode, ParseError> {
    use v2ray_rs_core::models::VlessConfig;

    let url = url::Url::parse(uri).map_err(|e| ParseError::InvalidFormat(e.to_string()))?;

    let uuid = percent_decode_component(url.username());
    if uuid.is_empty() {
        return Err(ParseError::InvalidFormat("missing UUID".into()));
    }

    let address = url
        .host_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing host".into()))
        .map(normalize_host)?;
    let port = url
        .port()
        .ok_or_else(|| ParseError::InvalidFormat("missing port".into()))?;

    let remark = percent_decode_fragment(url.fragment());

    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let transport = parse_url_transport(&params)?;
    let tls = parse_url_tls(&params)?;

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

    const MAX_VMESS_PAYLOAD: usize = 64 * 1024; // 64 KB
    const MAX_VMESS_ENCODED: usize = MAX_VMESS_PAYLOAD.div_ceil(3) * 4;

    let encoded = strip_scheme(uri, "vmess")?;
    let encoded = encoded.trim();
    if encoded.len() > MAX_VMESS_ENCODED {
        return Err(ParseError::InvalidFormat(format!(
            "vmess payload too large: {} encoded bytes",
            encoded.len()
        )));
    }

    let decoded = STANDARD
        .decode(encoded)
        .map_err(|e| ParseError::InvalidFormat(format!("base64 decode failed: {e}")))?;

    if decoded.len() > MAX_VMESS_PAYLOAD {
        return Err(ParseError::InvalidFormat(format!(
            "vmess payload too large: {} bytes",
            decoded.len()
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| ParseError::InvalidFormat(format!("invalid JSON: {e}")))?;

    let address = normalize_host(
        json["add"]
            .as_str()
            .ok_or_else(|| ParseError::InvalidFormat("missing 'add' field".into()))?,
    );
    let raw_port = json["port"]
        .as_u64()
        .or_else(|| json["port"].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| ParseError::InvalidFormat("missing 'port' field".into()))?;
    let port = u16::try_from(raw_port)
        .map_err(|_| ParseError::InvalidFormat(format!("port out of range: {raw_port}")))?;
    let uuid = json["id"]
        .as_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing 'id' field".into()))?
        .to_owned();
    let remark = json["ps"].as_str().map(|s| s.to_owned());

    let transport = match json["net"].as_str().filter(|net| !net.is_empty()) {
        None | Some("tcp") => TransportSettings::Tcp,
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
        Some("h2") | Some("http") => {
            let host = json["host"]
                .as_str()
                .map(|h| vec![h.to_owned()])
                .unwrap_or_default();
            let path = json["path"].as_str().unwrap_or("").to_owned();
            TransportSettings::H2(H2Settings { host, path })
        }
        Some("xhttp") | Some("splithttp") => TransportSettings::Xhttp(XhttpSettings {
            path: json["path"].as_str().unwrap_or("").to_owned(),
            host: json["host"].as_str().map(|s| s.to_owned()),
            mode: json["mode"]
                .as_str()
                .filter(|mode| !mode.is_empty())
                .unwrap_or("auto")
                .to_string(),
        }),
        Some(other) => {
            return Err(ParseError::InvalidFormat(format!(
                "unsupported transport: {other}"
            )));
        }
    };

    let tls = if json["tls"].as_str() == Some("tls") {
        let insecure = json["allowInsecure"].as_bool().unwrap_or(false)
            || json["allowInsecure"]
                .as_str()
                .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        if insecure {
            return Err(ParseError::InvalidFormat(
                "allowInsecure disables TLS certificate verification".into(),
            ));
        }
        let server_name = json["sni"]
            .as_str()
            .or_else(|| json["host"].as_str())
            .map(|s| s.to_owned());
        Some(TlsSettings {
            server_name,
            ..Default::default()
        })
    } else {
        None
    };

    Ok(ProxyNode::Vmess(VmessConfig {
        address,
        port,
        uuid,
        alter_id: json["aid"].as_u64().unwrap_or(0) as u32,
        security: json["scy"].as_str().unwrap_or("auto").to_owned(),
        transport,
        tls,
        remark,
    }))
}

fn parse_ss(uri: &str) -> Result<ProxyNode, ParseError> {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use v2ray_rs_core::models::ShadowsocksConfig;

    let without_scheme = strip_scheme(uri, "ss")?;

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

    let parsed = url::Url::parse(&format!("ss://{host_port}"))
        .map_err(|e| ParseError::InvalidFormat(format!("invalid host:port: {e}")))?;
    let address = parsed
        .host_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing host".into()))
        .map(normalize_host)?;
    let port = parsed
        .port()
        .ok_or_else(|| ParseError::InvalidFormat("missing port".into()))?;

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

    let password = percent_decode_component(url.username());
    if password.is_empty() {
        return Err(ParseError::InvalidFormat("missing password".into()));
    }

    let address = url
        .host_str()
        .ok_or_else(|| ParseError::InvalidFormat("missing host".into()))
        .map(normalize_host)?;
    let port = url
        .port()
        .ok_or_else(|| ParseError::InvalidFormat("missing port".into()))?;

    let remark = percent_decode_fragment(url.fragment());

    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let transport = parse_url_transport(&params)?;
    let tls = match params.get("security").map(String::as_str) {
        Some("none") => None,
        Some("tls") | Some("reality") => parse_url_tls(&params)?,
        _ => Some(TlsSettings {
            server_name: Some(address.clone()),
            ..Default::default()
        }),
    };

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
                if let Err(e) = proxy_node.validate() {
                    log::warn!("skipping invalid node: {e}");
                    errors.push((uri.clone(), ParseError::InvalidFormat(e.to_string())));
                } else {
                    nodes.push(v2ray_rs_core::models::SubscriptionNode::new(proxy_node));
                }
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
        percent_encoding::percent_decode_str(f)
            .decode_utf8()
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| f.to_owned())
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
    fn test_parse_vless_rejects_allow_insecure() {
        for flag in ["1", "true", "TRUE"] {
            let uri = format!("vless://uuid@host:443?security=tls&allowInsecure={flag}#Insecure");
            let err = parse_uri(&uri).expect_err("allowInsecure must be rejected");
            assert!(
                err.to_string().contains("allowInsecure"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn test_parse_trojan_rejects_allow_insecure() {
        let err = parse_uri("trojan://pass@host:443?security=tls&allowInsecure=1")
            .expect_err("allowInsecure must be rejected");
        assert!(err.to_string().contains("allowInsecure"), "{err}");
    }

    #[test]
    fn test_allow_insecure_uri_does_not_block_valid_uris() {
        let uris = vec![
            "vless://uuid@host:443?security=tls&allowInsecure=1".to_string(),
            "vless://uuid@host:443".to_string(),
        ];
        let result = parse_subscription_uris(&uris);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].1.to_string().contains("allowInsecure"));
    }

    #[test]
    fn test_parse_uri_dispatches_correctly() {
        let vless_uri = "vless://uuid@host:443";
        let vmess_json = r#"{"add":"host","port":"443","id":"uuid"}"#;
        let vmess_uri = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_json)
        );
        let ss_userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("method:pass");
        let ss_uri = format!("ss://{}@host:8388", ss_userinfo);
        let trojan_uri = "trojan://pass@host:443";

        assert!(matches!(parse_uri(vless_uri), Ok(ProxyNode::Vless(_))));
        assert!(matches!(parse_uri(&vmess_uri), Ok(ProxyNode::Vmess(_))));
        assert!(matches!(parse_uri(&ss_uri), Ok(ProxyNode::Shadowsocks(_))));
        assert!(matches!(parse_uri(trojan_uri), Ok(ProxyNode::Trojan(_))));
    }

    #[test]
    fn test_parse_vless_ipv6_stores_bare_literal() {
        let node = parse_uri("vless://uuid@[2001:db8::1]:443?security=tls#v6").unwrap();
        assert_eq!(node.address(), "2001:db8::1");
        assert!(node.validate().is_ok());
    }

    #[test]
    fn test_parse_vless_xhttp_transport() {
        let uri = "vless://uuid@example.com:443?type=xhttp&path=%2Fsplit&host=cdn.example.com&mode=packet-up";
        let ProxyNode::Vless(cfg) = parse_uri(uri).unwrap() else {
            panic!("expected VLESS")
        };
        assert_eq!(
            cfg.transport,
            TransportSettings::Xhttp(XhttpSettings {
                path: "/split".into(),
                host: Some("cdn.example.com".into()),
                mode: "packet-up".into(),
            })
        );
    }

    #[test]
    fn test_parse_unknown_transport_is_rejected() {
        for kind in ["httpupgrade", "kcp"] {
            let uri = format!("vless://uuid@example.com:443?type={kind}");
            let err = parse_uri(&uri).unwrap_err();
            assert!(
                err.to_string().contains("unsupported transport"),
                "unexpected error for {kind}: {err}"
            );
        }
    }

    #[test]
    fn test_parse_uppercase_vmess_and_ss_schemes() {
        let vmess_json = r#"{"add":"host","port":"443","id":"uuid"}"#;
        let vmess_uri = format!(
            "VMESS://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_json)
        );
        assert!(matches!(parse_uri(&vmess_uri), Ok(ProxyNode::Vmess(_))));

        let ss_userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("method:pass");
        let ss_uri = format!("SS://{}@host:8388", ss_userinfo);
        assert!(matches!(parse_uri(&ss_uri), Ok(ProxyNode::Shadowsocks(_))));
    }

    #[test]
    fn test_parse_trojan_defaults_tls_and_decodes_password() {
        let ProxyNode::Trojan(cfg) =
            parse_uri("trojan://secret%40pass@example.com:8443#node").unwrap()
        else {
            panic!("expected Trojan")
        };
        assert_eq!(cfg.password, "secret@pass");
        let tls = cfg.tls.expect("TLS must default on for Trojan");
        assert!(tls.verify);
        assert_eq!(tls.server_name.as_deref(), Some("example.com"));
    }

    #[test]
    fn test_parse_vmess_rejects_oversized_payload_and_port() {
        let oversized = format!("vmess://{}", "A".repeat((70 * 1024usize).div_ceil(3) * 4));
        assert!(parse_uri(&oversized).is_err());

        let bad_port = r#"{"add":"host","port":"70000","id":"uuid"}"#;
        let uri = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(bad_port)
        );
        let err = parse_uri(&uri).unwrap_err();
        assert!(err.to_string().contains("port out of range"), "{err}");
    }

    #[test]
    fn test_parse_subscription_uris_partial_success() {
        let vmess_json = r#"{"add":"example.com","port":"443","id":"uuid"}"#;
        let vmess_uri = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_json)
        );

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

        let error_schemes: Vec<_> = result
            .errors
            .iter()
            .map(|(uri, _)| uri.split("://").next().unwrap())
            .collect();
        assert!(error_schemes.contains(&"http"));
        assert!(error_schemes.contains(&"ss"));
    }
}
