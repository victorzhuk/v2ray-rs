use serde_json::Value;

const REDACTED: &str = "********";

const SENSITIVE_KEYS: &[&str] = &["id", "uuid", "password", "short_id", "shortId"];

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
];

pub fn redact_json(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let redacted = redact_value(value);
    serde_json::to_string_pretty(&redacted).ok()
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, child)| {
                    if SENSITIVE_KEYS.contains(&key.as_str()) {
                        (key, Value::String(REDACTED.to_owned()))
                    } else if key == "headers" {
                        (key, redact_headers(child))
                    } else if matches!(key.as_str(), "server" | "address") {
                        (key, redact_dns_address(child))
                    } else {
                        (key, redact_value(child))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        other => other,
    }
}

fn redact_headers(value: Value) -> Value {
    let Value::Object(map) = value else {
        return value;
    };
    Value::Object(
        map.into_iter()
            .map(|(key, child)| {
                if SENSITIVE_HEADERS.contains(&key.to_ascii_lowercase().as_str()) {
                    (key, Value::String(REDACTED.to_owned()))
                } else {
                    (key, child)
                }
            })
            .collect(),
    )
}

/// Masks non-default path and query components of DoH/H3 server URLs while
/// keeping the host visible.
fn redact_dns_address(value: Value) -> Value {
    let Value::String(address) = value else {
        return value;
    };
    let Ok(url) = url::Url::parse(&address) else {
        return Value::String(address);
    };
    if !matches!(url.scheme(), "https" | "h3") {
        return Value::String(address);
    }
    let default_path = matches!(url.path(), "" | "/" | "/dns-query");
    if default_path && url.query().is_none() {
        return Value::String(address);
    }
    let mut masked = url.clone();
    masked.set_path(&format!("/{REDACTED}"));
    masked.set_query(None);
    Value::String(masked.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_returns_none() {
        assert!(redact_json("not json").is_none());
    }

    #[test]
    fn v2ray_reality_shape_is_redacted() {
        let raw = r#"{
            "outbounds": [{
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "users": [{
                            "id": "550e8400-e29b-41d4-a716-446655440000",
                            "encryption": "none",
                            "flow": "xtls-rprx-vision"
                        }]
                    }]
                },
                "streamSettings": {
                    "security": "reality",
                    "realitySettings": {
                        "publicKey": "keep-me",
                        "shortId": "secret-sid",
                        "spiderX": "keep-spider"
                    }
                }
            }]
        }"#;
        let out = redact_json(raw).unwrap();
        assert!(out.contains(REDACTED));
        assert!(!out.contains("550e8400"));
        assert!(out.contains("keep-me"));
        assert!(!out.contains("secret-sid"));
        assert!(out.contains("keep-spider"));
    }

    #[test]
    fn xray_reality_shape_is_redacted() {
        let raw = r#"{
            "outbounds": [{
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "users": [{
                            "id": "xray-id-123",
                            "encryption": "none"
                        }]
                    }]
                },
                "streamSettings": {
                    "security": "reality",
                    "realitySettings": {
                        "publicKey": "xray-pbk",
                        "shortId": "xray-sid"
                    }
                }
            }]
        }"#;
        let out = redact_json(raw).unwrap();
        assert!(out.contains(REDACTED));
        assert!(!out.contains("xray-id-123"));
        assert!(out.contains("xray-pbk"));
        assert!(!out.contains("xray-sid"));
    }

    #[test]
    fn singbox_reality_shape_is_redacted() {
        let raw = r#"{
            "outbounds": [{
                "type": "vless",
                "uuid": "sing-uuid",
                "password": "sing-pass",
                "tls": {
                    "reality": {
                        "enabled": true,
                        "public_key": "sing-pbk",
                        "short_id": "sing-sid"
                    }
                }
            }]
        }"#;
        let out = redact_json(raw).unwrap();
        assert!(out.contains(REDACTED));
        assert!(!out.contains("sing-uuid"));
        assert!(!out.contains("sing-pass"));
        assert!(out.contains("sing-pbk"));
        assert!(!out.contains("sing-sid"));
    }

    #[test]
    fn sensitive_headers_are_redacted() {
        let raw = r#"{
            "outbounds": [{
                "transport": {
                    "headers": {
                        "Authorization": "Bearer secret-token",
                        "proxy-authorization": "Basic c2VjcmV0",
                        "Cookie": "session=abc",
                        "set-cookie": "sid=def",
                        "X-Api-Key": "api-key-value",
                        "Host": "edge.example.com"
                    }
                }
            }]
        }"#;
        let out = redact_json(raw).unwrap();
        for secret in [
            "Bearer secret-token",
            "Basic c2VjcmV0",
            "session=abc",
            "sid=def",
            "api-key-value",
        ] {
            assert!(!out.contains(secret), "secret kept: {secret}");
        }
        assert!(out.contains(REDACTED));
        assert!(out.contains("edge.example.com"));
    }

    #[test]
    fn tokenized_dns_urls_are_redacted_but_host_and_public_key_stay() {
        let raw = r#"{
            "dns": {
                "servers": [
                    {"tag": "doh", "address": "https://dns.example.com/secret-token?key=abc"},
                    {"tag": "h3", "address": "h3://dns.example.net/private/resolve?token=xyz"},
                    {"tag": "plain", "address": "https://dns.example.org/dns-query"},
                    {"tag": "udp", "address": "1.1.1.1"}
                ]
            },
            "outbounds": [{
                "tls": {"reality": {"public_key": "keep-public-key", "password": "drop-me"}}
            }]
        }"#;
        let out = redact_json(raw).unwrap();
        assert!(!out.contains("secret-token"));
        assert!(!out.contains("key=abc"));
        assert!(!out.contains("private/resolve"));
        assert!(!out.contains("token=xyz"));
        assert!(!out.contains("drop-me"));
        assert!(out.contains("dns.example.com"));
        assert!(out.contains("dns.example.net"));
        assert!(out.contains("https://dns.example.org/dns-query"));
        assert!(out.contains("1.1.1.1"));
        assert!(out.contains("keep-public-key"));
    }

    #[test]
    fn nested_objects_and_arrays_are_redacted_recursively() {
        let raw = r#"{
            "users": [
                {"id": "outer-id", "nested": {"password": "deep-pass", "uuid": "deep-uuid"}},
                [{"shortId": "array-shortid", "short_id": "array_short_id"}]
            ],
            "public_key": "top-pbk",
            "publicKey": "top-Pbk"
        }"#;
        let out = redact_json(raw).unwrap();
        assert!(!out.contains("outer-id"));
        assert!(!out.contains("deep-pass"));
        assert!(!out.contains("deep-uuid"));
        assert!(!out.contains("array-shortid"));
        assert!(!out.contains("array_short_id"));
        assert!(out.contains("top-pbk"));
        assert!(out.contains("top-Pbk"));
    }
}
