use super::RuleMatch;
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
    #[error("invalid domain keyword: {0}")]
    InvalidDomainKeyword(String),
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
    #[error("invalid test URL: {0} (must be an http:// or https:// URL)")]
    InvalidTestUrl(String),
    #[error("invalid tun interface name: {0}")]
    InvalidTunInterface(String),
    #[error("invalid tun mtu: {0} (must be 576-9000)")]
    InvalidTunMtu(u16),
    #[error("invalid process name: {0}")]
    InvalidProcessName(String),
}

/// Validates a Real Delay test URL: must parse as a URL with an `http` or
/// `https` scheme. Other schemes and unparseable inputs are rejected.
pub fn validate_test_url(url_str: &str) -> Result<(), ValidationError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|_| ValidationError::InvalidTestUrl(url_str.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(ValidationError::InvalidTestUrl(url_str.to_string())),
    }
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

/// Validates a TUN interface name: non-empty, at most 15 characters (IFNAMSIZ
/// minus the NUL terminator), restricted to lowercase letters, digits, `_` and `-`.
pub fn validate_tun_interface_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() || name.len() > 15 {
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

/// Validates a domain keyword: a bare substring match, so unlike
/// [`validate_domain_pattern`] it has neither a required dot nor a
/// leading-dot restriction — only non-empty and free of whitespace.
pub fn validate_domain_keyword(keyword: &str) -> Result<(), ValidationError> {
    if keyword.is_empty() || keyword.chars().any(char::is_whitespace) {
        return Err(ValidationError::InvalidDomainKeyword(keyword.to_string()));
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
        RuleMatch::DomainFull { domain } => validate_domain_pattern(domain),
        RuleMatch::IpCidr { .. } => Ok(()),
        RuleMatch::Protocol { name } => validate_protocol_name(name),
        RuleMatch::Port { spec } => validate_port_spec(spec),
        RuleMatch::Network { spec } => validate_network_spec(spec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
