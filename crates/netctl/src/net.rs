use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::AddressFamily;
use rtnetlink::packet_route::IpProtocol;
use rtnetlink::packet_route::route::{RouteAttribute, RouteMessage, RouteScope, RouteType};
use rtnetlink::packet_route::rule::{
    RuleAction, RuleAttribute, RuleMessage, RulePortRange, RuleUidRange,
};
use rtnetlink::{Handle, IpVersion, LinkUnspec, RouteMessageBuilder, new_connection};

pub struct Context {
    pub handle: Handle,
    pub owner: Option<crate::ownership::Owner>,
}

impl std::ops::Deref for Context {
    type Target = Handle;
    fn deref(&self) -> &Handle {
        &self.handle
    }
}

impl Context {
    async fn add_route(
        &mut self,
        route: rtnetlink::packet_route::route::RouteMessage,
    ) -> Result<(), String> {
        if let Some(owner) = self.owner.as_mut()
            && !owner
                .begin(
                    &self.handle,
                    v2ray_rs_tun_session::Resource::Route(crate::ownership::encode(&route)),
                )
                .await?
        {
            return Ok(());
        }
        self.add_prepared_route(route).await
    }

    async fn add_prepared_route(
        &mut self,
        route: rtnetlink::packet_route::route::RouteMessage,
    ) -> Result<(), String> {
        match self.handle.route().add(route).execute().await {
            Ok(()) => {}
            Err(error) if is_exists(&error) && self.owner.is_none() => {}
            Err(error) => return Err(format!("add route: {error}")),
        }
        if let Some(owner) = self.owner.as_mut() {
            owner.finish(&self.handle).await?;
        }
        Ok(())
    }

    async fn add_rule(&mut self, rule: RuleMessage) -> Result<(), String> {
        if let Some(owner) = self.owner.as_mut()
            && !owner
                .begin(
                    &self.handle,
                    v2ray_rs_tun_session::Resource::Rule(crate::ownership::encode(&rule)),
                )
                .await?
        {
            return Ok(());
        }
        let mut request = self.handle.rule().add();
        *request.message_mut() = rule;
        match request.execute().await {
            Ok(()) => {}
            Err(error) if is_exists(&error) && self.owner.is_none() => {}
            Err(error) => return Err(format!("add rule: {error}")),
        }
        if let Some(owner) = self.owner.as_mut() {
            owner.finish(&self.handle).await?;
        }
        Ok(())
    }
}

/// Route table sing-box uses for its `auto_route` policy routing. Used by the
/// recovery pass to flush rules/routes left behind after a SIGKILL.
use v2ray_rs_tun_session::SINGBOX_ROUTE_TABLE;

/// Dedicated table holding xray's TUN default route. Kept out of `main` so that
/// marked packets (xray's own sockets) can still reach the real default via
/// `main`, while everything else is funnelled into the tunnel.
pub(crate) const XRAY_ROUTE_TABLE: u32 = 2023;

/// fwmark xray stamps on its own outbound sockets via `streamSettings.sockopt.mark`.
/// The bypass rule below diverts marked packets to `main`, breaking the otherwise
/// infinite `tun-in -> direct` loop. Must match `XRAY_TUN_FWMARK` in v2ray-rs-core.
pub const XRAY_FWMARK: u32 = 255;

const RT_TABLE_MAIN: u32 = 254;

/// Per-UID bypass: traffic owned by the bypass UID reaches `main` ahead of the
/// capture rules, so it egresses the real interface instead of the tunnel.
const RULE_PREF_BYPASS_UID: u32 = 8998;

/// Route exclusions: destination prefixes that look up `main` ahead of every
/// other xray rule, so they leave through the real interface even when DNS
/// capture would otherwise pull a `:53` flow into the tunnel.
pub(crate) const RULE_PREF_EXCLUDE: u32 = 8997;

/// DNS capture: port-53 traffic reaches the tunnel table ahead of the rule that
/// keeps LAN routes working, so a resolver on the local subnet stops being the
/// one exception the tunnel never sees. Matched on an unmarked fwmark so the
/// proxy's own resolver queries still take the bypass at
/// [`RULE_PREF_BYPASS`] instead of looping back in.
const RULE_PREF_DNS: u32 = 8999;

