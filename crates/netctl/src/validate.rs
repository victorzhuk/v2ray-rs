use std::net::IpAddr;

/// Validates a TUN interface name: non-empty, at most 15 chars, restricted to
/// lowercase letters, digits, `_` and `-`. Mirrors the core model's rule so the
/// helper stays dependency-free.
pub fn validate_iface(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 15 {
        return Err(format!("invalid interface name: {name:?}"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(format!("invalid interface name: {name:?}"));
    }
    Ok(())
}

/// Parses and validates a CIDR string into its address and prefix length.
pub fn parse_cidr(cidr: &str) -> Result<(IpAddr, u8), String> {
    let (ip_str, prefix_str) = cidr
        .split_once('/')
        .ok_or_else(|| format!("invalid cidr (missing prefix): {cidr:?}"))?;
    let ip: IpAddr = ip_str
        .parse()
        .map_err(|_| format!("invalid cidr address: {cidr:?}"))?;
    let prefix: u8 = prefix_str
        .parse()
        .map_err(|_| format!("invalid cidr prefix: {cidr:?}"))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    if prefix > max {
        return Err(format!("cidr prefix out of range: {cidr:?}"));
    }
    Ok((ip, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn iface_rules() {
        for ok in ["tun0", "wg-tun_1", "a", "0123456789abcde"] {
            assert!(validate_iface(ok).is_ok(), "{ok}");
        }
        for bad in ["", "0123456789abcdef", "Tun0", "tun 0", "tun.0"] {
            assert!(validate_iface(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cidr_v4() {
        assert_eq!(
            parse_cidr("172.19.0.1/30").unwrap(),
            (IpAddr::V4(Ipv4Addr::new(172, 19, 0, 1)), 30)
        );
    }

    #[test]
    fn cidr_v6() {
        assert_eq!(
            parse_cidr("fd00::1/126").unwrap(),
            (IpAddr::V6("fd00::1".parse::<Ipv6Addr>().unwrap()), 126)
        );
    }

    #[test]
    fn cidr_rejects_bad() {
        for bad in ["1.2.3.4", "1.2.3.4/33", "::1/129", "garbage", "1.2.3.4/x"] {
            assert!(parse_cidr(bad).is_err(), "{bad}");
        }
    }
}
