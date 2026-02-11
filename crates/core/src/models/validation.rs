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

const VALID_COUNTRY_CODES: &[&str] = &[
    "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX",
    "AZ", "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ",
    "BR", "BS", "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK",
    "CL", "CM", "CN", "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM",
    "DO", "DZ", "EC", "EE", "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR",
    "GA", "GB", "GD", "GE", "GF", "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS",
    "GT", "GU", "GW", "GY", "HK", "HM", "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN",
    "IO", "IQ", "IR", "IS", "IT", "JE", "JM", "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN",
    "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC", "LI", "LK", "LR", "LS", "LT", "LU", "LV",
    "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK", "ML", "MM", "MN", "MO", "MP", "MQ",
    "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA", "NC", "NE", "NF", "NG", "NI",
    "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG", "PH", "PK", "PL", "PM",
    "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW", "SA", "SB", "SC",
    "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS", "ST", "SV",
    "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO", "TR",
    "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
    "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
];

const VALID_GEOSITE_CATEGORIES: &[&str] = &[
    "google",
    "facebook",
    "twitter",
    "amazon",
    "apple",
    "microsoft",
    "netflix",
    "spotify",
    "telegram",
    "youtube",
    "tiktok",
    "instagram",
    "whatsapp",
    "reddit",
    "github",
    "stackoverflow",
    "cn",
    "ru",
    "ir",
    "category-ads",
    "category-ads-all",
    "category-porn",
    "geolocation-cn",
    "geolocation-!cn",
    "tld-cn",
    "tld-ru",
];

pub fn validate_country_code(code: &str) -> Result<(), ValidationError> {
    if code.len() != 2 {
        return Err(ValidationError::InvalidCountryCode(code.to_string()));
    }

    if !code.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ValidationError::InvalidCountryCode(code.to_string()));
    }

    if !VALID_COUNTRY_CODES.contains(&code) {
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
        if !c.is_ascii_lowercase()
            && !c.is_ascii_digit()
            && c != '-'
            && c != '.'
            && c != '!'
        {
            return Err(ValidationError::InvalidGeoSiteCategory(
                category.to_string(),
            ));
        }
    }

    if !VALID_GEOSITE_CATEGORIES.contains(&category) {
        return Err(ValidationError::InvalidGeoSiteCategory(
            category.to_string(),
        ));
    }

    Ok(())
}

pub fn validate_rule_match(m: &RuleMatch) -> Result<(), ValidationError> {
    match m {
        RuleMatch::GeoIp { country_code } => validate_country_code(country_code),
        RuleMatch::GeoSite { category } => validate_geosite_category(category),
        RuleMatch::Domain { pattern } => validate_domain_pattern(pattern),
        RuleMatch::IpCidr { cidr } => validate_ip_cidr(&cidr.to_string()),
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
            ("us", false),
            ("USA", false),
            ("U", false),
            ("ZZ", false),
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
            ("Google", false),
            ("GOOGLE", false),
            ("unknown-category", false),
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
            RuleMatch::GeoSite {
                category: "google".to_string(),
            },
            RuleMatch::Domain {
                pattern: "example.com".to_string(),
            },
            RuleMatch::IpCidr {
                cidr: "192.168.1.0/24".parse().unwrap(),
            },
        ];

        for m in valid_cases {
            assert!(
                validate_rule_match(&m).is_ok(),
                "should be valid: {:?}",
                m
            );
        }

        let invalid_cases = vec![
            RuleMatch::GeoIp {
                country_code: "USA".to_string(),
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