const DNS_PORT: u16 = 53;

/// Policy-rule priorities, evaluated after `local` (0) and before `main` (32766).
const RULE_PREF_BYPASS: u32 = 9000;
const RULE_PREF_MAIN: u32 = 9001;
const RULE_PREF_TUN: u32 = 9002;

/// Every priority `xray_up` and `replace_exclusions` install, torn down together.
const XRAY_RULE_PREFS: [u32; 6] = [
    RULE_PREF_EXCLUDE,
    RULE_PREF_BYPASS_UID,
    RULE_PREF_DNS,
    RULE_PREF_BYPASS,
    RULE_PREF_MAIN,
    RULE_PREF_TUN,
];

/// Metric of the strict-mode `unreachable` default in [`XRAY_ROUTE_TABLE`]. The
/// highest possible value sorts it behind the TUN device route, so it only
/// answers once the kernel has dropped that route along with the device.
const FALLBACK_METRIC: u32 = u32::MAX;

const ENOENT: i32 = -2;
const ESRCH: i32 = -3;
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
///
/// With `strict`, an `unreachable` default is added to the tunnel table for both
/// families and the IPv6 policy rules are installed even without an IPv6
/// address, so traffic fails closed instead of leaking out `main` when the device
/// disappears. Without an IPv6 address those IPv6 steps are skipped when the host
/// has IPv6 disabled; otherwise an IPv6 failure fails the start. Every step is
/// idempotent.
pub async fn xray_up(
    handle: &mut Context,
    iface: &str,
    v4: (IpAddr, u8),
    v6: Option<(IpAddr, u8)>,
    bypass_uid: Option<u32>,
    capture_dns: bool,
    strict: bool,
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
    let v6_rules = v6_rules_needed(v6.is_some(), strict, host_has_ipv6());
    if capture_dns {
        add_dns_capture_rules(handle, AddressFamily::Inet).await?;
    }
    // Captured even without a tunnel address: a strict session must not let
    // on-link IPv6 resolvers leak the lookup onto the LAN.
    if ipv6_dns_capture(v6_rules, capture_dns) {
        add_dns_capture_rules(handle, AddressFamily::Inet6).await?;
    }
    if let Some(uid) = bypass_uid {
        add_bypass_uid_rule(handle, AddressFamily::Inet, uid).await?;
    }
    if strict {
        add_fallback_route_v4(handle).await?;
        if v6_rules {
            add_fallback_route_v6(handle).await?;
        }
    }
    if v6.is_some() {
        add_default_route_v6(handle, index).await?;
    }
    if v6_rules {
        add_xray_rules(handle, AddressFamily::Inet6).await?;
        if let Some(uid) = bypass_uid {
            add_bypass_uid_rule(handle, AddressFamily::Inet6, uid).await?;
        }
    }

    Ok(())
}

/// Replaces the route exclusions: every [`RULE_PREF_EXCLUDE`] rule is removed
/// for both families, then one rule per entry sends that destination to `main`,
/// in order. IPv6 entries are skipped when the host has IPv6 disabled. Already
/// present rules (two entries equal after normalization) are not an error.
pub async fn replace_exclusions(
    handle: &mut Context,
    exclude: &[(IpAddr, u8)],
) -> Result<(), String> {
    if let Some(owner) = handle.owner.as_mut() {
        owner.remove_exclusions(&handle.handle).await?;
    } else {
        del_rules_matching(handle, |rule| {
            rule_priority(rule) == Some(RULE_PREF_EXCLUDE)
        })
        .await?;
    }
    let has_ipv6 = host_has_ipv6();
    for &(ip, prefix) in exclude {
        if exclusion_installable(ip, has_ipv6) {
            add_exclusion_rule(handle, ip, prefix).await?;
        }
    }
    Ok(())
}

fn exclusion_installable(ip: IpAddr, host_has_ipv6: bool) -> bool {
    ip.is_ipv4() || host_has_ipv6
}

/// Whether `xray_up` installs the IPv6 policy rules (and, with `strict`, the
/// IPv6 fallback route). An explicit IPv6 address always wants them; strict mode
/// wants them to refuse IPv6, unless the host has no IPv6 to leak.
fn v6_rules_needed(v6_requested: bool, strict: bool, host_has_ipv6: bool) -> bool {
    v6_requested || (strict && host_has_ipv6)
}

