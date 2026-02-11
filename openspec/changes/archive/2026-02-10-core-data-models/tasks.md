# Tasks: Core Data Models & Persistence Layer

## 1. Project Setup

- [x] 1.1 Initialize Cargo workspace with `core` library crate
- [x] 1.2 Add dependencies: serde, serde_json, toml, uuid, directories, thiserror

## 2. Proxy Node Models

- [x] 2.1 Define transport settings types (TCP, WebSocket, gRPC, HTTP/2)
- [x] 2.2 Define TLS settings type
- [x] 2.3 Define VlessConfig struct with all fields
- [x] 2.4 Define VmessConfig struct with all fields
- [x] 2.5 Define ShadowsocksConfig struct with all fields
- [x] 2.6 Define TrojanConfig struct with all fields
- [x] 2.7 Define ProxyNode enum with serde tagged union serialization
- [x] 2.8 Write tests for ProxyNode serialization round-trip

## 3. Subscription & Routing Models

- [x] 3.1 Define Subscription struct (id, name, url, nodes, last_updated, auto_update_interval, enabled)
- [x] 3.2 Define RoutingRule types: RuleMatch enum (GeoIp, GeoSite, Domain, IpCidr) and RuleAction enum
- [x] 3.3 Define ordered RoutingRuleSet with priority handling
- [x] 3.4 Write tests for routing rule model

## 4. App Configuration Models

- [x] 4.1 Define BackendType enum (V2ray, Xray, SingBox) and BackendConfig struct
- [x] 4.2 Define AppSettings struct (proxy ports, auto-update prefs, UI prefs, language)
- [x] 4.3 Implement Default for AppSettings with sensible defaults (SOCKS5: 1080, HTTP: 1081, English)
- [x] 4.4 Write tests for default settings

## 5. Persistence Layer

- [x] 5.1 Implement XDG path resolution using `directories` crate
- [x] 5.2 Implement first-launch directory creation with 0700 permissions
- [x] 5.3 Implement TOML serialization/deserialization for AppSettings
- [x] 5.4 Implement JSON serialization/deserialization for subscriptions and routing rules
- [x] 5.5 Implement atomic file writes (write to temp, then rename)
- [x] 5.6 Implement corrupt config fallback (fall back to defaults, warn user)
- [x] 5.7 Add config file version field for schema evolution
- [x] 5.8 Write integration tests for save/load round-trips
