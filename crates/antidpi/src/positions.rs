//! Split-position markers for TCP segmentation strategies.
//!
//! Mirrors zapret2's marker set (nfq2/protocol.c `PM_*`): resolved against
//! the reassembled outgoing message — HTTP request or TLS ClientHello.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    /// Absolute offset; negative counts from the end.
    Abs(isize),
    /// Start of the hostname (HTTP host or TLS SNI).
    Host,
    /// End of the hostname.
    HostEnd,
    /// Start of the second-level domain.
    Sld,
    /// Middle of the second-level domain.
    Midsld,
    /// End of the second-level domain.
    EndSld,
    /// Start of the SNI TLS extension.
    SniExt,
}

/// A parsed position marker: a named/absolute base plus a numeric suffix
/// offset (`midsld+1`, `host-2`, `-10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    base: Base,
    off: isize,
}

impl Marker {
    /// Parses a marker token: `1`, `-10`, `host`, `endhost`, `sld`, `midsld`,
    /// `endsld`, `sniext`, `method` (treated as `host` for TLS), each
    /// optionally followed by `+N`/`-N`.
    pub fn parse(tok: &str) -> Option<Marker> {
        let (name, off) = match tok.find(['+', '-']) {
            Some(i) if i > 0 => (&tok[..i], tok[i..].parse::<isize>().ok()?),
            _ => (tok, 0),
        };
        let base = match name {
            "host" | "method" => Base::Host,
            "endhost" => Base::HostEnd,
            "sld" => Base::Sld,
            "midsld" => Base::Midsld,
            "endsld" => Base::EndSld,
            "sniext" => Base::SniExt,
            _ => Base::Abs(tok.parse::<isize>().ok()?),
        };
        Some(Marker { base, off })
    }

    /// Resolves this marker against the reassembled payload.
    pub fn resolve(self, data: &[u8]) -> Option<usize> {
        let pos = match self.base {
            Base::Abs(v) => {
                if v < 0 {
                    data.len()
                        .checked_add_signed(v)
                        .map(|p| p as isize)
                        .unwrap_or(isize::MIN)
                } else {
                    v
                }
            }
            Base::Host | Base::HostEnd | Base::Sld | Base::Midsld | Base::EndSld => {
                let (h, l) = host_range(data)?;
                match self.base {
                    Base::Host => h as isize,
                    Base::HostEnd => (h + l) as isize,
                    Base::Sld => sld_start(data, h, l)? as isize,
                    Base::EndSld => {
                        let s = sld_start(data, h, l)?;
                        (s + sld_len(data, s)) as isize
                    }
                    Base::Midsld => {
                        let s = sld_start(data, h, l)?;
                        let sl = sld_len(data, s);
                        // zapret2 FindNLD: a single-char SLD shifts the mid
                        // by one (position of the following dot).
                        (if sl == 1 { s + 1 } else { s + sl / 2 }) as isize
                    }
                    _ => unreachable!(),
                }
            }
            Base::SniExt => sni_ext_start(data)? as isize,
        };
        let off = pos.checked_add(self.off)?;
        if off >= 0 && (off as usize) < data.len() {
            Some(off as usize)
        } else {
            None
        }
    }
}

/// Find the hostname inside a TLS ClientHello (SNI) or an HTTP request
/// (Host header). Returns (offset, len).
pub fn host_range(data: &[u8]) -> Option<(usize, usize)> {
    if crate::tls::looks_like_client_hello(data) {
        let l = crate::tls::parse_layout(data).ok()?;
        let (off, len) = (l.host_offset?, l.host_len);
        off.checked_add(len)
            .filter(|&end| end <= data.len())
            .map(|_| (off, len))
    } else {
        http_host_range(data)
    }
}

/// Plain HTTP request: Host header value range.
pub fn http_host_range(data: &[u8]) -> Option<(usize, usize)> {
    let head_end = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .and_then(|p| p.checked_add(4))?;
    let head = data.get(..head_end)?;
    let nl = head.iter().position(|&b| b == b'\n')?;
    let first = &head[..nl];
    let is_request = first.starts_with(b"GET ")
        || first.starts_with(b"POST ")
        || first.starts_with(b"HEAD ")
        || first.starts_with(b"PUT ")
        || first.starts_with(b"DELETE ")
        || first.starts_with(b"OPTIONS ");
    if !is_request {
        return None;
    }
    let mut cursor = nl + 1;
    while cursor < head.len() {
        let rel = head[cursor..].iter().position(|&b| b == b'\n')?;
        let line_end = cursor.checked_add(rel)?.checked_add(1)?;
        let mut line = &head[cursor..line_end];
        if line.last() == Some(&b'\n') {
            line = &line[..line.len() - 1];
        }
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if line.len() > 5 && line[..5].eq_ignore_ascii_case(b"host:") {
            let val = &line[5..];
            let start = val.iter().position(|b| *b != b' ' && *b != b'\t')?;
            let end = val.iter().rposition(|b| *b != b' ' && *b != b'\t')? + 1;
            let offset = cursor.checked_add(5)?.checked_add(start)?;
            return Some((offset, end - start));
        }
        cursor = line_end;
    }
    None
}

