//! Strategy strings: nfqws2-like `name:arg=value:...` syntax.
//!
//! A chain is one or more attacks joined by `+`, applied to a single
//! connection's first data burst, e.g.
//! `fake:blob=tls:tls_mod=rnd,sni=fonts.google.com:tcp_seq=10000+multisplit:pos=1,midsld:seqovl=1`.
//! Multiple chains passed via repeated `--strategy` are rotated by the
//! circular orchestrator in the engine.

use crate::error::{Error, Result};
use crate::fooling::Fooling;
use crate::positions::Marker;
use crate::tls::TlsMods;

/// A payload blob, materialized at fire time.
#[derive(Debug, Clone)]
pub enum Blob {
    /// Crafted TLS ClientHello with the given SNI.
    Tls(String),
    /// Crafted ~1200 B QUIC v1 Initial.
    Quic,
    /// Static bytes (hex literal in the strategy string).
    Hex(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct FakeSpec {
    pub blob: Blob,
    pub tls_mods: TlsMods,
    pub fooling: Fooling,
}

#[derive(Debug, Clone)]
pub struct SplitSpec {
    pub markers: Vec<Marker>,
    pub seqovl: usize,
    pub seqovl_pattern: Option<Vec<u8>>,
    /// Send segments out of order (multidisorder).
    pub disorder: bool,
}

#[derive(Debug, Clone)]
pub enum Attack {
    Fake(FakeSpec),
    Split(SplitSpec),
}

/// One `--strategy` chain: attacks applied in order.
#[derive(Debug, Clone)]
pub struct Chain {
    pub attacks: Vec<Attack>,
}

impl Chain {
    /// True when this chain carries any split or a non-QUIC fake.
    pub fn applies_to_tcp(&self) -> bool {
        self.attacks.iter().any(|a| match a {
            Attack::Split(_) => true,
            Attack::Fake(spec) => !matches!(spec.blob, Blob::Quic),
        })
    }

    /// True when this chain carries a QUIC or raw-blob fake.
    pub fn applies_to_udp(&self) -> bool {
        self.attacks.iter().any(|a| match a {
            Attack::Fake(spec) => !matches!(spec.blob, Blob::Tls(_)),
            Attack::Split(_) => false,
        })
    }
}

/// Keenetic-default-inspired chains: TLS decoy + ClientHello split, QUIC
/// Initial decoy. Multiple `--strategy` flags enable circular rotation.
pub const DEFAULT_CHAINS: &[&str] = &[
    "fake:blob=tls:tls_mod=rnd,dupsid,sni=fonts.google.com:tcp_seq=10000+multisplit:pos=2,midsld:seqovl=1",
    "fake:blob=quic:repeats=11",
    "multidisorder:pos=midsld,endhost-2:seqovl=4",
];

const FAKE_PARAMS: &[&str] = &[
    "blob",
    "sni_blob",
    "tls_mod",
    "ip_ttl",
    "ip_autottl",
    "badsum",
    "tcp_seq",
    "tcp_ack",
    "tcp_md5",
    "tcp_flags_unset",
    "repeats",
];

const SPLIT_PARAMS: &[&str] = &["pos", "seqovl", "seqovl_pattern"];

/// Parse a chain string like `fake:blob=tls:tcp_seq=10000+multisplit:pos=1,midsld`.
pub fn parse_chain(s: &str) -> Result<Chain> {
    let mut attacks = Vec::new();
    for part in s.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut seg = part.split(':');
        let name = seg
            .next()
            .ok_or_else(|| Error::Strategy("empty attack".into()))?;
        let mut args: Vec<(String, String)> = Vec::new();
        for a in seg {
            let (k, v) = match a.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => (a.trim(), ""),
            };
            args.push((k.to_string(), v.to_string()));
        }
        let allowed = match name {
            "fake" => FAKE_PARAMS,
            "multisplit" | "multidisorder" => SPLIT_PARAMS,
            _ => &[],
        };
        let mut seen: Vec<&str> = Vec::new();
        for (k, _) in &args {
            if !allowed.contains(&k.as_str()) {
                return Err(Error::Strategy(format!(
                    "unknown parameter '{k}' in '{name}'"
                )));
            }
            if seen.contains(&k.as_str()) {
                return Err(Error::Strategy(format!(
                    "duplicate parameter '{k}' in '{name}'"
                )));
            }
            seen.push(k.as_str());
        }
        attacks.push(match name {
            "fake" => Attack::Fake(parse_fake(&args)?),
            "multisplit" => Attack::Split(parse_split(&args, false)?),
            "multidisorder" => Attack::Split(parse_split(&args, true)?),
            other => return Err(Error::Strategy(format!("unknown attack '{other}'"))),
        });
    }
    if attacks.is_empty() {
        return Err(Error::Strategy("empty strategy chain".into()));
    }
    Ok(Chain { attacks })
}

