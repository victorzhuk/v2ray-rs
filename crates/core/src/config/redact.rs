use serde_json::Value;

const REDACTED: &str = "********";

const SENSITIVE_KEYS: &[&str] = &["id", "uuid", "password", "short_id", "shortId"];

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