/// IPv6 port-53 capture follows the IPv6 policy rules, not the presence of a
/// tunnel address, so strict sessions close the on-link resolver path.
fn ipv6_dns_capture(v6_rules: bool, capture_dns: bool) -> bool {
    v6_rules && capture_dns
}

/// `/proc/sys/net/ipv6` is absent when the kernel boots with `ipv6.disable=1`,
/// and is per network namespace.
pub(crate) fn host_has_ipv6() -> bool {
    std::path::Path::new("/proc/sys/net/ipv6").exists()
}

/// Removes the policy rules, flushes [`XRAY_ROUTE_TABLE`] for both families and
/// deletes the interface (which drops its addresses and device-scoped routes).
/// The rules and the strict fallback routes outlive the device, so they are torn
/// down explicitly. A no-op when all are already absent.
pub async fn xray_down(handle: &Handle, iface: &str) -> Result<(), String> {
    let rules_result = del_xray_rules(handle).await;
    // Flushed before the device delete, so a failing delete cannot leave the
    // fallback routes blackholing traffic after the session ended.
    let routes_result = flush_table_routes(handle, XRAY_ROUTE_TABLE).await;

    // Only ever delete a TUN device. If the name resolves to something else
    // (a bridge, a WireGuard link, a physical NIC) leave it alone — rules are
    // still torn down above, which is all that's needed once the device is gone.
    let device_result = async {
        if crate::validate::is_tun_device(iface)
            && let Some(index) = link_index(handle, iface).await?
        {
            match handle.link().del(index).execute().await {
                Ok(()) => {}
                Err(e) if is_no_such_device(&e) => {}
                Err(e) => return Err(format!("delete link {iface}: {e}")),
            }
        }
        Ok(())
    }
    .await;
    combine_cleanup([rules_result, routes_result, device_result])
}

/// Recovers leftover xray TUN state: removes the policy rules and the device,
/// then flushes any routes orphaned in the dedicated table after a SIGKILL.
pub async fn recover_xray(handle: &Handle, iface: &str) -> Result<(), String> {
    let down = xray_down(handle, iface).await;
    let routes = flush_table_routes(handle, XRAY_ROUTE_TABLE).await;
    combine_cleanup([down, routes])
}

/// Recovers leftover sing-box routing state.
pub async fn recover_singbox(handle: &Handle, iface: &str) -> Result<(), String> {
    let down = xray_down(handle, iface).await;
    let rules = flush_table_rules(handle, SINGBOX_ROUTE_TABLE).await;
    let routes = flush_table_routes(handle, SINGBOX_ROUTE_TABLE).await;
    combine_cleanup([down, rules, routes])
}

fn combine_cleanup(results: impl IntoIterator<Item = Result<(), String>>) -> Result<(), String> {
    let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
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

async fn add_default_route_v4(handle: &mut Context, index: u32) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .output_interface(index)
        .table_id(XRAY_ROUTE_TABLE)
        .scope(RouteScope::Link)
        .build();
    handle.add_route(route).await
}

async fn add_default_route_v6(handle: &mut Context, index: u32) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv6Addr>::new()
        .destination_prefix(Ipv6Addr::UNSPECIFIED, 0)
        .output_interface(index)
        .table_id(XRAY_ROUTE_TABLE)
        .scope(RouteScope::Link)
        .build();
    handle.add_route(route).await
}

async fn add_fallback_route_v4(handle: &mut Context) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .table_id(XRAY_ROUTE_TABLE)
        .kind(RouteType::Unreachable)
        .priority(FALLBACK_METRIC)
        .build();
    handle.add_route(route).await
}

async fn add_fallback_route_v6(handle: &mut Context) -> Result<(), String> {
    let route = RouteMessageBuilder::<Ipv6Addr>::new()
        .destination_prefix(Ipv6Addr::UNSPECIFIED, 0)
        .table_id(XRAY_ROUTE_TABLE)
        .kind(RouteType::Unreachable)
        .priority(FALLBACK_METRIC)
        .build();
    handle.add_route(route).await
}

