## 1. URI Parsing

- [x] 1.1 Create `subscription` module with URI parser trait and common types
- [x] 1.2 Implement VLESS URI parser (extract uuid, host, port, transport, TLS, SNI, remark)
- [x] 1.3 Implement VMess URI parser (base64-decode JSON, extract v/ps/add/port/id/aid/net/type/host/path/tls)
- [x] 1.4 Implement Shadowsocks SIP002 URI parser (base64-decode method:password, extract host, port, remark)
- [x] 1.5 Implement Trojan URI parser (extract password, host, port, SNI, remark)
- [x] 1.6 Implement URI scheme dispatcher that routes to correct parser based on scheme prefix
- [x] 1.7 Write unit tests for each URI parser with valid and malformed inputs

## 2. Subscription Fetching & Decoding

- [x] 2.1 Add `reqwest` dependency with async/TLS features
- [x] 2.2 Implement HTTP fetcher with 30s connect / 60s total timeouts, redirect following, custom User-Agent
- [x] 2.3 Implement base64 response decoder that splits decoded content into individual URI lines
- [x] 2.4 Implement file-based import reading local file content and decoding identically to URL responses
- [x] 2.5 Implement partial-success import: collect parse errors per-URI, return valid nodes alongside error report
- [x] 2.6 Write integration tests for fetching and decoding flow

## 3. Subscription Storage

- [x] 3.1 Define subscription metadata struct (id, name, source URL/path, last update timestamp, node count, enabled)
- [x] 3.2 Define per-node storage struct with enabled/disabled flag
- [x] 3.3 Implement JSON serialization/deserialization for subscription data in `subscriptions/<uuid>.json`
- [x] 3.4 Implement CRUD operations: create, read, update, delete subscriptions
- [x] 3.5 Write tests for storage round-trip and multiple independent subscriptions

## 4. Auto-Update

- [x] 4.1 Implement configurable auto-update interval setting (stored in app config)
- [x] 4.2 Implement tokio interval timer that triggers subscription updates
- [x] 4.3 Implement exponential backoff retry (max 3 attempts) on update failure
- [x] 4.4 Implement node reconciliation: match by address+port+protocol, preserve enable/disable preferences
- [x] 4.5 Emit update status notifications to UI (success, failure, retrying)
- [x] 4.6 Write tests for reconciliation logic (node added, removed, preserved preferences)

## 5. Integration

- [x] 5.1 Wire subscription module into app initialization and shutdown lifecycle
- [x] 5.2 Expose subscription operations (import, update, delete, toggle node) to UI layer via messages
- [x] 5.3 Verify end-to-end flow: import URL → parse → store → update → reconcile
