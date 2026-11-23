//! Decoy-fooling parameters (TTL, seq/ack shifts, badsum, TCP MD5 option).
//!
//! Semantics mirror zapret2 `apply_fooling` (lua/zapret-lib.lua), including
//! the `autottl` hop-count guessing algorithm.

use crate::packet::{OutBuilder, Packet};

/// `delta,min-max` parsed from the strategy string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoTtl {
    pub delta: isize,
    pub min: u8,
    pub max: u8,
}

impl AutoTtl {
    /// Parses `delta,min-max`, e.g. `-1,3-30`. A third token is rejected.
    pub fn parse(s: &str) -> Option<AutoTtl> {
        let mut it = s.split(',');
        let delta = it.next()?.parse::<isize>().ok()?;
        let range = it.next()?;
        let (min, max) = range.split_once('-')?;
        let (min, max) = (min.parse::<u8>().ok()?, max.parse::<u8>().ok()?);
        if min > max || it.next().is_some() {
            return None;
        }
        Some(AutoTtl { delta, min, max })
    }
}

/// Guess the fake TTL so the decoy dies at the DPI hop but would have
/// reached the server (zapret2 `autottl`). `incoming_ttl` is the TTL observed
/// on the reply direction; `0` means discovery failed and fixed `ip_ttl`
/// should be used instead.
pub fn autottl(incoming_ttl: u8, attl: &AutoTtl) -> Option<u8> {
    fn hop_count_guess(incoming_ttl: u8) -> Option<u8> {
        // Guess the original TTL by class: cloudfront 255, common 128, local 64.
        if incoming_ttl > 223 {
            Some(255 - incoming_ttl)
        } else if (97..=128).contains(&incoming_ttl) {
            Some(128 - incoming_ttl)
        } else if (33..=64).contains(&incoming_ttl) {
            Some(64 - incoming_ttl)
        } else {
            None
        }
    }
    let hops = hop_count_guess(incoming_ttl)?;
    let d = (hops as isize).saturating_add(attl.delta);
    let fake = d.clamp(attl.min as isize, attl.max as isize) as u8;
    if (attl.delta < 0 && fake >= hops) || (attl.delta >= 0 && fake < hops) {
        return None;
    }
    Some(fake)
}

/// Fooling parameters for a generated packet.
#[derive(Debug, Clone)]
pub struct Fooling {
    pub ttl: Option<u8>,
    pub autottl: Option<AutoTtl>,
    pub badsum: bool,
    /// Additive seq shift (zapret2 `tcp_seq`, nfqws1 badseq).
    pub seq_off: i32,
    /// Additive ack shift (`tcp_ack`).
    pub ack_off: i32,
    /// Append a TCP MD5-signature option (TSPU cannot validate it; Linux
    /// hosts without the shared secret ignore it).
    pub tcp_md5: bool,
    /// Flags cleared from the generated packet (`tcp_flags_unset=ack`).
    pub flags_unset: u8,
    /// Repeat the decoy N times total.
    pub repeats: usize,
}

impl Default for Fooling {
    fn default() -> Self {
        Self {
            ttl: None,
            autottl: None,
            badsum: false,
            seq_off: 0,
            ack_off: 0,
            tcp_md5: false,
            flags_unset: 0,
            repeats: 1,
        }
    }
}

impl Fooling {
    /// Resolve the effective TTL: autottl (from the observed reply TTL) wins
    /// over the fixed value, mirroring `ttl_discover` in zapret2.
    pub fn resolve_ttl(&self, reply_ttl: Option<u8>) -> u8 {
        if let (Some(attl), Some(replies)) = (self.autottl, reply_ttl)
            && let Some(t) = autottl(replies, &attl)
        {
            return t;
        }
        self.ttl.unwrap_or(64)
    }
}

/// Applies fooling to an `OutBuilder` for a fake derived from the original
/// queued packet.
pub fn apply_fooling_tcp(
    b: OutBuilder,
    f: &Fooling,
    orig: &Packet,
    reply_ttl: Option<u8>,
    ip_id: u16,
) -> OutBuilder {
    let Some(t) = &orig.tcp else { return b };
    let mut b = b
        .seq(t.seq.wrapping_add_signed(f.seq_off))
        .ack(t.ack.wrapping_add_signed(f.ack_off))
        .flags(t.flags & !f.flags_unset)
        .window(t.window)
        .options(t.options)
        .ttl(f.resolve_ttl(reply_ttl))
        .ip_id(ip_id)
        .badsum(f.badsum);
    if f.tcp_md5 {
        // The MD5 option needs 18 bytes; a full options block keeps the
        // signature only, replacing the original options.
        if t.options.len() + 18 > 40 {
            b = b.options(&md5_option());
        } else {
            let mut opts = t.options.to_vec();
            opts.extend_from_slice(&md5_option());
            b = b.options(&opts);
        }
    }
    b
}