/// Installs the three policy rules for one address family:
/// 1. marked packets (xray's own sockets) look up `main`, reaching the real
///    default route instead of the tunnel;
/// 2. unmarked packets look up `main` with the default route suppressed, so LAN
///    and link routes keep working;
/// 3. everything else falls through to the tunnel's dedicated table.
async fn add_xray_rules(handle: &mut Context, family: AddressFamily) -> Result<(), String> {
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

/// Steers port-53 traffic into the tunnel table for one address family.
///
/// Without this a resolver on the local subnet is reached through the LAN route
/// that [`RULE_PREF_MAIN`] deliberately preserves, so every name the host looks
/// up is resolved outside the tunnel. `fwmark 0/XRAY_FWMARK` excludes the
/// proxy's own upstream queries, which carry the mark and must keep egressing
/// the real interface.
async fn add_dns_capture_rules(handle: &mut Context, family: AddressFamily) -> Result<(), String> {
    for proto in [IpProtocol::Udp, IpProtocol::Tcp] {
        let mut req = handle.rule().add();
        {
            let msg = req.message_mut();
            msg.header.family = family;
            msg.header.action = RuleAction::ToTable;
            msg.attributes.push(RuleAttribute::Table(XRAY_ROUTE_TABLE));
            msg.attributes.push(RuleAttribute::Priority(RULE_PREF_DNS));
            msg.attributes.push(RuleAttribute::FwMark(0));
            msg.attributes.push(RuleAttribute::FwMask(XRAY_FWMARK));
            msg.attributes.push(RuleAttribute::IpProtocol(proto));
            msg.attributes
                .push(RuleAttribute::DestinationPortRange(RulePortRange {
                    start: DNS_PORT,
                    end: DNS_PORT,
                }));
        }
        handle.add_rule(req.message_mut().clone()).await?;
    }
    Ok(())
}

async fn add_rule(
    handle: &mut Context,
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
    handle.add_rule(req.message_mut().clone()).await
}

async fn add_exclusion_rule(handle: &mut Context, ip: IpAddr, prefix: u8) -> Result<(), String> {
    let mut req = handle.rule().add();
    {
        let msg = req.message_mut();
        msg.header.family = match ip {
            IpAddr::V4(_) => AddressFamily::Inet,
            IpAddr::V6(_) => AddressFamily::Inet6,
        };
        msg.header.action = RuleAction::ToTable;
        msg.header.table = RT_TABLE_MAIN as u8;
        msg.header.dst_len = prefix;
        msg.attributes
            .push(RuleAttribute::Priority(RULE_PREF_EXCLUDE));
        msg.attributes.push(RuleAttribute::Destination(ip));
    }
    handle.add_rule(req.message_mut().clone()).await
}

/// Diverts traffic owned by `uid` to `main` at [`RULE_PREF_BYPASS_UID`], ahead of
/// the capture rules, so the bypass user's sockets reach the real default
/// instead of the tunnel. Installed per family when `--bypass-uid` is set.
async fn add_bypass_uid_rule(
    handle: &mut Context,
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
    handle.add_rule(req.message_mut().clone()).await
}

/// Deletes the policy rules `xray_up` installs, across both families. Matches on
/// our reserved priorities so unrelated rules are left untouched.
async fn del_xray_rules(handle: &Handle) -> Result<(), String> {
    del_rules_matching(handle, is_xray_rule).await
}

async fn del_rules_matching(
    handle: &Handle,
    matches: impl Fn(&RuleMessage) -> bool,
) -> Result<(), String> {
    let mut results = Vec::new();
    for version in [IpVersion::V4, IpVersion::V6] {
        let mut rules = handle.rule().get(version).execute();
        loop {
            let rule = match rules.try_next().await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(err) => {
                    results.push(Err(format!("enumerate rules: {err}")));
                    break;
                }
            };
            if matches(&rule) {
                results.push(del_rule(handle, rule).await);
            }
        }
    }
    combine_cleanup(results)
}

pub(crate) fn is_xray_rule(rule: &RuleMessage) -> bool {
    rule_priority(rule).is_some_and(|p| XRAY_RULE_PREFS.contains(&p))
}

pub(crate) fn rule_priority(rule: &RuleMessage) -> Option<u32> {
    rule.attributes.iter().find_map(|attr| match attr {
        RuleAttribute::Priority(p) => Some(*p),
        _ => None,
    })
}

