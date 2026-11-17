use super::*;
pub(super) fn backend_display_name(b: BackendType) -> &'static str {
    match b {
        BackendType::V2ray => "v2ray",
        BackendType::Xray => "xray",
        BackendType::SingBox => "sing-box",
    }
}

pub(crate) fn strategy_family_note(backend: BackendType) -> Option<&'static str> {
    match backend {
        BackendType::Xray | BackendType::V2ray => Some(
            "Prefer IPv4 and Prefer IPv6 query only the preferred address family on this backend",
        ),
        BackendType::SingBox => None,
    }
}

pub(crate) fn detour_note(backend: BackendType) -> Option<&'static str> {
    match backend {
        BackendType::SingBox => Some(
            "Both detours work: 'proxy' sends DNS through the proxy, 'direct' dials it directly",
        ),
        BackendType::Xray => {
            Some("Only 'direct' has an effect, and only while TUN is enabled; 'proxy' does nothing")
        }
        BackendType::V2ray => None,
    }
}

pub(super) fn private_dns_warning(
    server: &DnsServerConfig,
    backend: BackendType,
    tun_enabled: bool,
) -> Option<&'static str> {
    server
        .resolves_via_proxy_private(backend, tun_enabled)
        .then_some(PRIVATE_DNS_WARNING)
}

/// Names of enabled subscriptions whose imported provider profile carries its
/// own DNS config — those nodes bypass the app-wide DNS settings.
pub(super) fn dns_profile_subscription_names(subscriptions: &[Subscription]) -> Vec<String> {
    subscriptions
        .iter()
        .filter(|sub| {
            sub.enabled
                && sub.use_imported_profile
                && sub
                    .imported_profile
                    .as_ref()
                    .is_some_and(|profile| profile.dns.is_some())
        })
        .map(|sub| sub.name.clone())
        .collect()
}

/// Pure rendering of the effective-DNS summary: one line per resolver, with
/// consecutive bootstrap resolvers collapsed into a single explanatory row,
/// plus one row per DNS-carrying profile subscription.
pub(super) fn effective_dns_rows(
    summary: &EffectiveDns,
    profile_subscriptions: &[String],
) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut in_bootstrap_run = false;
    for entry in summary.entries() {
        if entry.source() == DnsSource::Bootstrap {
            if !in_bootstrap_run {
                in_bootstrap_run = true;
                rows.push("bootstrap resolvers for proxy hostnames (xray TUN)".to_string());
            }
            continue;
        }
        in_bootstrap_run = false;
        let transport = entry
            .transport()
            .map(protocol_display_name)
            .map(|name| format!(" [{name}]"))
            .unwrap_or_default();
        rows.push(format!(
            "{}{} path={} source={} scope={}",
            entry.address(),
            transport,
            entry.path().as_str(),
            entry.source().as_str(),
            entry.scope()
        ));
    }
    rows.extend(
        profile_subscriptions
            .iter()
            .map(|name| format!("nodes from \"{name}\" use the imported profile's DNS instead")),
    );
    rows
}

pub(super) fn primary_dns_subtitle(
    server: &DnsServerConfig,
    backend: BackendType,
    tun_enabled: bool,
) -> String {
    server_row_subtitle(server, backend, tun_enabled)
}

pub(super) fn validate_dns_settings_for_backend(settings: &AppSettings) -> Result<(), String> {
    settings.dns.validate().map_err(|err| err.to_string())
}

pub(super) fn primary_row_missing_note(tag: &str, use_custom_rules: bool) -> &'static str {
    if use_custom_rules {
        "Not configured - set up in Advanced"
    } else if tag == AUTO_SPLIT_REMOTE_TAG {
        "Not configured - routing-derived DNS rules for proxied domains are skipped"
    } else {
        "Not configured - routing-derived DNS rules for direct traffic are skipped"
    }
}
