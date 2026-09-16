use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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

/// Reports whether `iface` is a TUN/TAP device. Only such devices expose the
/// `tun_flags` sysfs attribute — `lo`, physical NICs, bridges, WireGuard and
/// veth do not. Used to refuse operating on interfaces this helper did not
/// create, so an unprivileged caller can't black-hole `lo` or delete another
/// tunnel's link.
pub fn is_tun_device(iface: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{iface}/tun_flags")).exists()
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

/// Same cap the app enforces on `exclude_routes` (`MAX_EXCLUDE_ROUTES` in
/// `crates/core/src/models/tun.rs`); the helper re-checks it because any local
/// process can invoke it directly.
pub const MAX_EXCLUSIONS: usize = 256;

/// Parses a route exclusion CIDR, refusing prefix 0 and clearing host bits.
pub fn parse_exclusion(cidr: &str) -> Result<(IpAddr, u8), String> {
    let (ip, prefix) = parse_cidr(cidr)?;
    if prefix == 0 {
        return Err(format!("exclusion prefix length 0 refused: {cidr:?}"));
    }
    let network = match ip {
        IpAddr::V4(v4) => Ipv4Addr::from(u32::from(v4) & (u32::MAX << (32 - prefix))).into(),
        IpAddr::V6(v6) => Ipv6Addr::from(u128::from(v6) & (u128::MAX << (128 - prefix))).into(),
    };
    Ok((network, prefix))
}

pub fn check_exclusion_count(count: usize) -> Result<(), String> {
    if count > MAX_EXCLUSIONS {
        return Err(format!(
            "too many exclusions: {count} (max {MAX_EXCLUSIONS})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn rejects_non_tun_interfaces() {
        // lo always exists but is not a TUN device; a missing name is also false.
        assert!(!is_tun_device("lo"));
        assert!(!is_tun_device("nonexistent-iface-xyz"));
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

    #[test]
    fn exclusion_accepts_host_route() {
        assert_eq!(
            parse_exclusion("10.15.12.100/32").unwrap(),
            (IpAddr::V4(Ipv4Addr::new(10, 15, 12, 100)), 32)
        );
    }

    #[test]
    fn exclusion_clears_host_bits() {
        assert_eq!(
            parse_exclusion("10.1.2.3/8").unwrap(),
            (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8)
        );
        assert_eq!(
            parse_exclusion("fd00::1/64").unwrap(),
            (IpAddr::V6("fd00::".parse::<Ipv6Addr>().unwrap()), 64)
        );
    }

    #[test]
    fn exclusion_refuses_default_prefix() {
        for bad in ["0.0.0.0/0", "::/0", "10.0.0.1/0"] {
            let err = parse_exclusion(bad).unwrap_err();
            assert!(err.contains(&format!("{bad:?}")), "{err}");
        }
    }

    #[test]
    fn exclusion_refuses_out_of_range_prefix() {
        for bad in ["1.2.3.4/33", "::1/129", "garbage", "1.2.3.4", "1.2.3.4/x"] {
            let err = parse_exclusion(bad).unwrap_err();
            assert!(err.contains(&format!("{bad:?}")), "{err}");
        }
    }

    #[test]
    fn exclusion_count_capped_at_256() {
        assert!(check_exclusion_count(0).is_ok());
        assert!(check_exclusion_count(256).is_ok());
        assert!(check_exclusion_count(257).unwrap_err().contains("257"));
    }
}