async fn flush_table_rules(handle: &Handle, table: u32) -> Result<(), String> {
    let mut results = Vec::new();
    for version in [IpVersion::V4, IpVersion::V6] {
        let mut rules = handle.rule().get(version).execute();
        loop {
            let rule = match rules.try_next().await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(err) => {
                    results.push(Err(format!("enumerate rules: {err}")));
                    break;
                }
            };
            if rule_table(&rule) == table {
                results.push(del_rule(handle, rule).await);
            }
        }
    }
    combine_cleanup(results)
}

async fn flush_table_routes(handle: &Handle, table: u32) -> Result<(), String> {
    let mut results = Vec::new();
    let v4 = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let v6 = RouteMessageBuilder::<Ipv6Addr>::new().build();
    for query in [v4, v6] {
        let mut routes = handle.route().get(query).execute();
        loop {
            let route = match routes.try_next().await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(err) => {
                    results.push(Err(format!("enumerate routes: {err}")));
                    break;
                }
            };
            let route_table = route
                .attributes
                .iter()
                .find_map(|a| match a {
                    RouteAttribute::Table(t) => Some(*t),
                    _ => None,
                })
                .unwrap_or(route.header.table as u32);
            if route_table == table {
                results.push(del_route(handle, route).await);
            }
        }
    }
    combine_cleanup(results)
}

pub(crate) fn rule_table(rule: &RuleMessage) -> u32 {
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

pub(crate) fn is_no_such_device(err: &rtnetlink::Error) -> bool {
    matches!(err, rtnetlink::Error::NetlinkError(msg) if msg.code.map(|c| c.get()) == Some(ENODEV))
}

/// Netlink's "no such object" answers for rule and route deletes: ENOENT and
/// ESRCH. An object that vanished between the snapshot and the delete leaves
/// the deletion satisfied; anything else is a genuine error.
pub(crate) fn is_no_such_object(err: &rtnetlink::Error) -> bool {
    matches!(err, rtnetlink::Error::NetlinkError(msg)
        if msg
            .code
            .map(|c| c.get())
            .is_some_and(|code| code == ENOENT || code == ESRCH))
}

async fn del_rule(handle: &Handle, rule: RuleMessage) -> Result<(), String> {
    match handle.rule().del(rule).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_no_such_object(&e) => Ok(()),
        Err(e) => Err(format!("delete rule: {e}")),
    }
}

