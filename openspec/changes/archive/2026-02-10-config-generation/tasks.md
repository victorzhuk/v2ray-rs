## 1. Generator Trait & Structure

- [x] 1.1 Define ConfigGenerator trait with generate() method returning serde_json::Value
- [x] 1.2 Create module structure: config/mod.rs, config/v2ray.rs, config/xray.rs, config/singbox.rs
- [x] 1.3 Add serde_json dependency

## 2. V2ray/Xray Config Generation

- [x] 2.1 Implement inbound section builder (SOCKS5 + HTTP with configurable ports)
- [x] 2.2 Implement outbound section builder for VLESS nodes
- [x] 2.3 Implement outbound section builder for VMess nodes
- [x] 2.4 Implement outbound section builder for Shadowsocks nodes
- [x] 2.5 Implement outbound section builder for Trojan nodes
- [x] 2.6 Implement "freedom" direct outbound and "blackhole" block outbound
- [x] 2.7 Implement routing section builder with GeoIP and GeoSite rules
- [x] 2.8 Implement routing section builder with domain and IP CIDR rules
- [x] 2.9 Assemble full v2ray config from sections
- [x] 2.10 Implement XrayGenerator (extends v2ray with xray-specific fields like XTLS flow)
- [x] 2.11 Write tests validating generated JSON structure

## 3. Sing-box Config Generation

- [x] 3.1 Implement sing-box inbound section (mixed type with SOCKS5+HTTP)
- [x] 3.2 Implement sing-box outbound section for each protocol
- [x] 3.3 Implement sing-box route section with geoip/geosite rules
- [x] 3.4 Assemble full sing-box config
- [x] 3.5 Write tests validating sing-box JSON structure

## 4. Config File Management

- [x] 4.1 Implement atomic config file writing (temp file + rename)
- [x] 4.2 Implement config output path resolution (default + user override)
- [x] 4.3 Implement reactive regeneration: event channel listener that triggers generate on data changes
- [x] 4.4 Write integration tests for full config generation pipeline
