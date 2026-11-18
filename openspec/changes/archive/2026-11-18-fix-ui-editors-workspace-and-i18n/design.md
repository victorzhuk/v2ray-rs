# Design

## Context

See `proposal.md` for motivation. Several GTK4/Relm4 dialogs in `crates/ui/` rebuild domain structs from widget state alone and drop unshown model fields or misinitialize `Option` values, `build_routing_error_page` still mutates `routing_rules.json` directly, `effective_dns` in Preferences lacks active node hostnames, and `crates/ui/src/i18n.rs` + `locale/` exist on disk without active `tr(...)` call sites or safe `LC_MESSAGES`-scoped initialization.

## Goals / Non-Goals

**Goals:**
- Preserve non-form model fields on edit across manual nodes (`WsSettings.headers`) and routing rules (`RoutingRule.enabled`, `RoutingRule.group`).
- Fix `show_server_dialog` port/detour initialization and `show_host_dialog` `(domain, ip)` edit/duplicate handling.
- Supply enabled subscription and manual node addresses to `effective_dns` in Preferences.
- Route corrupt-routing-file reset through `WorkspaceStore`.
- Wire `i18n::tr` across primary UI surfaces using `LocaleCategory::LcMessages` without corrupting `LC_NUMERIC`.

**Non-Goals:**
- Adding a full multi-header key-value editor widget for WebSocket headers in this change.
- Live hot-reloading every already-constructed GTK widget label in place without an application restart when `Language` changes.

## Decisions

### 1. Capture baseline unexposed fields in dialog state
- **Choice:**
  - In `crates/ui/src/nodes.rs`, store `ws_headers: BTreeMap<String, String>` (cloned from `TransportSettings::Ws(ws) => ws.headers.clone()`) inside `TransportRows`, and pass `headers: self.ws_headers.clone()` when saving `TransportSettings::Ws`.
  - In `crates/ui/src/preferences/routing.rs`, pass `existing.enabled` and `existing.group.clone()` when constructing the updated `RoutingRule` in the edit branch (`rule_index.is_some()`), defaulting to `enabled: true, group: None` only on new rules.

### 2. DNS server & host dialog state invariants
- **Choice:**
  - In `crates/ui/src/preferences/dns/servers.rs`, compute `init_port = existing.map(|(_, s)| s.port.unwrap_or_else(|| s.protocol.default_port())).unwrap_or_else(|| init_proto.default_port())` and `port_customized = Rc::new(Cell::new(existing.and_then(|(_, s)| s.port).is_some()))`. When `!port_customized.get()` and `raw_port == proto.default_port()`, save `port: None`. For `init_detour_idx`, default `None` to `1` (`"direct"`) when `backend == BackendType::SingBox`, keeping `0` (`"none"`) on `xray`/`v2ray`.
  - In `crates/ui/src/preferences/dns/hosts.rs`, pass `existing: Option<(&str, &str)>` (`(domain, ip)`), add an edit button (`document-edit-symbolic`) on each host row in `render_dns_hosts`, match the edited record by `(orig_domain, orig_ip)`, and reject saves if `(domain, ip)` already exists on another entry.

### 3. Node-aware Effective DNS in Preferences and `WorkspaceStore::reset_routing_rules`
- **Choice:**
  - Collect enabled node addresses from `workspace.subscriptions()` (`sub.enabled`, `sn.enabled`, `sn.node.address()`) and `workspace.manual_nodes()` (`mn.enabled`, `mn.node.address()`) in `crates/ui/src/preferences/dns/mod.rs` and pass `&node_addrs` to `effective_dns(...)`, subscribing the DNS page to subscription/manual-node snapshot updates alongside routing rules.
  - Add `pub fn reset_routing_rules(&self)` on `WorkspaceStore` (`crates/ui/src/workspace.rs`) that updates the in-memory `RoutingRuleSet` to `RoutingRuleSet::new()`, clears `routing_load_error`, enqueues an atomic `save_routing_rules` write on the background writer thread, and broadcasts the clean snapshot. Replace `std::fs::remove_file` in `crates/ui/src/preferences/routing.rs` with `workspace.reset_routing_rules()`.

### 4. `LC_MESSAGES`-only gettext initialization and catalog wiring
- **Choice:**
  - In `crates/ui/src/i18n.rs`, call `gettextrs::setlocale(LocaleCategory::LcMessages, locale_code)` (never `LcAll`) and set `LANGUAGE` before worker threads spawn during early `bootstrap` (or configure gettext domain lookup cleanly without mutating `LC_NUMERIC`), resolving locale directories from `V2RAY_RS_LOCALE_DIR`, `<exe_dir>/../share/locale`, `/usr/share/locale`, and repo `locale/` in dev builds.
  - Wrap user-facing strings in `app/`, `subscriptions/`, `nodes.rs`, `logs.rs`, `wizard.rs`, and `preferences/` with `i18n::tr(...)`, refresh `locale/ru_RU/LC_MESSAGES/v2ray-rs.po` and compiled `.mo`, and surface a restart hint in System Preferences when `Language` changes.
- **Why `LcMessages` over `LcAll`:** `LocaleCategory::LcAll` switches `LC_NUMERIC` to `,` in `ru_RU.UTF-8`, breaking C/GTK spinbutton and CSS/float formatting in mixed C libraries.

## Risks / Trade-offs

- **[Risk] `std::env::set_var("LANGUAGE", ...)` is `unsafe` in Rust 2024 if threads already exist** -> Mitigation: Perform locale initialization in `main` / early `bootstrap` **before** constructing the multi-threaded Tokio `Runtime` or spawning `WorkspaceStore` / D-Bus threads, and do not call `set_var` at runtime when Preferences changes `Language` (relying on persisted `settings.language` + restart notice).
