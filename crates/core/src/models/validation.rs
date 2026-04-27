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
    #[error("invalid geosite category: {0}")]
    InvalidGeoSiteCategory(String),
    #[error("index out of bounds: {0}")]
    IndexOutOfBounds(usize),
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
        RuleMatch::IpCidr { .. } => Ok(()),
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
            RuleMatch::IpCidr {
                cidr: "192.168.1.0/24".parse().unwrap(),
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