/// Start of the second-level domain (zapret2 `FindNLD` with nld=2): the text
/// between the second-to-last and the last dot; dotless hosts resolve to
/// nothing, single-dot hosts make the whole hostname prefix the SLD.
fn sld_start(data: &[u8], host_off: usize, host_len: usize) -> Option<usize> {
    let host = data.get(host_off..host_off.checked_add(host_len)?)?;
    let dots: Vec<usize> = host
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'.')
        .map(|(i, _)| i)
        .collect();
    if dots.is_empty() {
        return None;
    }
    if dots.len() >= 2 {
        Some(host_off + dots[dots.len() - 2] + 1)
    } else {
        Some(host_off)
    }
}

fn sld_len(data: &[u8], sld_off: usize) -> usize {
    let rest = data.get(sld_off..).unwrap_or(&[]);
    rest.iter().position(|b| *b == b'.').unwrap_or(rest.len())
}

/// Offset of the server_name_list length field inside the SNI extension
/// (zapret2 `PM_SNI_EXT` resolves to the extension start; the list-length
/// position is our stable stand-in).
fn sni_ext_start(data: &[u8]) -> Option<usize> {
    let l = crate::tls::parse_layout(data).ok()?;
    l.name_list_len_offset
}

/// Resolve a marker list into sorted, deduplicated positions (zapret2
/// `resolve_multi_pos` semantics).
pub fn resolve_multi(data: &[u8], markers: &[Marker]) -> Vec<usize> {
    let mut pos: Vec<usize> = markers.iter().filter_map(|m| m.resolve(data)).collect();
    pos.sort_unstable();
    pos.dedup();
    // zapret2: positions 0 and 1 are never valid split points.
    pos.retain(|p| *p != 0 && *p != 1);
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::{craft_client_hello, rand_bytes::Counter};

    #[test]
    fn resolve_multi_drops_position_zero() {
        let data = b"0123456789";
        let markers = vec![
            Marker::parse("0").unwrap(),
            Marker::parse("1").unwrap(),
            Marker::parse("5").unwrap(),
        ];
        assert_eq!(resolve_multi(data, &markers), vec![5]);
    }

    #[test]
    fn abs_markers() {
        let data = b"0123456789";
        assert_eq!(Marker::parse("2").unwrap().resolve(data), Some(2));
        assert_eq!(Marker::parse("-2").unwrap().resolve(data), Some(8));
        assert_eq!(Marker::parse("99").unwrap().resolve(data), None);
        assert_eq!(Marker::parse("abc"), None);
    }

    #[test]
    fn tls_host_markers() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let host = b"www.example.com";
        let hoff = ch.windows(host.len()).position(|w| w == host).unwrap();
        assert_eq!(Marker::parse("host").unwrap().resolve(&ch), Some(hoff));
        assert_eq!(
            Marker::parse("endhost").unwrap().resolve(&ch),
            Some(hoff + host.len())
        );
        let sld = hoff + 4; // after "www."
        assert_eq!(Marker::parse("sld").unwrap().resolve(&ch), Some(sld));
        // SLD is "example" (7 chars): mid at +3, end at +7.
        assert_eq!(
            Marker::parse("midsld").unwrap().resolve(&ch),
            Some(sld + 7 / 2)
        );
        assert_eq!(Marker::parse("endsld").unwrap().resolve(&ch), Some(sld + 7));
        assert_eq!(
            Marker::parse("sniext").unwrap().resolve(&ch),
            Marker {
                base: Base::SniExt,
                off: 0
            }
            .resolve(&ch)
        );
        assert_eq!(
            Marker::parse("midsld+2").unwrap().resolve(&ch),
            Some(sld + 7 / 2 + 2)
        );
    }

    #[test]
    fn single_dot_host_sld_is_whole_host() {
        let ch = craft_client_hello("example.com", &mut Counter(0)).expect("crafted client hello");
        let hoff = ch
            .windows(b"example.com".len())
            .position(|w| w == b"example.com")
            .unwrap();
        assert_eq!(Marker::parse("sld").unwrap().resolve(&ch), Some(hoff));
        // SLD "example" (7 chars, up to the last dot).
        assert_eq!(
            Marker::parse("midsld").unwrap().resolve(&ch),
            Some(hoff + 7 / 2)
        );
        assert_eq!(
            Marker::parse("endsld").unwrap().resolve(&ch),
            Some(hoff + 7)
        );
    }

    #[test]
    fn dotless_host_has_no_sld_marker() {
        let ch = craft_client_hello("localhost", &mut Counter(0)).expect("crafted client hello");
        assert_eq!(Marker::parse("sld").unwrap().resolve(&ch), None);
        assert_eq!(Marker::parse("midsld").unwrap().resolve(&ch), None);
    }

    #[test]
    fn resolve_multi_dedup_and_no_pos1() {
        let ch = craft_client_hello("a.com", &mut Counter(0)).expect("crafted client hello");
        let pos = resolve_multi(
            &ch,
            &[Marker::parse("1").unwrap(), Marker::parse("5").unwrap()],
        );
        assert_eq!(pos, vec![5]);
    }

    #[test]
    fn http_host_range_finds_header() {
        let req = b"GET /x HTTP/1.1\r\nHost: example.org\r\nUser-Agent: t\r\n\r\n";
        let (off, len) = http_host_range(req).unwrap();
        assert_eq!(&req[off..off + len], b"example.org");
        assert_eq!(Marker::parse("host").unwrap().resolve(req), Some(off));
        assert_eq!(
            Marker::parse("endhost").unwrap().resolve(req),
            Some(off + len)
        );
    }

    #[test]
    fn http_host_range_rejects_responses() {
        assert_eq!(http_host_range(b"HTTP/1.1 200 OK\r\nHost: x\r\n\r\n"), None);
    }
}
