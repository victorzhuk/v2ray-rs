use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::AddressFamily;
use rtnetlink::packet_route::route::{RouteAttribute, RouteScope};
use rtnetlink::packet_route::rule::{RuleAction, RuleAttribute, RuleMessage, RuleUidRange};
use rtnetlink::{Handle, IpVersion, LinkUnspec, RouteMessageBuilder, new_connection};

/// Route table sing-box uses for its `auto_route` policy routing. Used by the
/// recovery pass to flush rules/routes left behind after a SIGKILL.
const SINGBOX_ROUTE_TABLE: u32 = 2022;

/// Dedicated table holding xray's TUN default route. Kept out of `main` so that
/// marked packets (xray's own sockets) can still reach the real default via
/// `main`, while everything else is funnelled into the tunnel.
const XRAY_ROUTE_TABLE: u32 = 2023;

/// fwmark xray stamps on its own outbound sockets via `streamSettings.sockopt.mark`.
/// The bypass rule below diverts marked packets to `main`, breaking the otherwise
/// infinite `tun-in -> direct` loop. Must match `XRAY_TUN_FWMARK` in v2ray-rs-core.
const XRAY_FWMARK: u32 = 255;

const RT_TABLE_MAIN: u32 = 254;

/// Per-UID bypass: traffic owned by the bypass UID reaches `main` ahead of the
/// capture rules, so it egresses the real interface instead of the tunnel.
const RULE_PREF_BYPASS_UID: u32 = 8998;

/// Policy-rule priorities, evaluated after `local` (0) and before `main` (32766).
const RULE_PREF_BYPASS: u32 = 9000;
const RULE_PREF_MAIN: u32 = 9001;
const RULE_PREF_TUN: u32 = 9002;

const EEXIST: i32 = -17;
const ENODEV: i32 = -19;

pub fn connect() -> Result<Handle, String> {
    let (connection, handle, _) =
        new_connection().map_err(|e| format!("open netlink socket: {e}"))?;
    tokio::spawn(connection);
    Ok(handle)
}

/// Brings the interface up, assigns the address(es) (ignoring an already-present
/// address), installs the TUN default route into [`XRAY_ROUTE_TABLE`], and adds
/// the policy rules that exempt xray's own marked sockets from the tunnel (so
/// `direct` traffic egresses the real interface instead of looping back in).
/// Every step is idempotent.
pub async fn xray_up(
    handle: &Handle,
    iface: &str,
    v4: (IpAddr, u8),
    v6: Option<(IpAddr, u8)>,
    bypass_uid: Option<u32>,
) -> Result<(), String> {
    let index = link_index(handle, iface)
        .await?
        .ok_or_else(|| format!("interface {iface} not found"))?;

    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .map_err(|e| format!("set link up: {e}"))?;

    add_address(handle, index, v4).await?;
    if let Some(v6) = v6 {
        add_address(handle, index, v6).await?;
    }

    add_default_route_v4(handle, index).await?;
    add_xray_rules(handle, AddressFamily::Inet).await?;
    if let Some(uid) = bypass_uid {
        add_bypass_uid_rule(handle, AddressFamily::Inet, uid).await?;
    }
    if v6.is_some() {
        add_default_route_v6(handle, index).await?;
        add_xray_rules(handle, AddressFamily::Inet6).await?;
        if let Some(uid) = bypass_uid {
            add_bypass_uid_rule(handle, AddressFamily::Inet6, uid).await?;
        }
    }

    Ok(())
}

/// Removes the policy rules and deletes the interface (which drops its addresses
/// and device-scoped routes). The rules outlive the device, so they are torn down
/// explicitly. A no-op when both are already absent.
pub async fn xray_down(handle: &Handle, iface: &str) -> Result<(), String> {
    del_xray_rules(handle).await;

    if let Some(index) = link_index(handle, iface).await? {
        match handle.link().del(index).execute().await {
            Ok(()) => {}
            // The device may vanish between the lookup and the delete (e.g. the
            // proxy exits); "already gone" is the desired outcome.
            Err(e) if is_no_such_device(&e) => {}
            Err(e) => return Err(format!("delete link {iface}: {e}")),
        }
    }
    Ok(())
}

