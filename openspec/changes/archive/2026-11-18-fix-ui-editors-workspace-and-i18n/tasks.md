# Tasks

## 1. Preserve Hidden Model Fields in Manual Node and Routing Editors

- [x] 1.1 Store initial `WsSettings.headers` inside `TransportRows` in `crates/ui/src/nodes.rs` and preserve it when saving a WebSocket manual node, verifying unit tests confirm custom WebSocket headers survive round-trip edits
- [x] 1.2 Preserve `existing.enabled` and `existing.group` when editing a routing rule in `crates/ui/src/preferences/routing.rs`, and verify unit tests cover editing disabled and grouped rules

## 2. Repair DNS Server Dialog, Host Override Editor, and Effective DNS Inputs

- [x] 2.1 Fix `show_server_dialog` in `crates/ui/src/preferences/dns/servers.rs` so `port = None` initializes the spinbutton to `protocol.default_port()` and saves back as `None` when uncustomized, and `detour = None` initializes to `"direct"` on `sing-box`, verifying helper/unit tests cover both invariants
- [x] 2.2 Update `crates/ui/src/preferences/dns/hosts.rs` to edit by `(original_domain, original_ip)`, add an edit button to each row in `render_dns_hosts`, and reject duplicate `(domain, ip)` entries inline, verifying multi-IP host edit and duplicate-rejection unit tests pass
- [x] 2.3 Pass enabled subscription and manual node addresses into `effective_dns(...)` in `crates/ui/src/preferences/dns/mod.rs` (updating when subscriptions or manual nodes change), and verify `xray` TUN hostname-bootstrap rows appear in Effective DNS

## 3. Route Corrupt Routing Reset Through WorkspaceStore

- [x] 3.1 Add `WorkspaceStore::reset_routing_rules(&self)` in `crates/ui/src/workspace.rs` to clear `routing_load_error` and persist an empty `RoutingRuleSet` via the background writer, replace `std::fs::remove_file` in `crates/ui/src/preferences/routing.rs`, and verify `WorkspaceStore` unit tests cover corrupt-file reset

## 4. Wire Gettext Localization Safely

- [x] 4.1 Update `crates/ui/src/i18n.rs` and early `bootstrap.rs` startup (before Tokio/worker thread spawn) to initialize `LocaleCategory::LcMessages` without touching `LC_NUMERIC`, resolve installed and dev `locale/` paths, and verify unit tests confirm numeric formatting stays `.`-separated
- [x] 4.2 Route primary window, status bar, Subscriptions/Nodes, Logs, Wizard, and Preferences strings through `i18n::tr`, surface a restart notice on `Language` change in `crates/ui/src/preferences/system.rs`, and update `locale/ru_RU/LC_MESSAGES/v2ray-rs.po` and `.mo` (verifying `msgfmt --check` passes)
