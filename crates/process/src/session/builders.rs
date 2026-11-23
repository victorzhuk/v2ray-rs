use super::*;

pub(super) const STRICT_ROUTE_NOTICE: &str = "notice: kernel IPv6 is disabled; sing-box strict_route turned off for this session (IPv4 routing unchanged)";

pub(super) const FALLBACK_DNS_DISABLED_NOTICE_XRAY: &str = "DNS is disabled; TUN resolves through fallback DoH 1.1.1.1/8.8.8.8 via the proxy. Enable DNS in Preferences to use your servers";

pub(super) const FALLBACK_DNS_DISABLED_NOTICE_SINGBOX: &str = "DNS is disabled; TUN resolves through the fallback DoH 1.1.1.1 via the proxy. Enable DNS in Preferences to use your servers";

pub(super) const FALLBACK_DNS_SCOPED_NOTICE: &str = "Every configured DNS server is domain-scoped; the fallback 1.1.1.1 answers all other domains. Add an unscoped server in Preferences to change that";

/// Selects the fallback-DNS notice for a candidate, or `None` when the
/// effective DNS layout needs no explanation.
pub(super) fn fallback_dns_notice(
    summary: &EffectiveDns,
    dns_enabled: bool,
    tun: bool,
    backend: BackendType,
) -> Option<&'static str> {
    if !tun {
        return None;
    }
    if !dns_enabled {
        return match backend {
            BackendType::Xray => Some(FALLBACK_DNS_DISABLED_NOTICE_XRAY),
            BackendType::SingBox => Some(FALLBACK_DNS_DISABLED_NOTICE_SINGBOX),
            BackendType::V2ray => None,
        };
    }
    if summary.uses_fallback() {
        Some(FALLBACK_DNS_SCOPED_NOTICE)
    } else {
        None
    }
}

pub(super) const PIN_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const PIN_LOOKUP_CONCURRENCY: usize = 16;

pub(super) const PIN_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);

pub(super) const PIN_PROBE_TOTAL: Duration = Duration::from_secs(2);

pub(super) async fn os_lookup(host: String, port: u16) -> std::io::Result<Vec<IpAddr>> {
    let addrs = tokio::net::lookup_host((host.as_str(), port)).await?;
    Ok(addrs.map(|addr| addr.ip()).collect())
}

/// Looks up every host concurrently within one overall `deadline`. A host
/// whose lookup fails, comes back empty, or is still running at the deadline
/// takes its `last_good` addresses, or is left out when there are none.
pub(super) async fn resolve_pins<F, Fut>(
    hosts: Vec<(String, u16)>,
    last_good: &[HostOverride],
    deadline: Duration,
    lookup: F,
) -> HashMap<String, Vec<IpAddr>>
where
    F: Fn(String, u16) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'static,
{
    let mut resolved = HashMap::new();
    if hosts.is_empty() {
        return resolved;
    }

    let until = tokio::time::Instant::now() + deadline;
    let permits = Arc::new(Semaphore::new(PIN_LOOKUP_CONCURRENCY));
    let mut lookups = JoinSet::new();
    for (host, port) in hosts.iter().cloned() {
        let permits = Arc::clone(&permits);
        let lookup = lookup.clone();
        lookups.spawn(async move {
            let _permit = permits.acquire_owned().await;
            let result = lookup(host.clone(), port).await;
            (host, result)
        });
    }

    let mut answers = HashMap::new();
    while let Ok(Some(joined)) = tokio::time::timeout_at(until, lookups.join_next()).await {
        if let Ok((host, result)) = joined {
            answers.insert(host, result);
        }
    }
    // Aborts whatever is still looking up past the deadline.
    drop(lookups);

    for (host, _) in hosts {
        let failure = match answers.remove(&host) {
            Some(Ok(addrs)) if !addrs.is_empty() => {
                resolved.insert(host, unique(addrs));
                continue;
            }
            Some(Ok(_)) => "no addresses".to_string(),
            Some(Err(err)) => err.to_string(),
            None => "lookup timed out".to_string(),
        };
        log::warn!("cannot pin {host}: {failure}");
        let fallback = unique(
            last_good
                .iter()
                .filter(|h| h.domain == host)
                .filter_map(|h| h.ip.parse::<IpAddr>().ok()),
        );
        if !fallback.is_empty() {
            log::warn!("pin {host}: using last good addresses");
            resolved.insert(host, fallback);
        }
    }
    resolved
}