fn get<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn parse_fake(args: &[(String, String)]) -> Result<FakeSpec> {
    let blob = match get(args, "blob").unwrap_or("tls") {
        "tls" => Blob::Tls(
            get(args, "sni_blob")
                .map(str::to_string)
                .unwrap_or_else(|| "www.microsoft.com".to_string()),
        ),
        "quic" => Blob::Quic,
        hex if hex.starts_with("0x") => {
            let body = &hex[2..];
            let bytes = decode_hex(body)?;
            Blob::Hex(bytes)
        }
        other => return Err(Error::Strategy(format!("unknown blob '{other}'"))),
    };
    let mut mods = TlsMods::default();
    if let Some(list) = get(args, "tls_mod") {
        for m in list.split(',') {
            let m = m.trim();
            if m.is_empty() {
                continue;
            }
            match m {
                "rnd" => mods.rnd = true,
                "rndsni" => mods.rndsni = true,
                "dupsid" => mods.dupsid = true,
                "padencap" => mods.padencap = true,
                rest if rest.starts_with("sni=") => mods.sni = Some(rest[4..].to_string()),
                other => return Err(Error::Strategy(format!("unknown tls_mod '{other}'"))),
            }
        }
    }
    let fooling = parse_fooling(args)?;
    Ok(FakeSpec {
        blob,
        tls_mods: mods,
        fooling,
    })
}

fn parse_flag(args: &[(String, String)], key: &str) -> Result<bool> {
    match get(args, key) {
        None => Ok(false),
        Some("") | Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(v) if v.eq_ignore_ascii_case("true") => Ok(true),
        Some(v) if v.eq_ignore_ascii_case("false") => Ok(false),
        Some(_) => Err(Error::Strategy(format!(
            "bad {key}: expected true or false"
        ))),
    }
}

fn parse_fooling(args: &[(String, String)]) -> Result<Fooling> {
    let mut f = Fooling::default();
    if let Some(v) = get(args, "ip_ttl") {
        f.ttl = Some(
            v.parse()
                .map_err(|_| Error::Strategy("bad ip_ttl".into()))?,
        );
    }
    if let Some(v) = get(args, "ip_autottl") {
        f.autottl = Some(
            crate::fooling::AutoTtl::parse(v)
                .ok_or_else(|| Error::Strategy(format!("bad ip_autottl '{v}'")))?,
        );
    }
    if parse_flag(args, "badsum")? {
        f.badsum = true;
    }
    if let Some(v) = get(args, "tcp_seq") {
        f.seq_off = v
            .parse()
            .map_err(|_| Error::Strategy("bad tcp_seq".into()))?;
    }
    if let Some(v) = get(args, "tcp_ack") {
        f.ack_off = v
            .parse()
            .map_err(|_| Error::Strategy("bad tcp_ack".into()))?;
    }
    if parse_flag(args, "tcp_md5")? {
        f.tcp_md5 = true;
    }
    if let Some(v) = get(args, "tcp_flags_unset") {
        for flag in v.split(',') {
            match flag.trim().to_ascii_uppercase().as_str() {
                "ACK" => f.flags_unset |= crate::packet::TCP_FLAG_ACK,
                "PSH" => f.flags_unset |= crate::packet::TCP_FLAG_PSH,
                other => return Err(Error::Strategy(format!("unknown tcp flag '{other}'"))),
            }
        }
    }
    if let Some(v) = get(args, "repeats") {
        f.repeats = v
            .parse()
            .map_err(|_| Error::Strategy("bad repeats".into()))?;
        if !(1..=64).contains(&f.repeats) {
            return Err(Error::Strategy("repeats must be between 1 and 64".into()));
        }
    }
    Ok(f)
}

