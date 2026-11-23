use super::tun::MAX_EXCLUDE_ROUTES;
use super::{DnsRuleMatch, RuleMatch};
use ipnet::IpNet;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("invalid country code: {0}")]
    InvalidCountryCode(String),
    #[error("invalid ip cidr: {0}")]
    InvalidIpCidr(String),
    #[error("invalid domain pattern: {0}")]
    InvalidDomainPattern(String),
    #[error("invalid domain suffix: {0}")]
    InvalidDomainSuffix(String),
    #[error("invalid domain keyword: {0}")]
    InvalidDomainKeyword(String),
    #[error(
        "invalid domain keyword '{0}': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'"
    )]
    WildcardDomainKeyword(String),
    #[error(
        "invalid full domain '{0}': full-domain rules match an exact host and cannot contain '*', use Domain or Domain Suffix instead"
    )]
    WildcardDomainFull(String),
    #[error("invalid geosite category: {0}")]
    InvalidGeoSiteCategory(String),
    #[error("invalid protocol name: {0}")]
    InvalidProtocolName(String),
    #[error("invalid port spec: {0}")]
    InvalidPortSpec(String),
    #[error("invalid network spec: {0}")]
    InvalidNetworkSpec(String),
    #[error("index out of bounds: {0}")]
    IndexOutOfBounds(usize),
    #[error("invalid listen address: {0}")]
    InvalidListenAddress(String),
    #[error("proxy port must be between 1 and 65535: {0}")]
    InvalidProxyPort(u16),
    #[error("socks_port and http_port must differ (both {0})")]
    DuplicateProxyPort(u16),
    #[error("invalid test URL: {0} (must be an http:// or https:// URL)")]
    InvalidTestUrl(String),
    #[error("invalid tun interface name: {0}")]
    InvalidTunInterface(String),
    #[error("invalid tun mtu: {0} (must be 576-9000)")]
    InvalidTunMtu(u16),
    #[error("too many excluded routes: {0} (at most {max})", max = MAX_EXCLUDE_ROUTES)]
    TooManyExcludedRoutes(usize),
    #[error("excluded route covers every address: {0}")]
    ExcludeRouteCoversAll(String),
    #[error("invalid tun address family: {field} must be an {expected} CIDR: {value}")]
    InvalidTunAddressFamily {
        field: &'static str,
        expected: &'static str,
        value: String,
    },
    #[error("invalid process name: {0}")]
    InvalidProcessName(String),
    #[error("invalid anti-DPI configuration: {0}")]
    InvalidAntiDpiConfig(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    pub scheme: ProbeScheme,
    pub host: String,
    pub port: u16,
    pub host_header: String,
    pub request_target: String,
}

/// Parses and validates a Real Delay / health probe URL: must be `http://` or
/// `https://` with a non-empty host, no userinfo credentials, no `#fragment`,
/// and no ASCII control characters.
pub fn parse_probe_target(url_str: &str) -> Result<ProbeTarget, ValidationError> {
    if url_str.chars().any(char::is_control) {
        return Err(ValidationError::InvalidTestUrl(url_str.to_string()));
    }
    let parsed = url::Url::parse(url_str)
        .map_err(|_| ValidationError::InvalidTestUrl(url_str.to_string()))?;
    let scheme = match parsed.scheme() {
        "http" => ProbeScheme::Http,
        "https" => ProbeScheme::Https,
        _ => return Err(ValidationError::InvalidTestUrl(url_str.to_string())),
    };
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(ValidationError::InvalidTestUrl(url_str.to_string()));
    }
    let Some(host) = parsed.host_str().filter(|h| !h.is_empty()) else {
        return Err(ValidationError::InvalidTestUrl(url_str.to_string()));
    };
    let default_port = match scheme {
        ProbeScheme::Http => 80,
        ProbeScheme::Https => 443,
    };
    let port = parsed.port_or_known_default().unwrap_or(default_port);
    let host_header = if port == default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    Ok(ProbeTarget {
        scheme,
        host: host.to_string(),
        port,
        host_header,
        request_target: parsed.as_str().to_string(),
    })
}

