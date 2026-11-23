//! IPv4/TCP/UDP packet parse and build, hand-rolled.
//!
//! NFQUEUE payloads are L3 packets (no Ethernet header). We only need the
//! IPv4+TCP and IPv4+UDP shapes; parsing mirrors the offsets we emit when
//! building, so round-trips are exact. Checksums follow RFC 1071 with the
//! standard TCP/UDP pseudo-header.

use crate::error::{Error, Result};

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_PSH: u8 = 0x08;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const TCP_FLAG_URG: u8 = 0x20;

/// Parsed view of an NFQUEUE payload.
#[derive(Debug)]
pub struct Packet<'a> {
    pub src: [u8; 4],
    pub dst: [u8; 4],
    pub ttl: u8,
    pub ip_id: u16,
    pub tos: u8,
    pub dont_fragment: bool,
    pub tcp: Option<TcpView<'a>>,
    pub udp: Option<UdpView<'a>>,
}

#[derive(Debug)]
pub struct TcpView<'a> {
    pub sport: u16,
    pub dport: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    /// Raw TCP option bytes (between data offset and payload).
    pub options: &'a [u8],
    pub payload: &'a [u8],
}

#[derive(Debug)]
pub struct UdpView<'a> {
    pub sport: u16,
    pub dport: u16,
    pub payload: &'a [u8],
}

impl<'a> Packet<'a> {
    /// Parses an IPv4/TCP or IPv4/UDP packet. Trailing bytes beyond the IP
    /// total length are ignored; fragments and out-of-range length fields
    /// are rejected.
    pub fn parse(bytes: &'a [u8]) -> Result<Packet<'a>> {
        if bytes.len() < 20 {
            return Err(Error::Packet("ip header truncated".into()));
        }
        if bytes[0] >> 4 != 4 {
            return Err(Error::Packet("not ipv4".into()));
        }
        let ihl = usize::from(bytes[0] & 0x0F) * 4;
        if ihl < 20 || bytes.len() < ihl {
            return Err(Error::Packet("ip ihl out of range".into()));
        }
        let total_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if !(ihl..=bytes.len()).contains(&total_len) {
            return Err(Error::Packet("ip total length out of range".into()));
        }
        let flags_frag = u16::from_be_bytes([bytes[6], bytes[7]]);
        if flags_frag & 0x3FFF != 0 {
            return Err(Error::Packet("fragmented ipv4 packet".into()));
        }
        let proto = bytes[9];
        let mut packet = Packet {
            src: [bytes[12], bytes[13], bytes[14], bytes[15]],
            dst: [bytes[16], bytes[17], bytes[18], bytes[19]],
            ttl: bytes[8],
            ip_id: u16::from_be_bytes([bytes[4], bytes[5]]),
            tos: bytes[1],
            dont_fragment: bytes[6] & 0x40 != 0,
            tcp: None,
            udp: None,
        };
        let rest = &bytes[ihl..total_len];
        match proto {
            6 => {
                if rest.len() < 20 {
                    return Err(Error::Packet("tcp header truncated".into()));
                }
                let data_offset = usize::from(rest[12] >> 4) * 4;
                if data_offset < 20 || rest.len() < data_offset {
                    return Err(Error::Packet("tcp data offset out of range".into()));
                }
                packet.tcp = Some(TcpView {
                    sport: u16::from_be_bytes([rest[0], rest[1]]),
                    dport: u16::from_be_bytes([rest[2], rest[3]]),
                    seq: u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]),
                    ack: u32::from_be_bytes([rest[8], rest[9], rest[10], rest[11]]),
                    flags: rest[13],
                    window: u16::from_be_bytes([rest[14], rest[15]]),
                    options: &rest[20..data_offset],
                    payload: &rest[data_offset..],
                });
            }
            17 => {
                if rest.len() < 8 {
                    return Err(Error::Packet("udp header truncated".into()));
                }
                let udp_len = u16::from_be_bytes([rest[4], rest[5]]) as usize;
                if !(8..=rest.len()).contains(&udp_len) {
                    return Err(Error::Packet("udp length out of range".into()));
                }
                packet.udp = Some(UdpView {
                    sport: u16::from_be_bytes([rest[0], rest[1]]),
                    dport: u16::from_be_bytes([rest[2], rest[3]]),
                    payload: &rest[8..udp_len],
                });
            }
            _ => return Err(Error::Packet("unsupported protocol".into())),
        }
        Ok(packet)
    }

    pub fn is_syn(&self) -> bool {
        self.tcp
            .as_ref()
            .is_some_and(|t| t.flags & (TCP_FLAG_SYN | TCP_FLAG_ACK) == TCP_FLAG_SYN)
    }
}

