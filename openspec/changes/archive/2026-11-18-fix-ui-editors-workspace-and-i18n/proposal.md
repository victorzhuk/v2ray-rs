# Proposal

## Why

Several GTK/Relm4 editor dialogs drop or corrupt existing state on save -- wiping custom WebSocket headers, clamping unset DNS server ports to `1`, stripping routing-rule `enabled` and `group` fields, and overwriting sibling IPs on multi-address host overrides -- while the Language setting has no effect because UI labels bypass `i18n::tr`.

## What Changes

- Preserve existing `WsSettings.headers` when editing and saving a WebSocket manual node in `crates/ui/src/nodes.rs`.
- Initialize the DNS server editor port spin control from `srv.port.unwrap_or_else(|| srv.protocol.default_port())` rather than `0` (which GTK clamps to `1`), and initialize `detour = None` as `"direct"` when `sing-box` is active so the dialog matches emitted routing.
- Key DNS host-override edits by `(domain, ip)`, reject duplicate `(domain, ip)` pairs, and expose an Edit button on each host row wired to `show_dns_host_dialog`.
- Preserve `RoutingRule.enabled` and `RoutingRule.group` when editing an existing routing rule, and route the Preferences Routing `"Reset Data"` recovery action through `WorkspaceStore::reset_routing_rules()` so the in-memory snapshot stays coherent with disk.
- Pass enabled subscription and manual node addresses into `effective_dns(...)` on the DNS Preferences page so `xray` TUN bootstrap rows reflect configured hostname nodes.
- Configure `i18n::init` to set message locale (`LC_MESSAGES` and `LANGUAGE`) without altering `LC_NUMERIC`, route primary window, dialog, and Preferences strings through `i18n::tr`, refresh `locale/ru_RU/LC_MESSAGES/v2ray-rs.{po,mo}`, and show a restart notice when the user changes `Language`.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `single-connection-management`: Preserve existing custom WebSocket headers when editing a manual node.
- `dns-preferences-ui`: Fix default-port initialization (`None` -> protocol default instead of `1`), align `sing-box` `detour = None` initial selection with `"direct"`, support `(domain, ip)` host editing, and pass configured proxy node addresses to `Effective DNS`.
- `routing-rules`: Preserve `enabled` and `group` fields when editing an existing routing rule.
- `workspace-architecture`: Route corrupt `routing_rules.json` resets through `WorkspaceStore::reset_routing_rules()`.
- `main-window`: Localize UI chrome, dialogs, and Preferences via `i18n::tr` and `gettext` message catalogs without corrupting `LC_NUMERIC`, with a restart notice on language change.

## Impact

- `crates/ui`: `nodes.rs`, `preferences/dns/{mod.rs,servers.rs,hosts.rs}`, `preferences/{routing.rs,system.rs,network.rs,tun.rs}`, `workspace.rs`, `i18n.rs`, `app/`, `subscriptions/`, `logs.rs`, `wizard.rs`
- `locale/ru_RU/LC_MESSAGES/v2ray-rs.{po,mo}` and `locale/en_US/LC_MESSAGES/v2ray-rs.{po,mo}`
- No new dependencies (`gettext-rs` and system `msgfmt` are already used by the workspace).