/// Validates a Real Delay test URL: must parse as an `http://` or `https://`
/// URL with a non-empty host, no credentials, no fragment, and no control
/// characters.
pub fn validate_test_url(url_str: &str) -> Result<(), ValidationError> {
    parse_probe_target(url_str).map(|_| ())
}

/// Validates a GeoIP country code or extended tag.
///
/// Accepts 2 or more uppercase ASCII characters. Used for both ISO 3166-1 country
/// codes (e.g., US, CN, RU) AND extended GeoIP tags (GOOGLE, FACEBOOK, NETFLIX,
/// PRIVATE, etc.).
pub fn validate_country_code(code: &str) -> Result<(), ValidationError> {
    if code.len() < 2 || !code.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ValidationError::InvalidCountryCode(code.to_string()));
    }
    Ok(())
}

pub fn validate_ip_cidr(cidr: &str) -> Result<(), ValidationError> {
    cidr.parse::<IpNet>()
        .map(|_| ())
        .map_err(|_| ValidationError::InvalidIpCidr(cidr.to_string()))
}

/// Validates that a TUN interface address belongs to the expected family:
/// `address_v4` must be IPv4 and `address_v6` must be IPv6. The caller has
/// already validated the CIDR syntax.
pub fn validate_tun_address_family(
    cidr: &str,
    field: &'static str,
    v6: bool,
) -> Result<(), ValidationError> {
    let net: IpNet = cidr
        .parse()
        .map_err(|_| ValidationError::InvalidIpCidr(cidr.to_string()))?;
    let matches_family = match net {
        IpNet::V4(_) => !v6,
        IpNet::V6(_) => v6,
    };
    if matches_family {
        Ok(())
    } else {
        Err(ValidationError::InvalidTunAddressFamily {
            field,
            expected: if v6 { "IPv6" } else { "IPv4" },
            value: cidr.to_string(),
        })
    }
}

/// Validates a TUN route exclusion: a CIDR with a non-zero prefix, since a
/// whole-address-space exclusion would route everything around the tunnel.
pub fn validate_exclude_route(cidr: &str) -> Result<(), ValidationError> {
    match cidr.parse::<IpNet>() {
        Ok(net) if net.prefix_len() > 0 => Ok(()),
        Ok(_) => Err(ValidationError::ExcludeRouteCoversAll(cidr.to_string())),
        Err(_) => Err(ValidationError::InvalidIpCidr(cidr.to_string())),
    }
}

/// Validates a TUN interface name: non-empty, at most 15 characters (IFNAMSIZ
/// minus the NUL terminator), restricted to lowercase letters, digits, `_` and `-`.
pub fn validate_tun_interface_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() || name.len() > 15 || name == v2ray_rs_tun_session::RESERVATION_NAME {
        return Err(ValidationError::InvalidTunInterface(name.to_string()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(ValidationError::InvalidTunInterface(name.to_string()));
    }
    Ok(())
}

pub fn validate_domain_pattern(pattern: &str) -> Result<(), ValidationError> {
    if pattern.is_empty() {
        return Err(ValidationError::InvalidDomainPattern(pattern.to_string()));
    }

    if pattern.starts_with('.') {
        return Err(ValidationError::InvalidDomainPattern(pattern.to_string()));
    }

    if !pattern.contains('.') {
        return Err(ValidationError::InvalidDomainPattern(pattern.to_string()));
    }

    let wildcard_prefix = pattern.strip_prefix("*.");
    let to_check = wildcard_prefix.unwrap_or(pattern);

    for c in to_check.chars() {
        if !c.is_alphanumeric() && c != '.' && c != '-' {
            return Err(ValidationError::InvalidDomainPattern(pattern.to_string()));
        }
    }

    if wildcard_prefix.is_some() && pattern.chars().filter(|&c| c == '*').count() > 1 {
        return Err(ValidationError::InvalidDomainPattern(pattern.to_string()));
    }

    Ok(())
}

/// Removes a single leading `.` from a domain suffix (sing-box `.ru` notation),
/// leaving the rest intact. `..ru` keeps one dot and stays invalid.
pub fn normalize_domain_suffix(suffix: &str) -> &str {
    suffix.strip_prefix('.').unwrap_or(suffix)
}

/// Validates a DNS domain suffix: an optional single leading `.` (sing-box
/// suffix notation, normalized away) or `*.`, followed by one or more ASCII
/// alphanumeric or hyphenated labels (allowing single-label TLD suffixes like
/// `ru`). The error payload carries the normalized value.
pub fn validate_domain_suffix(suffix: &str) -> Result<(), ValidationError> {
    let suffix = normalize_domain_suffix(suffix);
    let rest = suffix.strip_prefix("*.").unwrap_or(suffix);
    if rest.is_empty()
        || !rest.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return Err(ValidationError::InvalidDomainSuffix(suffix.to_string()));
    }
    Ok(())
}