/// Recovers leftover xray TUN state: removes the policy rules and the device,
/// then flushes any routes orphaned in the dedicated table after a SIGKILL.
pub async fn recover_xray(handle: &Handle, iface: &str) -> Result<(), String> {
    let _ = xray_down(handle, iface).await;
    flush_table_routes(handle, XRAY_ROUTE_TABLE).await;
    Ok(())
}

/// Recovers leftover sing-box TUN state: removes the device and flushes the
/// policy rules and routes sing-box leaves in its `auto_route` table.
pub async fn recover_singbox(handle: &Handle, iface: &str) -> Result<(), String> {
    let _ = xray_down(handle, iface).await;
    flush_table_rules(handle, SINGBOX_ROUTE_TABLE).await;
    flush_table_routes(handle, SINGBOX_ROUTE_TABLE).await;
    Ok(())
}

async fn link_index(handle: &Handle, iface: &str) -> Result<Option<u32>, String> {
    let mut links = handle.link().get().match_name(iface.to_string()).execute();
    match links.try_next().await {
        Ok(Some(link)) => Ok(Some(link.header.index)),
        // A name-filtered link lookup answers with ENODEV (not an empty dump)
        // when the device is absent; treat that as "not found" so xray-down and
        // recover stay idempotent.
        Ok(None) => Ok(None),
        Err(e) if is_no_such_device(&e) => Ok(None),
        Err(e) => Err(format!("look up interface {iface}: {e}")),
    }
}

async fn add_address(
    handle: &Handle,
    index: u32,
    (ip, prefix): (IpAddr, u8),
) -> Result<(), String> {
    match handle.address().add(index, ip, prefix).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add address {ip}/{prefix}: {e}")),
    }
}

async fn add_default_route_v4(handle: &Handle, index: u32) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .output_interface(index)
        .table_id(XRAY_ROUTE_TABLE)
        .scope(RouteScope::Link)
        .build();
    match handle.route().add(route).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add tun default route (v4): {e}")),
    }
}

async fn add_default_route_v6(handle: &Handle, index: u32) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv6Addr>::new()
        .destination_prefix(Ipv6Addr::UNSPECIFIED, 0)
        .output_interface(index)
        .table_id(XRAY_ROUTE_TABLE)
        .scope(RouteScope::Link)
        .build();
    match handle.route().add(route).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add tun default route (v6): {e}")),
    }
}

/// Installs the three policy rules for one address family:
/// 1. marked packets (xray's own sockets) look up `main`, reaching the real
///    default route instead of the tunnel;
/// 2. unmarked packets look up `main` with the default route suppressed, so LAN
///    and link routes keep working;
/// 3. everything else falls through to the tunnel's dedicated table.
async fn add_xray_rules(handle: &Handle, family: AddressFamily) -> Result<(), String> {
    add_rule(
        handle,
        family,
        RULE_PREF_BYPASS,
        RT_TABLE_MAIN,
        Some(XRAY_FWMARK),
        None,
    )
    .await?;
    add_rule(handle, family, RULE_PREF_MAIN, RT_TABLE_MAIN, None, Some(0)).await?;
    add_rule(handle, family, RULE_PREF_TUN, XRAY_ROUTE_TABLE, None, None).await?;
    Ok(())
}

async fn add_rule(
    handle: &Handle,
    family: AddressFamily,
    priority: u32,
    table: u32,
    fwmark: Option<u32>,
    suppress_prefixlen: Option<u32>,
) -> Result<(), String> {
    let mut req = handle.rule().add();
    {
        let msg = req.message_mut();
        msg.header.family = family;
        msg.header.action = RuleAction::ToTable;
        if table > 255 {
            msg.attributes.push(RuleAttribute::Table(table));
        } else {
            msg.header.table = table as u8;
        }
        msg.attributes.push(RuleAttribute::Priority(priority));
        if let Some(mark) = fwmark {
            msg.attributes.push(RuleAttribute::FwMark(mark));
        }
        if let Some(len) = suppress_prefixlen {
            msg.attributes.push(RuleAttribute::SuppressPrefixLen(len));
        }
    }
    match req.execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add policy rule (pref {priority}): {e}")),
    }
}