fn unique(ips: impl IntoIterator<Item = IpAddr>) -> Vec<IpAddr> {
    let mut out = Vec::new();
    for ip in ips {
        if !out.contains(&ip) {
            out.push(ip);
        }
    }
    out
}

pub(super) fn pins_enabled(settings: &AppSettings) -> bool {
    settings.tun.enabled && settings.backend.backend_type != BackendType::V2ray
}

/// Hostname pins resolved before the first candidate starts. Strict xray TUN
/// installs fail-closed routes before its first start, so every lookup after
/// that is captured by the tunnel; only the first
/// [`TUN_PIN_LOOKAHEAD`] planned candidates are resolved up front.
pub(super) const TUN_PIN_LOOKAHEAD: usize = 8;

/// Hostnames to pin for one candidate's node set, under its effective
/// settings. The first port seen for a hostname wins.
pub(super) fn pin_hosts_for(nodes: &[ProxyNode], settings: &AppSettings) -> Vec<(String, u16)> {
    let mut hosts: Vec<(String, u16)> = Vec::new();
    for node in nodes {
        let host = node.address();
        if host.parse::<std::net::IpAddr>().is_ok()
            || settings.dns.hosts.iter().any(|h| h.domain == host)
            || hosts.iter().any(|(known, _)| known == host)
        {
            continue;
        }
        hosts.push((host.to_string(), node.port()));
    }
    hosts
}

/// Hostnames to pin before any candidate starts: each candidate node and its
/// via nodes, under that candidate's effective settings. The first port seen
/// for a hostname wins.
pub(super) fn pin_hosts(
    candidates: &[ConnectionCandidate],
    subscriptions: &[Subscription],
    manual_nodes: &[ManualNode],
    enabled_rules: &[RoutingRule],
    settings: &AppSettings,
) -> Vec<(String, u16)> {
    let mut hosts: Vec<(String, u16)> = Vec::new();
    for candidate in candidates {
        let (mut rules, effective) =
            resolve_effective_config(&candidate.node_ref, subscriptions, enabled_rules, settings);
        if !pins_enabled(&effective) {
            continue;
        }
        let via = resolve_via_nodes(&mut rules, subscriptions, manual_nodes);
        let nodes: Vec<ProxyNode> = std::iter::once(candidate.node.clone()).chain(via).collect();
        for (host, port) in pin_hosts_for(&nodes, &effective) {
            if !hosts.iter().any(|(known, _)| known == &host) {
                hosts.push((host, port));
            }
        }
    }
    hosts
}

/// Addresses from the last successful connection, keyed by hostname, for hosts
/// whose live lookup must not run under installed kill-switch routes.
pub(super) fn last_good_resolution(
    hosts: &[(String, u16)],
    last_good: &[HostOverride],
) -> HashMap<String, Vec<IpAddr>> {
    let mut resolved = HashMap::new();
    for (host, _) in hosts {
        let addrs = unique(
            last_good
                .iter()
                .filter(|h| &h.domain == host)
                .filter_map(|h| h.ip.parse::<IpAddr>().ok()),
        );
        if !addrs.is_empty() {
            resolved.insert(host.clone(), addrs);
        }
    }
    resolved
}

/// Pins each hostname-addressed node to its resolved addresses in `dns.hosts`.
/// Every family is carried; the generator keeps what its backend can use. A
/// hostname the user already overrides keeps that override. Returns the pins
/// added, never the user's overrides.
pub(super) fn apply_pins(
    settings: &mut AppSettings,
    nodes: &[ProxyNode],
    resolved: &HashMap<String, Vec<IpAddr>>,
) -> Vec<HostOverride> {
    let overridden: Vec<String> = settings
        .dns
        .hosts
        .iter()
        .map(|h| h.domain.clone())
        .collect();
    let first_pin = settings.dns.hosts.len();
    let mut seen: Vec<&str> = Vec::new();
    for node in nodes {
        let host = node.address();
        if host.parse::<IpAddr>().is_ok()
            || overridden.iter().any(|d| d == host)
            || seen.contains(&host)
        {
            continue;
        }
        seen.push(host);
        let Some(addrs) = resolved.get(host) else {
            continue;
        };
        settings
            .dns
            .hosts
            .extend(addrs.iter().map(|ip| HostOverride {
                domain: host.to_string(),
                ip: ip.to_string(),
            }));
    }
    settings.dns.hosts[first_pin..].to_vec()
}