/// TCP MD5 signature option (kind 19, RFC 2385): 16 random bytes. The
/// checksum is recomputed with the option in place.
pub fn md5_option() -> Vec<u8> {
    use crate::tls::rand_bytes::Rng;
    let mut v = Vec::with_capacity(18);
    // Kind 19, length 18.
    v.push(19);
    v.push(18);
    v.extend_from_slice(&crate::tls::rand_bytes::OsRng.bytes(16));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autottl_parse() {
        assert_eq!(
            AutoTtl::parse("-1,3-30"),
            Some(AutoTtl {
                delta: -1,
                min: 3,
                max: 30
            })
        );
        assert_eq!(AutoTtl::parse("bad"), None);
        assert_eq!(AutoTtl::parse("5,30-3"), None);
    }

    #[test]
    fn autottl_hops_from_cloudfront_255() {
        // incoming 250 → hops 5; delta -1 → fake 4 (< 5 OK, dies 4 hops out,
        // after passing the DPI at hop ~2).
        assert_eq!(
            autottl(
                250,
                &AutoTtl {
                    delta: -1,
                    min: 3,
                    max: 30
                }
            ),
            Some(4)
        );
        // delta +10 → fake 15 ≥ hops 5: still valid (out-of-window decoys die
        // at the server anyway); nfqws only rejects when a negative delta
        // would leave the fake reaching the server.
        assert_eq!(
            autottl(
                250,
                &AutoTtl {
                    delta: 10,
                    min: 3,
                    max: 30
                }
            ),
            Some(15)
        );
    }

    #[test]
    fn autottl_hops_from_128() {
        assert_eq!(
            autottl(
                120,
                &AutoTtl {
                    delta: -1,
                    min: 3,
                    max: 30
                }
            ),
            Some(7)
        );
    }

    #[test]
    fn autottl_unknown_base_class() {
        assert_eq!(
            autottl(
                80,
                &AutoTtl {
                    delta: -1,
                    min: 3,
                    max: 30
                }
            ),
            None
        );
    }

    #[test]
    fn fake_without_repeats_sends_once() {
        assert_eq!(Fooling::default().repeats, 1);
    }

    #[test]
    fn autottl_parse_rejects_trailing_tokens() {
        assert_eq!(AutoTtl::parse("-1,3-30,5"), None);
    }

    fn orig_bytes(seq: u32, options: &[u8]) -> Vec<u8> {
        OutBuilder::tcp([10, 0, 0, 1], [10, 0, 0, 2], 1000, 443)
            .seq(seq)
            .ack(0x5566_7788)
            .options(options)
            .payload(b"body")
            .build()
            .expect("valid packet")
            .bytes
    }

    #[test]
    fn seq_offsets_wrap() {
        let bytes = orig_bytes(u32::MAX - 15, &[]);
        let orig = Packet::parse(&bytes).expect("valid packet");
        let f = Fooling {
            seq_off: 32,
            ..Default::default()
        };
        let b = crate::packet::OutBuilder::tcp([10, 0, 0, 1], [10, 0, 0, 2], 1000, 443);
        let out = apply_fooling_tcp(b, &f, &orig, None, 1)
            .payload(b"body")
            .build()
            .expect("valid packet");
        let t = Packet::parse(&out.bytes)
            .expect("valid packet")
            .tcp
            .unwrap();
        assert_eq!(t.seq, 16);
    }

    #[test]
    fn tcp_md5_keeps_options_within_40_bytes() {
        let f = Fooling {
            tcp_md5: true,
            ..Default::default()
        };
        // 24 option bytes + 18-byte MD5 option would exceed 40: the MD5
        // option replaces the original options.
        let bytes = orig_bytes(1, &[0x01; 24]);
        let orig = Packet::parse(&bytes).expect("valid packet");
        let b = crate::packet::OutBuilder::tcp([10, 0, 0, 1], [10, 0, 0, 2], 1000, 443);
        let out = apply_fooling_tcp(b, &f, &orig, None, 1)
            .payload(b"body")
            .build()
            .expect("valid packet");
        let t = Packet::parse(&out.bytes)
            .expect("valid packet")
            .tcp
            .unwrap();
        assert_eq!(t.options.len(), 20);
        assert_eq!(&t.options[..2], &[19, 18]);

        // 20 option bytes + MD5 stay within the 40-byte options block.
        let bytes = orig_bytes(1, &[0x01; 20]);
        let orig = Packet::parse(&bytes).expect("valid packet");
        let b = crate::packet::OutBuilder::tcp([10, 0, 0, 1], [10, 0, 0, 2], 1000, 443);
        let out = apply_fooling_tcp(b, &f, &orig, None, 1)
            .payload(b"body")
            .build()
            .expect("valid packet");
        let t = Packet::parse(&out.bytes)
            .expect("valid packet")
            .tcp
            .unwrap();
        assert!(t.options.len() <= 40);
        assert!(t.options.starts_with(&[0x01; 20]));
    }
}