fn parse_split(args: &[(String, String)], disorder: bool) -> Result<SplitSpec> {
    let pos_str = get(args, "pos").unwrap_or("2");
    let mut markers = Vec::new();
    for tok in pos_str.split(',') {
        markers.push(
            Marker::parse(tok.trim())
                .ok_or_else(|| Error::Strategy(format!("bad split position '{tok}'")))?,
        );
    }
    let seqovl = get(args, "seqovl")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| Error::Strategy("bad seqovl".into()))
        })
        .transpose()?
        .unwrap_or(0);
    if seqovl > 1500 {
        return Err(Error::Strategy("seqovl must be at most 1500".into()));
    }
    let seqovl_pattern = match get(args, "seqovl_pattern") {
        Some(p) if p.starts_with("0x") => Some(decode_hex(&p[2..])?),
        Some(other) => return Err(Error::Strategy(format!("unknown seqovl_pattern '{other}'"))),
        None => None,
    };
    Ok(SplitSpec {
        markers,
        seqovl,
        seqovl_pattern,
        disorder,
    })
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    let s = s.replace('_', "");
    if !s.is_ascii()
        || s.is_empty()
        || !s.len().is_multiple_of(2)
        || !s.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Strategy(
            "bad hex blob: expected an even number of hex digits".into(),
        ));
    }
    (0..s.len() / 2)
        .map(|i| {
            u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| Error::Strategy("bad hex blob".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fake_chain() {
        let c = parse_chain("fake:blob=tls:tls_mod=rnd,dupsid,sni=fonts.google.com:tcp_seq=10000")
            .unwrap();
        assert_eq!(c.attacks.len(), 1);
        let Attack::Fake(f) = &c.attacks[0] else {
            panic!()
        };
        assert!(matches!(f.blob, Blob::Tls(_)));
        assert!(f.tls_mods.rnd);
        assert!(f.tls_mods.dupsid);
        assert_eq!(f.tls_mods.sni.as_deref(), Some("fonts.google.com"));
        assert_eq!(f.fooling.seq_off, 10000);
    }

    #[test]
    fn parse_fake_chain_defaults_to_one_repeat() {
        let c = parse_chain("fake:blob=tls").unwrap();
        let Attack::Fake(f) = &c.attacks[0] else {
            panic!()
        };
        assert_eq!(f.fooling.repeats, 1);
    }

    #[test]
    fn parse_chain_with_split() {
        let c = parse_chain("fake:blob=tls:repeats=2+multisplit:pos=1,midsld:seqovl=1").unwrap();
        assert_eq!(c.attacks.len(), 2);
        let Attack::Split(s) = &c.attacks[1] else {
            panic!()
        };
        assert!(!s.disorder);
        assert_eq!(s.seqovl, 1);
        assert_eq!(s.markers.len(), 2);
        // pos=1 is filtered at resolve time, kept here
    }

    #[test]
    fn parse_disorder_and_hex_blob() {
        let c = parse_chain("multidisorder:pos=midsld,endhost-2:seqovl=4:seqovl_pattern=0x001122")
            .unwrap();
        let Attack::Split(s) = &c.attacks[0] else {
            panic!()
        };
        assert!(s.disorder);
        assert_eq!(s.seqovl, 4);
        assert_eq!(s.seqovl_pattern.as_deref(), Some(&[0x00, 0x11, 0x22][..]));
    }

    #[test]
    fn parse_quic_fake() {
        let c = parse_chain("fake:blob=quic:repeats=11").unwrap();
        let Attack::Fake(f) = &c.attacks[0] else {
            panic!()
        };
        assert!(matches!(f.blob, Blob::Quic));
        assert_eq!(f.fooling.repeats, 11);
    }

    #[test]
    fn bad_syntax_rejected() {
        assert!(parse_chain("unknown:blob=tls").is_err());
        assert!(parse_chain("").is_err());
        assert!(parse_chain("fake:tls_mod=zzz").is_err());
        assert!(parse_chain("multisplit:pos=zzz").is_err());
    }

    #[test]
    fn unknown_and_duplicate_parameters_are_rejected() {
        assert!(parse_chain("fake:blob=tls:sni=example.com").is_err());
        assert!(parse_chain("fake:blob=tls:blob=quic").is_err());
        assert!(parse_chain("multisplit:pos=1:seqovl=1:seqovl=2").is_err());
        assert!(parse_chain("multisplit:repeats=2").is_err());
    }

    #[test]
    fn hex_blobs_reject_non_ascii_and_odd_lengths() {
        assert!(parse_chain("fake:blob=0xaé").is_err());
        assert!(parse_chain("fake:blob=0xabc").is_err());
        assert!(parse_chain("fake:blob=0x").is_err());
        assert!(parse_chain("fake:blob=0x+1").is_err());
        assert!(parse_chain("fake:blob=0x00_11").is_ok());
    }

    #[test]
    fn repeats_and_seqovl_are_bounded() {
        assert!(parse_chain("fake:blob=tls:repeats=0").is_err());
        assert!(parse_chain("fake:blob=tls:repeats=65").is_err());
        let c = parse_chain("fake:blob=tls:repeats=64").unwrap();
        let Attack::Fake(f) = &c.attacks[0] else {
            panic!()
        };
        assert_eq!(f.fooling.repeats, 64);
        assert!(parse_chain("multisplit:pos=2:seqovl=1501").is_err());
        assert!(parse_chain("multisplit:pos=2:seqovl=1500").is_ok());
    }

    #[test]
    fn boolean_parameters_are_validated() {
        let c = parse_chain("fake:blob=tls:badsum=false").unwrap();
        let Attack::Fake(f) = &c.attacks[0] else {
            panic!()
        };
        assert!(!f.fooling.badsum);
        assert!(parse_chain("fake:blob=tls:badsum=maybe").is_err());
    }

    #[test]
    fn default_chains_partition_by_protocol() {
        let got: Vec<(bool, bool)> = DEFAULT_CHAINS
            .iter()
            .map(|s| {
                let c = parse_chain(s).unwrap();
                (c.applies_to_tcp(), c.applies_to_udp())
            })
            .collect();
        assert_eq!(got, vec![(true, false), (false, true), (true, false)]);
    }
}