/// A fully built L3 packet ready for raw-send.
#[derive(Debug, Clone)]
pub struct OutPacket {
    pub bytes: Vec<u8>,
}

impl OutPacket {
    /// Wraps pre-built wire bytes (checksum assumed correct).
    pub fn from_bytes(bytes: Vec<u8>) -> OutPacket {
        OutPacket { bytes }
    }
}

/// Kind of transport the builder emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Tcp,
    Udp,
}

/// Builder collecting fields, then `build()` produces the wire packet with
/// correct checksums. Rejects TCP options over 40 bytes and packets larger
/// than the IPv4 limit.
pub struct OutBuilder {
    src: [u8; 4],
    dst: [u8; 4],
    sport: u16,
    dport: u16,
    transport: Transport,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    options: Vec<u8>,
    payload: Vec<u8>,
    ttl: u8,
    ip_id: u16,
    tos: u8,
    dont_fragment: bool,
    badsum: bool,
}

impl OutBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn tcp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> Self {
        Self {
            src,
            dst,
            sport,
            dport,
            transport: Transport::Tcp,
            seq: 0,
            ack: 0,
            flags: TCP_FLAG_ACK | TCP_FLAG_PSH,
            window: 65535,
            options: Vec::new(),
            payload: Vec::new(),
            ttl: 64,
            ip_id: 0,
            tos: 0,
            dont_fragment: true,
            badsum: false,
        }
    }

    pub fn udp(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> Self {
        Self {
            transport: Transport::Udp,
            flags: 0,
            window: 0,
            seq: 0,
            ack: 0,
            options: Vec::new(),
            badsum: false,
            src,
            dst,
            sport,
            dport,
            payload: Vec::new(),
            ttl: 64,
            ip_id: 0,
            tos: 0,
            dont_fragment: true,
        }
    }

    pub fn seq(mut self, v: u32) -> Self {
        self.seq = v;
        self
    }
    pub fn ack(mut self, v: u32) -> Self {
        self.ack = v;
        self
    }
    pub fn flags(mut self, v: u8) -> Self {
        self.flags = v;
        self
    }
    pub fn window(mut self, v: u16) -> Self {
        self.window = v;
        self
    }
    pub fn options(mut self, v: &[u8]) -> Self {
        self.options = v.to_vec();
        self
    }
    pub fn payload(mut self, v: impl Into<Vec<u8>>) -> Self {
        self.payload = v.into();
        self
    }
    pub fn ttl(mut self, v: u8) -> Self {
        self.ttl = v;
        self
    }
    pub fn ip_id(mut self, v: u16) -> Self {
        self.ip_id = v;
        self
    }
    pub fn tos(mut self, v: u8) -> Self {
        self.tos = v;
        self
    }
    pub fn dont_fragment(mut self, v: bool) -> Self {
        self.dont_fragment = v;
        self
    }
    pub fn badsum(mut self, v: bool) -> Self {
        self.badsum = v;
        self
    }

    pub fn build(self) -> Result<OutPacket> {
        match self.transport {
            Transport::Tcp => self.build_tcp(),
            Transport::Udp => self.build_udp(),
        }
    }

    fn build_tcp(self) -> Result<OutPacket> {
        // Options must be 4-byte aligned; pad with NOPs when they are not.
        let mut options = self.options.clone();
        if !options.len().is_multiple_of(4) {
            while !options.len().is_multiple_of(4) {
                options.push(0x01); // NOP
            }
        }
        if options.len() > 40 {
            return Err(Error::Packet("tcp options exceed 40 bytes".into()));
        }
        let header_len = 20 + options.len();
        let tcp_len = header_len + self.payload.len();
        let total = 20 + tcp_len;
        if total > u16::MAX as usize {
            return Err(Error::Packet("packet exceeds 65535 bytes".into()));
        }

        let mut b = Vec::with_capacity(total);
        // IP flags+fragment-offset field: only DF, never a fragment offset.
        let mut flags_frag: u16 = 0;
        if self.dont_fragment {
            flags_frag |= 0x4000;
        }
        b.extend_from_slice(&[
            0x45, // IPv4, IHL=5: no IP options in generated packets
            self.tos,
            (total >> 8) as u8,
            total as u8,
            (self.ip_id >> 8) as u8,
            self.ip_id as u8,
            (flags_frag >> 8) as u8,
            flags_frag as u8,
            self.ttl,
            6,
            0,
            0,
        ]);
        b.extend_from_slice(&self.src);
        b.extend_from_slice(&self.dst);
        b.extend_from_slice(&self.sport.to_be_bytes());
        b.extend_from_slice(&self.dport.to_be_bytes());
        b.extend_from_slice(&self.seq.to_be_bytes());
        b.extend_from_slice(&self.ack.to_be_bytes());
        b.push(((header_len / 4) as u8) << 4);
        b.push(self.flags);
        b.extend_from_slice(&self.window.to_be_bytes());
        let checksum_offset = b.len();
        b.extend_from_slice(&[0, 0, 0, 0]); // checksum + urgent pointer
        b.extend_from_slice(&options);
        b.extend_from_slice(&self.payload);

        let mut sum = pseudo_sum(self.src, self.dst, 6, tcp_len as u32);
        sum = add_bytes(sum, &b[20..]);
        let csum = fold(sum);
        b[checksum_offset] = (csum >> 8) as u8;
        b[checksum_offset + 1] = csum as u8;

        let isum = add_bytes(0, &b[..20]);
        let ic = fold(isum);
        b[10] = (ic >> 8) as u8;
        b[11] = ic as u8;

        if self.badsum {
            // zapret2 semantics: computed sum XOR-corrupted, well-formed but
            // never matching.
            let c = u16::from_be_bytes([b[checksum_offset], b[checksum_offset + 1]]) ^ 0x1234;
            b[checksum_offset] = (c >> 8) as u8;
            b[checksum_offset + 1] = c as u8;
        }

        Ok(OutPacket { bytes: b })
    }

    fn build_udp(self) -> Result<OutPacket> {
        let udp_len = 8 + self.payload.len();
        let total = 20 + udp_len;
        if total > u16::MAX as usize {
            return Err(Error::Packet("packet exceeds 65535 bytes".into()));
        }
        let mut b = Vec::with_capacity(total);
        // IP flags+fragment-offset field: only DF, never a fragment offset.
        let mut flags_frag: u16 = 0;
        if self.dont_fragment {
            flags_frag |= 0x4000;
        }
        b.extend_from_slice(&[
            0x40 | 5,
            self.tos,
            (total >> 8) as u8,
            total as u8,
            (self.ip_id >> 8) as u8,
            self.ip_id as u8,
            (flags_frag >> 8) as u8,
            flags_frag as u8,
            self.ttl,
            17,
            0,
            0,
        ]);
        b.extend_from_slice(&self.src);
        b.extend_from_slice(&self.dst);
        b.extend_from_slice(&self.sport.to_be_bytes());
        b.extend_from_slice(&self.dport.to_be_bytes());
        b.extend_from_slice(&(udp_len as u16).to_be_bytes());
        let checksum_offset = b.len();
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&self.payload);

        let mut sum = pseudo_sum(self.src, self.dst, 17, udp_len as u32);
        sum = add_bytes(sum, &b[20..]);
        let csum = fold(sum);
        // A computed zero is transmitted as 0xFFFF per RFC 768.
        let csum = if csum == 0 { 0xFFFF } else { csum };
        b[checksum_offset] = (csum >> 8) as u8;
        b[checksum_offset + 1] = csum as u8;
        if self.badsum {
            // zapret2 semantics: computed sum XOR-corrupted; a zero result
            // would disable verification in IPv4, so it maps to 0xFFFF.
            let c = match u16::from_be_bytes([b[checksum_offset], b[checksum_offset + 1]]) ^ 0x1234
            {
                0 => 0xFFFF,
                c => c,
            };
            b[checksum_offset] = (c >> 8) as u8;
            b[checksum_offset + 1] = c as u8;
        }

        let isum = add_bytes(0, &b[..20]);
        let ic = fold(isum);
        b[10] = (ic >> 8) as u8;
        b[11] = ic as u8;

        Ok(OutPacket { bytes: b })
    }
}