pub(super) fn probe_allowed(parked: Option<&ProcessManager>) -> bool {
    parked.is_none_or(|mgr| !mgr.has_tun_runtime())
}

/// Narrows the resolved addresses of every multi-address node hostname to the
/// ones accepting a connect on that node's port. Hosts are probed concurrently
/// so one candidate spends at most `PIN_PROBE_TOTAL` here.
pub(super) async fn probe_pins(
    nodes: &[ProxyNode],
    resolved: &HashMap<String, Vec<IpAddr>>,
) -> HashMap<String, Vec<IpAddr>> {
    let mut probes = JoinSet::new();
    let mut probed = HashMap::new();
    for node in nodes {
        let host = node.address();
        let Some(addrs) = resolved.get(host) else {
            continue;
        };
        if probed.contains_key(host) {
            continue;
        }
        probed.insert(host.to_string(), addrs.clone());
        if addrs.len() < 2 {
            continue;
        }
        let (host, addrs, port) = (host.to_string(), addrs.clone(), node.port());
        probes.spawn(async move {
            let kept = filter_reachable(
                &host,
                &addrs,
                port,
                PIN_PROBE_CONNECT_TIMEOUT,
                PIN_PROBE_TOTAL,
            )
            .await;
            (host, kept)
        });
    }
    while let Some(joined) = probes.join_next().await {
        if let Ok((host, kept)) = joined {
            probed.insert(host, kept);
        }
    }
    probed
}

/// Keeps the addresses that accept a TCP connect on `port`, in input order.
/// When none does within the budgets, every address is kept: an unreachable
/// probe is no reason to leave a node unpinned.
pub(super) async fn filter_reachable(
    host: &str,
    addrs: &[IpAddr],
    port: u16,
    per_connect: Duration,
    total: Duration,
) -> Vec<IpAddr> {
    if addrs.is_empty() {
        return Vec::new();
    }
    let until = tokio::time::Instant::now() + total;
    let mut probes = JoinSet::new();
    for &ip in addrs {
        probes.spawn(async move {
            let connect = tokio::net::TcpStream::connect(SocketAddr::new(ip, port));
            (
                ip,
                matches!(tokio::time::timeout(per_connect, connect).await, Ok(Ok(_))),
            )
        });
    }
    let mut reachable = Vec::new();
    while let Ok(Some(joined)) = tokio::time::timeout_at(until, probes.join_next()).await {
        if let Ok((ip, true)) = joined {
            reachable.push(ip);
        }
    }
    let kept: Vec<IpAddr> = addrs
        .iter()
        .copied()
        .filter(|ip| reachable.contains(ip))
        .collect();
    if kept.is_empty() {
        log::warn!("pin probe: no address of {host} accepted on port {port}; keeping all");
        addrs.to_vec()
    } else {
        kept
    }
}

/// The first node hostname the settings the config is generated from cannot
/// resolve on their own. Capturing port 53 while the backend still needs the
/// OS resolver to find its own server would send that lookup into the tunnel it
/// is trying to build. An override xray answers with an empty set — the wrong
/// family for its query strategy — does not count.
pub(super) fn unpinned_host<'a>(settings: &AppSettings, nodes: &'a [ProxyNode]) -> Option<&'a str> {
    nodes.iter().map(ProxyNode::address).find(|host| {
        host.parse::<std::net::IpAddr>().is_err()
            && !settings
                .dns
                .hosts
                .iter()
                .any(|h| h.domain == *host && h.matches_strategy(settings.effective_dns_strategy()))
    })
}

pub(super) fn dns_capture_off_notice(host: &str) -> String {
    format!("notice: DNS capture off for this session: {host} has no usable pinned address")
}

pub(super) fn dns_capture_off_toast(host: &str) -> String {
    format!("DNS capture is off for this session: {host} has no usable pinned address")
}