/// Validates a full-domain matcher: an exact multi-label domain without `*`.
pub fn validate_domain_full(domain: &str) -> Result<(), ValidationError> {
    if domain.contains('*') {
        return Err(ValidationError::WildcardDomainFull(domain.to_string()));
    }
    validate_domain_pattern(domain)
}

/// Validates a domain keyword: a plain substring match — non-empty, no
/// whitespace, no `*` (wildcard suffixes belong to [`validate_domain_pattern`]
/// and the `Domain` rule type).
pub fn validate_domain_keyword(keyword: &str) -> Result<(), ValidationError> {
    if keyword.is_empty() || keyword.chars().any(char::is_whitespace) {
        return Err(ValidationError::InvalidDomainKeyword(keyword.to_string()));
    }
    if keyword.contains('*') {
        return Err(ValidationError::WildcardDomainKeyword(keyword.to_string()));
    }
    Ok(())
}

const SNIFFED_PROTOCOLS: &[&str] = &[
    "http",
    "tls",
    "quic",
    "stun",
    "dns",
    "bittorrent",
    "dtls",
    "ssh",
    "rdp",
    "ntp",
];

pub fn validate_protocol_name(name: &str) -> Result<(), ValidationError> {
    if SNIFFED_PROTOCOLS.contains(&name) {
        Ok(())
    } else {
        Err(ValidationError::InvalidProtocolName(name.to_string()))
    }
}

pub fn validate_network_spec(spec: &str) -> Result<(), ValidationError> {
    if spec.is_empty() {
        return Err(ValidationError::InvalidNetworkSpec(spec.to_string()));
    }
    for part in spec.split(',') {
        if part != "tcp" && part != "udp" {
            return Err(ValidationError::InvalidNetworkSpec(spec.to_string()));
        }
    }
    Ok(())
}

pub fn validate_port_spec(spec: &str) -> Result<(), ValidationError> {
    if spec.is_empty() {
        return Err(ValidationError::InvalidPortSpec(spec.to_string()));
    }
    for part in spec.split(',') {
        let valid = match part.split_once('-') {
            Some((start, end)) => match (start.parse::<u16>(), end.parse::<u16>()) {
                (Ok(start), Ok(end)) => start <= end,
                _ => false,
            },
            None => part.parse::<u16>().is_ok_and(|p| p != 0),
        };
        if !valid {
            return Err(ValidationError::InvalidPortSpec(spec.to_string()));
        }
    }
    Ok(())
}

pub fn validate_geosite_category(category: &str) -> Result<(), ValidationError> {
    if category.is_empty() {
        return Err(ValidationError::InvalidGeoSiteCategory(
            category.to_string(),
        ));
    }

    for c in category.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' && c != '.' && c != '!' {
            return Err(ValidationError::InvalidGeoSiteCategory(
                category.to_string(),
            ));
        }
    }

    Ok(())
}

pub fn validate_rule_match(m: &RuleMatch) -> Result<(), ValidationError> {
    match m {
        RuleMatch::GeoIp { country_code } => validate_country_code(country_code),
        RuleMatch::GeoSite { category } => validate_geosite_category(category),
        RuleMatch::Domain { pattern } => validate_domain_pattern(pattern),
        RuleMatch::DomainKeyword { keyword } => validate_domain_keyword(keyword),
        RuleMatch::DomainFull { domain } => validate_domain_full(domain),
        RuleMatch::IpCidr { .. } => Ok(()),
        RuleMatch::Protocol { name } => validate_protocol_name(name),
        RuleMatch::Port { spec } => validate_port_spec(spec),
        RuleMatch::Network { spec } => validate_network_spec(spec),
    }
}