/// Diverts traffic owned by `uid` to `main` at [`RULE_PREF_BYPASS_UID`], ahead of
/// the capture rules, so the bypass user's sockets reach the real default
/// instead of the tunnel. Installed per family when `--bypass-uid` is set.
async fn add_bypass_uid_rule(
    handle: &Handle,
    family: AddressFamily,
    uid: u32,
) -> Result<(), String> {
    let mut req = handle.rule().add();
    {
        let msg = req.message_mut();
        msg.header.family = family;
        msg.header.action = RuleAction::ToTable;
        msg.header.table = RT_TABLE_MAIN as u8;
        msg.attributes
            .push(RuleAttribute::Priority(RULE_PREF_BYPASS_UID));
        msg.attributes.push(RuleAttribute::UidRange(RuleUidRange {
            start: uid,
            end: uid,
        }));
    }
    match req.execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!(
            "add bypass-uid rule (pref {RULE_PREF_BYPASS_UID}): {e}"
        )),
    }
}

/// Deletes the policy rules `xray_up` installs, across both families. Matches on
/// our reserved priorities so unrelated rules are left untouched.
async fn del_xray_rules(handle: &Handle) {
    for version in [IpVersion::V4, IpVersion::V6] {
        let mut rules = handle.rule().get(version).execute();
        while let Ok(Some(rule)) = rules.try_next().await {
            if is_xray_rule(&rule) {
                let _ = handle.rule().del(rule).execute().await;
            }
        }
    }
}

fn is_xray_rule(rule: &RuleMessage) -> bool {
    rule.attributes.iter().any(|attr| {
        matches!(
            attr,
            RuleAttribute::Priority(p)
                if [RULE_PREF_BYPASS_UID, RULE_PREF_BYPASS, RULE_PREF_MAIN, RULE_PREF_TUN].contains(p)
        )
    })
}

async fn flush_table_rules(handle: &Handle, table: u32) {
    for version in [IpVersion::V4, IpVersion::V6] {
        let mut rules = handle.rule().get(version).execute();
        while let Ok(Some(rule)) = rules.try_next().await {
            if rule_table(&rule) == table {
                let _ = handle.rule().del(rule).execute().await;
            }
        }
    }
}

async fn flush_table_routes(handle: &Handle, table: u32) {
    let v4 = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let v6 = RouteMessageBuilder::<Ipv6Addr>::new().build();
    for query in [v4, v6] {
        let mut routes = handle.route().get(query).execute();
        while let Ok(Some(route)) = routes.try_next().await {
            let route_table = route
                .attributes
                .iter()
                .find_map(|a| match a {
                    RouteAttribute::Table(t) => Some(*t),
                    _ => None,
                })
                .unwrap_or(route.header.table as u32);
            if route_table == table {
                let _ = handle.route().del(route).execute().await;
            }
        }
    }
}

fn rule_table(rule: &RuleMessage) -> u32 {
    rule.attributes
        .iter()
        .find_map(|a| match a {
            RuleAttribute::Table(t) => Some(*t),
            _ => None,
        })
        .unwrap_or(rule.header.table as u32)
}

fn is_exists(err: &rtnetlink::Error) -> bool {
    matches!(err, rtnetlink::Error::NetlinkError(msg) if msg.code.map(|c| c.get()) == Some(EEXIST))
}

fn is_no_such_device(err: &rtnetlink::Error) -> bool {
    matches!(err, rtnetlink::Error::NetlinkError(msg) if msg.code.map(|c| c.get()) == Some(ENODEV))
}