fn pseudo_sum(src: [u8; 4], dst: [u8; 4], proto: u8, len: u32) -> u32 {
    let mut s = add_bytes(0, &src);
    s = add_bytes(s, &dst);
    s = add_bytes(s, &[0, proto]);
    // Length field in network byte order (hi byte first).
    add_bytes(s, &[(len >> 8) as u8, len as u8, 0, 0])
}

fn add_bytes(mut sum: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i + 2 <= data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}

fn fold(sum: u32) -> u16 {
    let mut s = (sum >> 16) + (sum & 0xFFFF);
    s = (s >> 16) + (s & 0xFFFF);
    !s as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: [u8; 4] = [192, 168, 7, 2];
    const DST: [u8; 4] = [192, 168, 7, 1];

    #[test]
    fn tcp_roundtrip() {
        let out = OutBuilder::tcp(SRC, DST, 12345, 443)
            .seq(0x11223344)
            .ack(0x55667788)
            .flags(TCP_FLAG_PSH | TCP_FLAG_ACK)
            .window(32100)
            .payload(b"hello world")
            .ttl(64)
            .ip_id(0xABCD)
            .build()
            .expect("valid packet");
        let p = Packet::parse(&out.bytes).unwrap();
        let t = p.tcp.unwrap();
        assert_eq!(p.src, SRC);
        assert_eq!(p.dst, DST);
        assert_eq!(p.ttl, 64);
        assert_eq!(p.ip_id, 0xABCD);
        assert!(p.dont_fragment);
        assert_eq!(t.sport, 12345);
        assert_eq!(t.dport, 443);
        assert_eq!(t.seq, 0x11223344);
        assert_eq!(t.ack, 0x55667788);
        assert_eq!(t.flags, TCP_FLAG_PSH | TCP_FLAG_ACK);
        assert_eq!(t.window, 32100);
        assert_eq!(t.payload, b"hello world");
        assert!(t.options.is_empty());
    }

    #[test]
    fn tcp_checksum_matches_reference() {
        // Independent RFC 1071 implementation: word-at-a-time over the
        // pseudo-header and the TCP segment with a zeroed checksum field.
        const PAYLOAD_LEN: usize = 64;
        let tcp_len = 20 + PAYLOAD_LEN;
        let out = OutBuilder::tcp(SRC, DST, 5000, 443)
            .ack(1)
            .window(65535)
            .payload(vec![0u8; PAYLOAD_LEN])
            .ttl(128)
            .build()
            .expect("valid packet");

        let word = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]) as u32;
        let mut sum: u32 = 0;
        // pseudo-header: src (2 words), dst (2 words), zero+proto, length
        sum += word(&SRC[0..2]) + word(&SRC[2..4]);
        sum += word(&DST[0..2]) + word(&DST[2..4]);
        sum += word(&[0, 6]);
        sum += word(&[(tcp_len >> 8) as u8, tcp_len as u8]);
        // TCP header with zeroed checksum (checksum field at offset 16)
        let mut seg = out.bytes[20..].to_vec();
        seg[16] = 0;
        seg[17] = 0;
        let mut i = 0;
        while i + 2 <= seg.len() {
            sum += word(&seg[i..i + 2]);
            i += 2;
        }
        assert_eq!(i, seg.len(), "odd-length segment not expected here");
        while sum >> 16 != 0 {
            sum = (sum >> 16) + (sum & 0xFFFF);
        }
        let want = !(sum as u16);
        // TCP checksum lives at IP header (20) + TCP offset 16.
        const CSUM_OFF: usize = 36;
        let got = u16::from_be_bytes([out.bytes[CSUM_OFF], out.bytes[CSUM_OFF + 1]]);
        assert_eq!(got, want);
    }

    #[test]
    fn tcp_options_roundtrip_and_pad() {
        let out = OutBuilder::tcp(SRC, DST, 1, 2)
            .flags(TCP_FLAG_SYN)
            .options(&[0x02, 0x04, 0x05, 0xB4, 0x01, 0x01])
            .build()
            .expect("valid packet"); // 6 bytes: must be padded to 8
        let t = Packet::parse(&out.bytes).unwrap().tcp.unwrap();
        assert_eq!(t.options, [0x02, 0x04, 0x05, 0xB4, 0x01, 0x01, 0x01, 0x01]);
    }

    #[test]
    fn udp_roundtrip() {
        let out = OutBuilder::udp(SRC, DST, 50000, 443)
            .payload(b"quic")
            .ttl(64)
            .ip_id(7)
            .build()
            .expect("valid packet");
        let p = Packet::parse(&out.bytes).unwrap();
        let u = p.udp.unwrap();
        assert_eq!(u.dport, 443);
        assert_eq!(u.payload, b"quic");
    }

    #[test]
    fn badsum_corrupts() {
        const CSUM_OFF: usize = 36;
        let good = OutBuilder::tcp(SRC, DST, 1, 2)
            .payload(b"x")
            .build()
            .expect("valid packet");
        let bad = OutBuilder::tcp(SRC, DST, 1, 2)
            .payload(b"x")
            .badsum(true)
            .build()
            .expect("valid packet");
        assert_ne!(
            &good.bytes[CSUM_OFF..CSUM_OFF + 2],
            &bad.bytes[CSUM_OFF..CSUM_OFF + 2]
        );
        // Corruption keeps the rest byte-identical.
        let mut a = good.bytes.clone();
        let mut c = bad.bytes.clone();
        a[CSUM_OFF] = 0;
        a[CSUM_OFF + 1] = 0;
        c[CSUM_OFF] = 0;
        c[CSUM_OFF + 1] = 0;
        assert_eq!(a, c);
    }

    #[test]
    fn odd_length_payload_checksum() {
        let out = OutBuilder::tcp(SRC, DST, 1, 2)
            .payload(b"odd!")
            .build()
            .expect("valid packet");
        let _ = Packet::parse(&out.bytes).unwrap();
        let out2 = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"od")
            .build()
            .expect("valid packet");
        let _ = Packet::parse(&out2.bytes).unwrap();
    }

    #[test]
    fn udp_badsum_is_nonzero_and_invalid() {
        const CSUM_OFF: usize = 26; // IP header (20) + UDP offset (6)
        let good = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"quic")
            .build()
            .expect("valid packet");
        let bad = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"quic")
            .badsum(true)
            .build()
            .expect("valid packet");
        // Independent RFC 1071 reference over the UDP datagram with a
        // zeroed checksum field.
        let word = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]) as u32;
        let mut sum = 0u32;
        sum += word(&SRC[0..2]) + word(&SRC[2..4]);
        sum += word(&DST[0..2]) + word(&DST[2..4]);
        sum += word(&[0, 17]);
        let udp_len = good.bytes.len() - 20;
        sum += word(&[(udp_len >> 8) as u8, udp_len as u8]);
        let mut seg = good.bytes[20..].to_vec();
        seg[6] = 0;
        seg[7] = 0;
        let mut i = 0;
        while i + 2 <= seg.len() {
            sum += word(&seg[i..i + 2]);
            i += 2;
        }
        while sum >> 16 != 0 {
            sum = (sum >> 16) + (sum & 0xFFFF);
        }
        let reference = !(sum as u16);
        let emitted = u16::from_be_bytes([bad.bytes[CSUM_OFF], bad.bytes[CSUM_OFF + 1]]);
        assert_ne!(emitted, reference);
        assert_ne!(emitted, 0);
    }

    #[test]
    fn tcp_options_over_40_bytes_are_rejected() {
        for len in [41usize, 44] {
            let opts = vec![0x01u8; len];
            assert!(
                OutBuilder::tcp(SRC, DST, 1, 2)
                    .options(&opts)
                    .build()
                    .is_err(),
                "{len} option bytes must be rejected"
            );
        }
        let opts = vec![0x01u8; 40];
        assert!(
            OutBuilder::tcp(SRC, DST, 1, 2)
                .options(&opts)
                .build()
                .is_ok()
        );
    }

    #[test]
    fn oversized_payloads_are_rejected() {
        assert!(
            OutBuilder::tcp(SRC, DST, 1, 2)
                .payload(vec![0u8; 65_495])
                .build()
                .is_ok()
        );
        assert!(
            OutBuilder::tcp(SRC, DST, 1, 2)
                .payload(vec![0u8; 65_516])
                .build()
                .is_err()
        );
        assert!(
            OutBuilder::udp(SRC, DST, 1, 2)
                .payload(vec![0u8; 65_507])
                .build()
                .is_ok()
        );
        assert!(
            OutBuilder::udp(SRC, DST, 1, 2)
                .payload(vec![0u8; 65_528])
                .build()
                .is_err()
        );
    }

    #[test]
    fn ipv4_total_length_bounds_the_payload() {
        let mut bytes = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"abcd")
            .build()
            .expect("valid packet")
            .bytes;
        bytes.extend_from_slice(b"tail");
        let p = Packet::parse(&bytes).unwrap();
        assert_eq!(p.udp.unwrap().payload, b"abcd");

        // Total length below the IP header.
        let mut short = bytes.clone();
        short[2..4].copy_from_slice(&16u16.to_be_bytes());
        assert!(Packet::parse(&short).is_err());

        // Total length beyond the buffer.
        let len = bytes.len();
        let mut long = bytes;
        long[2..4].copy_from_slice(&(len as u16 + 1).to_be_bytes());
        assert!(Packet::parse(&long).is_err());
    }

    #[test]
    fn fragmented_ipv4_is_rejected() {
        let mut bytes = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"abcd")
            .build()
            .expect("valid packet")
            .bytes;
        // Fragment offset 8 (8-byte units) in the low bits.
        bytes[6..8].copy_from_slice(&(8u16).to_be_bytes());
        assert!(Packet::parse(&bytes).is_err());
        // More-fragments bit.
        bytes[6..8].copy_from_slice(&(0x2000u16).to_be_bytes());
        assert!(Packet::parse(&bytes).is_err());
    }

    #[test]
    fn udp_length_is_enforced() {
        let mut bytes = OutBuilder::udp(SRC, DST, 1, 2)
            .payload(b"abcd")
            .build()
            .expect("valid packet")
            .bytes;
        // Below the UDP header size.
        bytes[24..26].copy_from_slice(&7u16.to_be_bytes());
        assert!(Packet::parse(&bytes).is_err());
        // Beyond the remaining bytes.
        bytes[24..26].copy_from_slice(&64u16.to_be_bytes());
        assert!(Packet::parse(&bytes).is_err());
    }
}