pub fn validate_dns_rule_match(m: &DnsRuleMatch) -> Result<(), ValidationError> {
    match m {
        DnsRuleMatch::GeoSite { category } => validate_geosite_category(category),
        DnsRuleMatch::DomainSuffix { suffix } => validate_domain_suffix(suffix),
        DnsRuleMatch::DomainKeyword { keyword } => validate_domain_keyword(keyword),
        DnsRuleMatch::DomainFull { domain } => validate_domain_full(domain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_route_refuses_whole_address_space() {
        for cidr in ["0.0.0.0/0", "::/0"] {
            assert_eq!(
                validate_exclude_route(cidr),
                Err(ValidationError::ExcludeRouteCoversAll(cidr.to_string()))
            );
        }
        assert_eq!(
            validate_exclude_route("bogus"),
            Err(ValidationError::InvalidIpCidr("bogus".to_string()))
        );
    }

    #[test]
    fn exclude_route_accepts_prefix() {
        for cidr in ["10.0.0.0/8", "fd00::/64"] {
            assert_eq!(validate_exclude_route(cidr), Ok(()), "cidr={cidr}");
        }
    }

    #[test]
    fn test_validate_country_code() {
        let tests = vec![
            ("US", true),
            ("CN", true),
            ("RU", true),
            ("GOOGLE", true),
            ("PRIVATE", true),
            ("FACEBOOK", true),
            ("us", false),
            ("U", false),
            ("1X", false),
            ("A1", false),
            ("", false),
        ];

        for (code, expected_valid) in tests {
            let result = validate_country_code(code);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "code={} expected_valid={} got={:?}",
                code,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_ip_cidr() {
        let tests = vec![
            ("192.168.1.0/24", true),
            ("10.0.0.0/8", true),
            ("2001:db8::/32", true),
            ("192.168.1.1/32", true),
            ("192.168.1.0/33", false),
            ("256.1.1.1/24", false),
            ("not-an-ip", false),
            ("192.168.1.1", false),
            ("", false),
        ];

        for (cidr, expected_valid) in tests {
            let result = validate_ip_cidr(cidr);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "cidr={} expected_valid={} got={:?}",
                cidr,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_domain_pattern() {
        let tests = vec![
            ("example.com", true),
            ("sub.example.com", true),
            ("*.example.com", true),
            ("*.sub.example.com", true),
            ("example-site.com", true),
            (".example.com", false),
            ("example", false),
            ("", false),
            ("example.com*", false),
            ("*example.com", false),
            ("*.*.example.com", false),
            ("example$.com", false),
            ("exam ple.com", false),
        ];

        for (pattern, expected_valid) in tests {
            let result = validate_domain_pattern(pattern);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "pattern={} expected_valid={} got={:?}",
                pattern,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_geosite_category() {
        let tests = vec![
            ("google", true),
            ("geolocation-cn", true),
            ("geolocation-!cn", true),
            ("category-ads", true),
            ("tld-cn", true),
            ("category-ru", true),
            ("unknown-category", true),
            ("category-ai-!cn", true),
            ("category-ai-chat-!cn", true),
            ("Google", false),
            ("GOOGLE", false),
            ("", false),
            ("category with spaces", false),
            ("category_underscore", false),
        ];

        for (category, expected_valid) in tests {
            let result = validate_geosite_category(category);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "category={} expected_valid={} got={:?}",
                category,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_domain_keyword() {
        let tests = vec![
            ("sina", true),
            ("sina.com", true),
            (".example", true),
            ("", false),
            ("has space", false),
            ("*.ru", false),
            ("a*b", false),
        ];

        for (keyword, expected_valid) in tests {
            let result = validate_domain_keyword(keyword);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "keyword={} expected_valid={} got={:?}",
                keyword,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_protocol_name() {
        for name in ["http", "tls", "quic", "bittorrent", "dns"] {
            assert!(
                validate_protocol_name(name).is_ok(),
                "{name} should be valid"
            );
        }
        for name in ["HTTP", "socks", ""] {
            assert!(
                validate_protocol_name(name).is_err(),
                "{name} should be invalid"
            );
        }
    }

    #[test]
    fn test_validate_port_spec() {
        let tests = vec![
            ("53", true),
            ("1000-2000", true),
            ("80,443", true),
            ("53,1000-2000,80", true),
            ("0", false),
            ("2000-1000", false),
            ("abc", false),
            ("", false),
        ];

        for (spec, expected_valid) in tests {
            let result = validate_port_spec(spec);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "spec={} expected_valid={} got={:?}",
                spec,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_network_spec() {
        let tests = vec![
            ("tcp", true),
            ("udp", true),
            ("tcp,udp", true),
            ("icmp", false),
            ("TCP", false),
            ("", false),
        ];

        for (spec, expected_valid) in tests {
            let result = validate_network_spec(spec);
            assert_eq!(
                result.is_ok(),
                expected_valid,
                "spec={} expected_valid={} got={:?}",
                spec,
                expected_valid,
                result
            );
        }
    }

    #[test]
    fn test_validate_rule_match() {
        let valid_cases = vec![
            RuleMatch::GeoIp {
                country_code: "US".to_string(),
            },
            RuleMatch::GeoIp {
                country_code: "GOOGLE".to_string(),
            },
            RuleMatch::GeoSite {
                category: "google".to_string(),
            },
            RuleMatch::GeoSite {
                category: "category-ai-!cn".to_string(),
            },
            RuleMatch::Domain {
                pattern: "example.com".to_string(),
            },
            RuleMatch::DomainKeyword {
                keyword: "sina".to_string(),
            },
            RuleMatch::DomainFull {
                domain: "example.com".to_string(),
            },
            RuleMatch::IpCidr {
                cidr: "192.168.1.0/24".parse().unwrap(),
            },
            RuleMatch::Protocol {
                name: "bittorrent".to_string(),
            },
            RuleMatch::Port {
                spec: "1000-2000".to_string(),
            },
            RuleMatch::Network {
                spec: "tcp,udp".to_string(),
            },
        ];

        for m in valid_cases {
            assert!(validate_rule_match(&m).is_ok(), "should be valid: {:?}", m);
        }

        let invalid_cases = vec![
            RuleMatch::GeoIp {
                country_code: "1RU".to_string(),
            },
            RuleMatch::GeoSite {
                category: "INVALID".to_string(),
            },
            RuleMatch::Domain {
                pattern: ".example.com".to_string(),
            },
            RuleMatch::DomainKeyword {
                keyword: String::new(),
            },
            RuleMatch::DomainFull {
                domain: ".example.com".to_string(),
            },
            RuleMatch::DomainFull {
                domain: "*.example.com".to_string(),
            },
            RuleMatch::Protocol {
                name: "socks".to_string(),
            },
            RuleMatch::Port {
                spec: "abc".to_string(),
            },
            RuleMatch::Network {
                spec: "icmp".to_string(),
            },
        ];

        for m in invalid_cases {
            assert!(
                validate_rule_match(&m).is_err(),
                "should be invalid: {:?}",
                m
            );
        }
    }

    #[test]
    fn test_validate_and_parse_probe_target() {
        let valid_https = parse_probe_target("https://www.gstatic.com/generate_204").unwrap();
        assert_eq!(valid_https.scheme, ProbeScheme::Https);
        assert_eq!(valid_https.host, "www.gstatic.com");
        assert_eq!(valid_https.port, 443);
        assert_eq!(valid_https.host_header, "www.gstatic.com");

        let valid_http_port =
            parse_probe_target("http://probe.example.com:8080/generate_204?x=1").unwrap();
        assert_eq!(valid_http_port.scheme, ProbeScheme::Http);
        assert_eq!(valid_http_port.host, "probe.example.com");
        assert_eq!(valid_http_port.port, 8080);
        assert_eq!(valid_http_port.host_header, "probe.example.com:8080");
        assert_eq!(
            valid_http_port.request_target,
            "http://probe.example.com:8080/generate_204?x=1"
        );

        let valid_v6 = parse_probe_target("https://[2001:db8::1]:8443/ping").unwrap();
        assert_eq!(valid_v6.host, "[2001:db8::1]");
        assert_eq!(valid_v6.port, 8443);

        for invalid in [
            "not-a-url",
            "ftp://example.com/",
            "http://",
            "https://user:pass@example.com/",
            "https://user@example.com/",
            "https://example.com/generate_204#section",
            "https://[::1]extra/path",
            "https://example.com/\nX-Injected: 1",
        ] {
            assert!(
                validate_test_url(invalid).is_err(),
                "expected invalid test URL: {invalid:?}"
            );
        }
    }

    #[test]
    fn test_validate_domain_full() {
        assert_eq!(validate_domain_full("api.example.com"), Ok(()));
        assert_eq!(
            validate_domain_full("*.example.com"),
            Err(ValidationError::WildcardDomainFull("*.example.com".into()))
        );
        assert_eq!(
            validate_domain_full("foo*bar.com"),
            Err(ValidationError::WildcardDomainFull("foo*bar.com".into()))
        );
        assert!(matches!(
            validate_domain_full("localhost"),
            Err(ValidationError::InvalidDomainPattern(_))
        ));
    }

    #[test]
    fn test_validate_domain_suffix() {
        for valid in [
            "example.com",
            "*.example.com",
            "ru",
            "*.ru",
            ".ru",
            "sub-1.example.org",
        ] {
            assert_eq!(validate_domain_suffix(valid), Ok(()), "suffix={valid}");
        }
        for invalid in [
            "",
            "*.",
            ".",
            "..ru",
            "ru.",
            "a..b",
            "*.*.example.com",
            "foo*bar.com",
            "bad_domain.com",
        ] {
            assert_eq!(
                validate_domain_suffix(invalid),
                Err(ValidationError::InvalidDomainSuffix(
                    normalize_domain_suffix(invalid).to_string()
                )),
                "suffix={invalid}"
            );
        }
    }

    #[test]
    fn test_normalize_domain_suffix() {
        assert_eq!(normalize_domain_suffix(".ru"), "ru");
        assert_eq!(normalize_domain_suffix("ru"), "ru");
        assert_eq!(normalize_domain_suffix("*.ru"), "*.ru");
        assert_eq!(normalize_domain_suffix("..ru"), ".ru");
        assert_eq!(normalize_domain_suffix(""), "");
    }

    #[test]
    fn test_validate_dns_rule_match() {
        for valid in [
            DnsRuleMatch::GeoSite {
                category: "category-ru".into(),
            },
            DnsRuleMatch::DomainSuffix {
                suffix: "*.example.com".into(),
            },
            DnsRuleMatch::DomainSuffix {
                suffix: "ru".into(),
            },
            DnsRuleMatch::DomainSuffix {
                suffix: ".ru".into(),
            },
            DnsRuleMatch::DomainKeyword {
                keyword: "yandex".into(),
            },
            DnsRuleMatch::DomainFull {
                domain: "api.example.com".into(),
            },
        ] {
            assert!(validate_dns_rule_match(&valid).is_ok(), "{valid:?}");
        }

        assert!(matches!(
            validate_dns_rule_match(&DnsRuleMatch::DomainSuffix {
                suffix: "..ru".into(),
            }),
            Err(ValidationError::InvalidDomainSuffix(_))
        ));
        assert!(matches!(
            validate_dns_rule_match(&DnsRuleMatch::DomainFull {
                domain: "*.example.com".into(),
            }),
            Err(ValidationError::WildcardDomainFull(_))
        ));
        assert!(matches!(
            validate_dns_rule_match(&DnsRuleMatch::DomainKeyword {
                keyword: "*.ru".into(),
            }),
            Err(ValidationError::WildcardDomainKeyword(_))
        ));
        assert!(matches!(
            validate_dns_rule_match(&DnsRuleMatch::GeoSite {
                category: "GOOGLE".into(),
            }),
            Err(ValidationError::InvalidGeoSiteCategory(_))
        ));
    }
}
