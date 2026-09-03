use crate::models::{AppSettings, DnsServerConfig, ProxyNode};

pub(crate) fn outbound_tag(node: &ProxyNode, index: usize) -> String {
    match node.remark() {
        Some(name) if !name.is_empty() => format!("proxy-{index}-{name}"),
        _ => format!("proxy-{index}"),
    }
}

/// The server that answers for names excluded from the tunnel. A server
/// detoured to `direct` is the only one that actually resolves outside it;
/// failing that, the first server on the default route keeps the old behavior.
pub(crate) fn split_horizon_server(settings: &AppSettings) -> Option<&DnsServerConfig> {
    let servers = &settings.dns.servers;
    servers
        .iter()
        .find(|s| s.detours_direct())
        .or_else(|| servers.first())
}