pub(super) fn hijack_field(mode: DnsHijackMode) -> &'static str {
    match mode {
        DnsHijackMode::Hijack => "hijack",
        DnsHijackMode::Native => "native",
        DnsHijackMode::Disabled => "disabled",
    }
}

/// Host facts resolved once per connection off the async workers: the
/// bypass lookup is a blocking NSS query, and helper resolution stats the
/// filesystem and reads the mount table.
pub(super) struct HostTools {
    pub(super) bypass_uid: Option<u32>,
    pub(super) helper_path: PathBuf,
    pub(super) antidpi_path: PathBuf,
}

/// Builds the TUN runtime from settings, or `None` when TUN is off or the
/// backend is v2ray (which has no native TUN inbound).
pub(super) fn build_tun_runtime(
    settings: &AppSettings,
    nodes_pinned: bool,
    host: &HostTools,
) -> Option<TunRuntime> {
    if !settings.tun.enabled {
        return None;
    }
    let backend = settings.backend.backend_type;
    if backend == BackendType::V2ray {
        return None;
    }
    Some(TunRuntime {
        backend,
        iface: settings.tun.interface_name.clone(),
        addr_v4: settings.tun.address_v4.clone(),
        addr_v6: settings.tun.address_v6.clone(),
        helper_path: host.helper_path.clone(),
        bypass_uid: host.bypass_uid,
        // sing-box captures DNS itself via auto_route; only the xray path needs
        // the policy rule, and only when the config actually hijacks port 53.
        // `nodes_pinned` is the safety interlock: a node the generated config
        // cannot resolve on its own still needs the OS resolver to be found,
        // and capturing port 53 would swallow that lookup.
        capture_dns: backend == BackendType::Xray
            && settings.tun.dns_hijack == DnsHijackMode::Hijack
            && nodes_pinned,
        strict: settings.tun.strict_route,
        device_ifindex: None,
        journal: None,
        exclude_routes: if backend == BackendType::Xray {
            settings.tun.exclude_routes.clone()
        } else {
            Vec::new()
        },
    })
}

/// The warning to surface when TUN is on but the backend is v2ray, which has no
/// native TUN inbound: nothing is tunnelled, only the proxy endpoints work.
pub fn v2ray_tun_warning(settings: &AppSettings) -> Option<String> {
    if !settings.tun.enabled || settings.backend.backend_type != BackendType::V2ray {
        return None;
    }
    Some(format!(
        "TUN is not supported by v2ray; only apps using the SOCKS proxy at {} or the HTTP proxy at {} are proxied",
        listen_endpoint(&settings.listen_address, settings.socks_port),
        listen_endpoint(&settings.listen_address, settings.http_port),
    ))
}

/// Builds the anti-DPI runtime from settings, or `None` when the engine is
/// disabled. Gated on the switch only — NFQUEUE sees OUTPUT egress regardless
/// of backend or TUN, so v2ray (which has no TUN inbound) benefits too, and
/// the tunnel handshake's own ClientHello is covered. Shape-validates the
/// config like `TunConfig::validate()` at connect: a hand-edited settings
/// file must fail the candidate, not reach `install-rules --queue 0`.
pub(super) fn build_antidpi_runtime(
    settings: &AppSettings,
    antidpi_path: &Path,
) -> Result<Option<AntiDpiRuntime>, String> {
    if !settings.antidpi.enabled {
        return Ok(None);
    }
    settings.antidpi.validate().map_err(|e| e.to_string())?;
    Ok(Some(AntiDpiRuntime {
        helper_path: antidpi_path.to_path_buf(),
        queue: settings.antidpi.queue,
        strategies: settings.antidpi.strategies.clone(),
        hostlist: settings
            .antidpi
            .hostlist_path
            .as_deref()
            .map(std::path::PathBuf::from),
    }))
}

fn listen_endpoint(addr: &str, port: u16) -> String {
    if addr.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{addr}]:{port}")
    } else {
        format!("{addr}:{port}")
    }
}

/// sing-box cannot program its strict IPv6 rules on a host whose kernel has
/// IPv6 disabled and fails to start; xray's strict routing is done by netctl,
/// which already skips the IPv6 rules there.
pub(super) fn singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool {
    backend != BackendType::SingBox || host_has_ipv6
}
