use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::route::{RouteAttribute, RouteScope};
use rtnetlink::packet_route::rule::{RuleAttribute, RuleMessage};
use rtnetlink::{Handle, IpVersion, LinkUnspec, RouteMessageBuilder, new_connection};

/// Route table sing-box uses for its `auto_route` policy routing. Used by the
/// recovery pass to flush rules/routes left behind after a SIGKILL.
const SINGBOX_ROUTE_TABLE: u32 = 2022;

const EEXIST: i32 = -17;

pub fn connect() -> Result<Handle, String> {
    let (connection, handle, _) =
        new_connection().map_err(|e| format!("open netlink socket: {e}"))?;
    tokio::spawn(connection);
    Ok(handle)
}

/// Brings the interface up, assigns the address(es) (ignoring an already-present
/// address), and installs the `0.0.0.0/1` + `128.0.0.0/1` split routes bound to
/// the device (plus the IPv6 `::/1` + `8000::/1` equivalents when an IPv6 address
/// is supplied). Every step is idempotent.
pub async fn xray_up(
    handle: &Handle,
    iface: &str,
    v4: (IpAddr, u8),
    v6: Option<(IpAddr, u8)>,
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

    add_route_v4(handle, index, Ipv4Addr::UNSPECIFIED, 1).await?;
    add_route_v4(handle, index, Ipv4Addr::new(128, 0, 0, 0), 1).await?;
    if v6.is_some() {
        add_route_v6(handle, index, Ipv6Addr::UNSPECIFIED, 1).await?;
        add_route_v6(handle, index, Ipv6Addr::new(0x8000, 0, 0, 0, 0, 0, 0, 0), 1).await?;
    }

    Ok(())
}

/// Deletes the interface, removing its addresses and device-scoped routes. A
/// no-op when the device is already absent.
pub async fn xray_down(handle: &Handle, iface: &str) -> Result<(), String> {
    if let Some(index) = link_index(handle, iface).await? {
        handle
            .link()
            .del(index)
            .execute()
            .await
            .map_err(|e| format!("delete link {iface}: {e}"))?;
    }
    Ok(())
}

/// Recovers leftover xray TUN state: removes the device if present.
pub async fn recover_xray(handle: &Handle, iface: &str) -> Result<(), String> {
    xray_down(handle, iface).await
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
        Ok(None) => Ok(None),
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

async fn add_route_v4(
    handle: &Handle,
    index: u32,
    dest: Ipv4Addr,
    prefix: u8,
) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(dest, prefix)
        .output_interface(index)
        .scope(RouteScope::Link)
        .build();
    match handle.route().add(route).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add route {dest}/{prefix}: {e}")),
    }
}

async fn add_route_v6(
    handle: &Handle,
    index: u32,
    dest: Ipv6Addr,
    prefix: u8,
) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv6Addr>::new()
        .destination_prefix(dest, prefix)
        .output_interface(index)
        .scope(RouteScope::Link)
        .build();
    match handle.route().add(route).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_exists(&e) => Ok(()),
        Err(e) => Err(format!("add route {dest}/{prefix}: {e}")),
    }
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