async fn del_route(handle: &Handle, route: RouteMessage) -> Result<(), String> {
    match handle.route().del(route).execute().await {
        Ok(()) => Ok(()),
        Err(e) if is_no_such_object(&e) => Ok(()),
        Err(e) => Err(format!("delete route: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{
        RULE_PREF_EXCLUDE, XRAY_FWMARK, XRAY_RULE_PREFS, exclusion_installable, ipv6_dns_capture,
        v6_rules_needed,
    };
    use v2ray_rs_core::config::XRAY_TUN_FWMARK;

    #[test]
    fn cleanup_reports_every_failure() {
        assert_eq!(super::combine_cleanup([Ok(()), Ok(())]), Ok(()));
        let error = super::combine_cleanup([Err("rules".into()), Ok(()), Err("routes".into())])
            .unwrap_err();
        assert!(error.contains("rules"));
        assert!(error.contains("routes"));
    }

    /// Guards against the two fwmark constants drifting: xray stamps its own
    /// outbound sockets with one value and the route helper's bypass rule
    /// matches the other, so a mismatch silently loops direct traffic back into
    /// the tunnel.
    #[test]
    fn fwmark_matches_core() {
        assert_eq!(XRAY_FWMARK, XRAY_TUN_FWMARK);
    }

    #[test]
    fn v6_rules_follow_address_strict_and_host_support() {
        let cases = [
            // (v6_requested, strict, host_has_ipv6, want)
            (true, true, true, true),
            (true, true, false, true),
            (true, false, true, true),
            (true, false, false, true),
            (false, true, true, true),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, false),
        ];
        for (v6_requested, strict, host_has_ipv6, want) in cases {
            assert_eq!(
                v6_rules_needed(v6_requested, strict, host_has_ipv6),
                want,
                "v6_requested={v6_requested} strict={strict} host_has_ipv6={host_has_ipv6}"
            );
        }
    }

    #[test]
    fn strict_ipv4_only_session_captures_ipv6_dns_when_host_supports_it() {
        let v6_rules = v6_rules_needed(false, true, true);
        assert!(v6_rules);
        assert!(ipv6_dns_capture(v6_rules, true));
    }

    #[test]
    fn disabled_host_skips_ipv6_dns_capture() {
        let v6_rules = v6_rules_needed(false, true, false);
        assert!(!v6_rules);
        assert!(!ipv6_dns_capture(v6_rules, true));
        assert!(!ipv6_dns_capture(true, false));
    }

    #[test]
    fn exclusion_installable_skips_v6_without_host_ipv6() {
        let v4: IpAddr = "10.0.0.0".parse().unwrap();
        let v6: IpAddr = "2001:db8::".parse().unwrap();
        let cases = [
            (v4, true, true),
            (v4, false, true),
            (v6, true, true),
            (v6, false, false),
        ];
        for (ip, host_has_ipv6, want) in cases {
            assert_eq!(
                exclusion_installable(ip, host_has_ipv6),
                want,
                "ip={ip} host_has_ipv6={host_has_ipv6}"
            );
        }
    }

    #[test]
    fn xray_rule_prefs_cover_exclusions() {
        assert!(XRAY_RULE_PREFS.contains(&RULE_PREF_EXCLUDE));
        for pref in XRAY_RULE_PREFS.iter().filter(|&&p| p != RULE_PREF_EXCLUDE) {
            assert!(RULE_PREF_EXCLUDE < *pref, "pref={pref}");
        }
    }
    #[test]
    #[cfg(feature = "privileged-tests")]
    fn journaled_route_mutations_in_private_namespace() {
        const CHILD: &str = "V2RAY_RS_ROUTE_JOURNAL_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new("timeout")
                .args(["60s", "unshare", "-rn", "--"])
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "net::tests::journaled_route_mutations_in_private_namespace",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .env(
                    "V2RAY_RS_ROUTE_JOURNAL_PARENT_NS",
                    v2ray_rs_tun_session::Epoch::current()
                        .unwrap()
                        .namespace_inode
                        .to_string(),
                )
                .status()
                .unwrap();
            assert!(
                status.success(),
                "private namespace journal acceptance failed"
            );
            return;
        }
        let parent: u64 = std::env::var("V2RAY_RS_ROUTE_JOURNAL_PARENT_NS")
            .unwrap()
            .parse()
            .unwrap();
        assert_ne!(
            parent,
            v2ray_rs_tun_session::Epoch::current()
                .unwrap()
                .namespace_inode
        );
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use rtnetlink::packet_route::rule::{RuleAttribute, RuleMessage};
                use rtnetlink::{LinkUnspec, RouteMessageBuilder};
                use v2ray_rs_tun_session::{Ownership, Phase, Session};
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("journal.json");
                let session = Session {
                    backend: "xray".to_string(),
                    iface: "ownedtun0".to_string(),
                    ownership: Some(Ownership::new().unwrap()),
                };
                let id = session.ownership.as_ref().unwrap().session_id.to_string();
                v2ray_rs_tun_session::write(&path, &session).unwrap();
                let handle = super::connect().unwrap();
                let mut owner = crate::ownership::Owner::open(&path, "ownedtun0", &id).unwrap();
                owner.reserve(&handle).await.unwrap();
                let mut context = super::Context {
                    handle,
                    owner: Some(owner),
                };
                context
                    .handle
                    .link()
                    .set(LinkUnspec::new_with_index(1).up().build())
                    .execute()
                    .await
                    .unwrap();
                let route = RouteMessageBuilder::<std::net::Ipv4Addr>::new()
                    .destination_prefix(std::net::Ipv4Addr::UNSPECIFIED, 0)
                    .output_interface(1)
                    .table_id(super::XRAY_ROUTE_TABLE)
                    .scope(super::RouteScope::Link)
                    .build();
                let intent =
                    v2ray_rs_tun_session::Resource::Route(crate::ownership::encode(&route));
                assert!(
                    context
                        .owner
                        .as_mut()
                        .unwrap()
                        .begin(&context.handle, intent)
                        .await
                        .unwrap()
                );
                context
                    .handle
                    .route()
                    .add(route.clone())
                    .execute()
                    .await
                    .unwrap();
                assert!(context.add_prepared_route(route.clone()).await.is_err());
                let collided = v2ray_rs_tun_session::read::<String>(&path)
                    .unwrap()
                    .ownership
                    .unwrap();
                assert!(collided.pending.is_some());
                assert!(collided.resources.is_empty());
                assert!(
                    context
                        .owner
                        .as_mut()
                        .unwrap()
                        .cleanup(&context.handle)
                        .await
                        .is_err()
                );
                context
                    .handle
                    .route()
                    .del(route.clone())
                    .execute()
                    .await
                    .unwrap();
                context
                    .owner
                    .as_mut()
                    .unwrap()
                    .reserve(&context.handle)
                    .await
                    .unwrap();
                context.add_route(route.clone()).await.unwrap();
                context.add_route(route.clone()).await.unwrap();
                if super::host_has_ipv6() {
                    let route = RouteMessageBuilder::<std::net::Ipv6Addr>::new()
                        .destination_prefix(std::net::Ipv6Addr::UNSPECIFIED, 0)
                        .output_interface(1)
                        .table_id(super::XRAY_ROUTE_TABLE)
                        .scope(super::RouteScope::Link)
                        .build();
                    context.add_route(route).await.unwrap();
                }
                let mut rule = RuleMessage::default();
                rule.header.family = super::AddressFamily::Inet;
                rule.header.action = super::RuleAction::ToTable;
                rule.attributes = vec![
                    RuleAttribute::Priority(super::RULE_PREF_TUN),
                    RuleAttribute::Table(super::XRAY_ROUTE_TABLE),
                ];
                context.add_rule(rule.clone()).await.unwrap();
                context.add_rule(rule).await.unwrap();
                let saved = v2ray_rs_tun_session::read::<String>(&path).unwrap();
                assert!(saved.ownership.as_ref().unwrap().pending.is_none());
                assert!(saved.ownership.as_ref().unwrap().resources.len() >= 2);

                let other_path = dir.path().join("other.json");
                let other = Session {
                    backend: "xray".to_string(),
                    iface: "otherownedtun0".to_string(),
                    ownership: Some(Ownership::new().unwrap()),
                };
                let other_id = other.ownership.as_ref().unwrap().session_id.to_string();
                v2ray_rs_tun_session::write(&other_path, &other).unwrap();
                let mut other_owner =
                    crate::ownership::Owner::open(&other_path, "otherownedtun0", &other_id)
                        .unwrap();
                assert!(other_owner.reserve(&context.handle).await.is_err());

                context.handle.route().del(route).execute().await.unwrap();
                let foreign = RouteMessageBuilder::<std::net::Ipv4Addr>::new()
                    .destination_prefix("192.0.2.0".parse().unwrap(), 24)
                    .output_interface(1)
                    .table_id(super::XRAY_ROUTE_TABLE)
                    .scope(super::RouteScope::Link)
                    .build();
                context
                    .handle
                    .route()
                    .add(foreign.clone())
                    .execute()
                    .await
                    .unwrap();
                assert!(
                    context
                        .owner
                        .as_mut()
                        .unwrap()
                        .cleanup(&context.handle)
                        .await
                        .is_err()
                );
                assert_eq!(v2ray_rs_tun_session::read::<String>(&path).unwrap(), saved);
                context.handle.route().del(foreign).execute().await.unwrap();
                context
                    .owner
                    .as_mut()
                    .unwrap()
                    .cleanup(&context.handle)
                    .await
                    .unwrap();
                context
                    .owner
                    .as_mut()
                    .unwrap()
                    .cleanup(&context.handle)
                    .await
                    .unwrap();
                assert!(
                    v2ray_rs_tun_session::read::<String>(&path)
                        .unwrap()
                        .ownership
                        .unwrap()
                        .resources
                        .is_empty()
                );
                assert_eq!(
                    v2ray_rs_tun_session::read::<String>(&path)
                        .unwrap()
                        .ownership
                        .unwrap()
                        .phase,
                    Phase::Complete
                );
            });
    }
}
