//! TLS ClientHello parse, craft and mutation (`tls_mod`).
//!
//! Layout offsets follow RFC 8446 §4.1.2 (record 5-byte header + handshake).
//! Every length field that `sni=` replacement has to adjust is captured as an
//! offset during parsing, mirroring zapret2's `TLSMod` (nfq2/protocol.c).

use crate::error::{Error, Result};

pub const TLS_EXT_SERVER_NAME: u16 = 0x0000;
pub const TLS_EXT_PADDING: u16 = 0x0015;

/// Offsets of the length fields and the SNI value inside a full
/// ClientHello record.
#[derive(Debug)]
pub struct ClientHelloLayout {
    /// u16 record length at offset 3.
    pub record_len_offset: usize,
    /// u24 handshake length at offset 6.
    pub handshake_len_offset: usize,
    /// u16 extensions total length.
    pub ext_total_len_offset: usize,
    /// End of the extensions block (exclusive offset into the record).
    pub exts_end: usize,
    /// u16 SNI extension length (of the ext body).
    pub sni_ext_len_offset: Option<usize>,
    /// u16 server_name_list length (inside the ext body).
    pub name_list_len_offset: Option<usize>,
    /// u16 host-entry length, then the hostname bytes.
    pub host_len_offset: Option<usize>,
    pub host_offset: Option<usize>,
    pub host_len: usize,
    /// u8 session id length at offset 43; bytes follow.
    pub session_id_len: usize,
    /// Total size of the record (5 + record_len).
    pub total_len: usize,
}

/// Returns true when `data` starts with a TLS record header that plausibly
/// wraps a ClientHello handshake message.
pub fn looks_like_client_hello(data: &[u8]) -> bool {
    if data.len() < 9 {
        return false;
    }
    if data[0] != 0x16 || data[1] != 0x03 || data[5] != 0x01 {
        return false;
    }
    let record_len = u16::from_be_bytes([data[3], data[4]]) as usize;
    let hs_len = u32::from_be_bytes([0, data[6], data[7], data[8]]) as usize;
    // Handshake length should be record length minus its own 4-byte header.
    record_len >= 4 && hs_len + 4 == record_len
}

/// Full record length if the ClientHello is complete in `data`, else `None`
/// (partial reassembly keeps waiting).
pub fn client_hello_len(data: &[u8]) -> Option<usize> {
    if !looks_like_client_hello(data) {
        return None;
    }
    let record_len = u16::from_be_bytes([data[3], data[4]]) as usize;
    let total = record_len.checked_add(5)?;
    if data.len() >= total {
        Some(total)
    } else {
        None
    }
}

/// Parse the layout of a complete ClientHello record.
pub fn parse_layout(ch: &[u8]) -> Result<ClientHelloLayout> {
    let Some(total) = client_hello_len(ch) else {
        return Err(Error::Packet("not a complete client hello".into()));
    };
    let hs_len = u32::from_be_bytes([0, ch[6], ch[7], ch[8]]) as usize;
    let hs_end = 9usize
        .checked_add(hs_len)
        .filter(|&e| e <= ch.len())
        .ok_or_else(|| Error::Packet("handshake overruns record".into()))?;

    let mut p = 9usize;
    if ch.len() < p + 2 + 32 + 1 {
        return Err(Error::Packet("client hello truncated".into()));
    }
    p += 2; // client version
    p += 32; // random
    let session_id_len = ch[p] as usize;
    p += 1 + session_id_len;
    let read_u16 = |p: usize| -> Result<u16> {
        ch.get(p..p + 2)
            .map(|s| u16::from_be_bytes([s[0], s[1]]))
            .ok_or_else(|| Error::Packet("client hello truncated".into()))
    };
    let cipher_len = read_u16(p)? as usize;
    p += 2 + cipher_len;
    if ch.len() <= p {
        return Err(Error::Packet("client hello truncated".into()));
    }
    let comp_len = ch[p] as usize;
    p += 1 + comp_len;
    if p + 2 > hs_end {
        return Err(Error::Packet("no extensions block".into()));
    }
    // The extensions length field; all tls_mod length adjustments anchor here.
    let ext_total_len_offset = p;
    let exts_len = read_u16(p)? as usize;
    let exts_start = p
        .checked_add(2)
        .filter(|&s| s <= hs_end)
        .ok_or_else(|| Error::Packet("client hello truncated".into()))?;
    let exts_end = exts_start
        .checked_add(exts_len)
        .filter(|&e| e <= hs_end && e <= ch.len())
        .ok_or_else(|| Error::Packet("extensions overrun record".into()))?;

    let mut out = ClientHelloLayout {
        record_len_offset: 3,
        handshake_len_offset: 6,
        ext_total_len_offset,
        exts_end,
        sni_ext_len_offset: None,
        name_list_len_offset: None,
        host_len_offset: None,
        host_offset: None,
        host_len: 0,
        session_id_len,
        total_len: total,
    };

    let mut e = exts_start;
    while e.checked_add(4).is_some_and(|n| n <= exts_end) {
        let etype = read_u16(e)?;
        let elen = read_u16(e + 2)? as usize;
        let body = e + 4;
        let body_end = body
            .checked_add(elen)
            .filter(|&b| b <= exts_end && b <= ch.len())
            .ok_or_else(|| Error::Packet("extension body overrun".into()))?;
        if etype == TLS_EXT_SERVER_NAME {
            out.sni_ext_len_offset = Some(e + 2);
            // server_name_list length, then entries: type(1) len(2) name
            out.name_list_len_offset = Some(body);
            let list_len = read_u16(body)? as usize;
            let list = body
                .checked_add(2)
                .filter(|&s| s <= body_end)
                .ok_or_else(|| Error::Packet("server name list truncated".into()))?;
            let list_end = list
                .checked_add(list_len)
                .filter(|&l| l <= body_end && l <= ch.len())
                .ok_or_else(|| Error::Packet("server name list overrun".into()))?;
            if list_len >= 3 && ch.get(list) == Some(&0) {
                let name_len = read_u16(list + 1)? as usize;
                let _name_end = list
                    .checked_add(3)
                    .and_then(|s| s.checked_add(name_len))
                    .filter(|&n| n <= list_end && n <= ch.len())
                    .ok_or_else(|| Error::Packet("server name overrun".into()))?;
                out.host_len_offset = Some(list + 1);
                out.host_offset = Some(list + 3);
                out.host_len = name_len;
            }
        }
        e = body_end;
    }
    Ok(out)
}

pub fn extract_sni(ch: &[u8]) -> Option<String> {
    let l = parse_layout(ch).ok()?;
    let off = l.host_offset?;
    ch.get(off..off.checked_add(l.host_len)?)
        .map(|s| String::from_utf8_lossy(s).into_owned())
}

/// A generated fake ClientHello record, TLS 1.3-flavoured, with a padding
/// extension reserved at the end so `padencap` can grow it to the real size.
pub fn craft_client_hello(sni: &str, rng: &mut impl rand_bytes::Rng) -> Result<Vec<u8>> {
    let u16v = |v: usize| -> Result<u16> {
        u16::try_from(v)
            .map_err(|_| Error::Packet("crafted client hello exceeds TLS length limits".into()))
    };
    let mut hs: Vec<u8> = Vec::with_capacity(400);
    hs.extend_from_slice(&[0x03, 0x03]); // client version (legacy TLS 1.2)
    hs.extend_from_slice(&rng.bytes(32)); // random

    // session id (32 bytes random)
    hs.push(32);
    hs.extend_from_slice(&rng.bytes(32));

    // cipher suites
    let suites: [u8; 32] = [
        0x13, 0x01, 0x13, 0x02, 0x13, 0x03, 0xC0, 0x2B, 0xC0, 0x2F, 0xC0, 0x2C, 0xC0, 0x30, 0xC0,
        0x0A, 0xC0, 0x14, 0x00, 0x9C, 0x00, 0x9D, 0x00, 0x2F, 0x00, 0x35, 0x00, 0x0A, 0x01, 0x00,
        0x00, 0x00,
    ];
    hs.extend_from_slice(&(suites.len() as u16).to_be_bytes());
    hs.extend_from_slice(&suites);

    // compression methods: null
    hs.extend_from_slice(&[0x01, 0x00]);

    // extensions
    let mut exts: Vec<u8> = Vec::with_capacity(200);
    // server_name: body = server_name_list length field (2) + list content
    let host = sni.as_bytes();
    let list_len = 1 + 2 + host.len();
    exts.extend_from_slice(&TLS_EXT_SERVER_NAME.to_be_bytes());
    exts.extend_from_slice(&u16v(list_len + 2)?.to_be_bytes());
    exts.extend_from_slice(&u16v(list_len)?.to_be_bytes());
    exts.push(0x00);
    exts.extend_from_slice(&u16v(host.len())?.to_be_bytes());
    exts.extend_from_slice(host);
    // supported_groups
    let groups: [u8; 12] = [
        0x00, 0x1D, 0x00, 0x17, 0x00, 0x18, 0x00, 0x19, 0x01, 0x00, 0x01, 0x01,
    ];
    exts.extend_from_slice(&0x000A_u16.to_be_bytes());
    exts.extend_from_slice(&((groups.len() + 2) as u16).to_be_bytes());
    exts.extend_from_slice(&(groups.len() as u16).to_be_bytes());
    exts.extend_from_slice(&groups);
    // signature_algorithms
    let sigalgs: [u8; 24] = [
        0x04, 0x03, 0x08, 0x04, 0x04, 0x01, 0x05, 0x03, 0x08, 0x05, 0x05, 0x01, 0x08, 0x06, 0x06,
        0x01, 0x04, 0x02, 0x02, 0x01, 0x08, 0x0A, 0x08, 0x0B,
    ];
    exts.extend_from_slice(&0x000D_u16.to_be_bytes());
    exts.extend_from_slice(&((sigalgs.len() + 2) as u16).to_be_bytes());
    exts.extend_from_slice(&(sigalgs.len() as u16).to_be_bytes());
    exts.extend_from_slice(&sigalgs);
    // supported_versions: list_len 2, TLS1.3 + TLS1.2
    exts.extend_from_slice(&0x002B_u16.to_be_bytes());
    exts.extend_from_slice(&[0x00, 0x05, 0x02, 0x03, 0x04, 0x03, 0x03]);
    // key_share: x25519 random
    exts.extend_from_slice(&0x0033_u16.to_be_bytes());
    exts.extend_from_slice(&[0x00, 0x26, 0x00, 0x24, 0x00, 0x1D, 0x00, 0x20]);
    exts.extend_from_slice(&rng.bytes(32));
    // alpn: list_len 12 = "h2"(3) + "http/1.1"(9); ext len = 2+12 = 14
    exts.extend_from_slice(&0x0010_u16.to_be_bytes());
    exts.extend_from_slice(&[
        0x00, 0x0E, 0x00, 0x0C, 0x02, b'h', b'2', 0x08, b'h', b't', b't', b'p', b'/', b'1', b'.',
        b'1',
    ]);
    // session_ticket (empty)
    exts.extend_from_slice(&0x0023_u16.to_be_bytes());
    exts.extend_from_slice(&[0x00, 0x00]);
    // padding (empty body; padencap grows it)
    exts.extend_from_slice(&TLS_EXT_PADDING.to_be_bytes());
    exts.extend_from_slice(&[0x00, 0x00]);

    hs.extend_from_slice(&u16v(exts.len())?.to_be_bytes());
    hs.extend_from_slice(&exts);

    let mut rec: Vec<u8> = Vec::with_capacity(hs.len() + 9);
    rec.extend_from_slice(&[0x16, 0x03, 0x01]);
    let hs_len = hs.len();
    let rl = u16v(hs_len + 4)?;
    rec.push((rl >> 8) as u8); // record length at [3,4]
    rec.push(rl as u8);
    rec.push(0x01); // handshake type: client_hello at [5]
    rec.extend_from_slice(&(hs_len as u32).to_be_bytes()[1..4]); // u24 length at [6..9]
    rec.extend_from_slice(&hs);
    Ok(rec)
}

/// Mutations applied to a fake ClientHello, mirroring zapret2 `TLSMod`.
#[derive(Debug, Default, Clone)]
pub struct TlsMods {
    pub rnd: bool,
    pub rndsni: bool,
    pub sni: Option<String>,
    pub dupsid: bool,
    pub padencap: bool,
}

pub fn apply_tls_mods(
    fake: &mut Vec<u8>,
    mods: &TlsMods,
    real: Option<&[u8]>,
    rng: &mut impl rand_bytes::Rng,
) {
    // Only act on a parseable ClientHello at all.
    if parse_layout(fake).is_err() {
        return;
    }
    // sni= / rndsni rewrite the SNI value; run sni= first (deterministic).
    if let Some(sni) = &mods.sni {
        replace_sni(fake, sni);
    }
    if mods.rndsni {
        randomize_sni(fake, rng);
    }
    if mods.rnd {
        // ClientHello.random and the session id.
        if let Ok(l2) = parse_layout(fake) {
            let end = 44 + l2.session_id_len;
            let bytes = rng.bytes(32 + l2.session_id_len);
            fake[11..43].copy_from_slice(&bytes[..32]);
            if l2.session_id_len > 0 {
                fake[44..end].copy_from_slice(&bytes[32..]);
            }
        }
    }
    if mods.dupsid {
        // Copy the real ClientHello's session id when the lengths match.
        if let Some(real) = real
            && let (Ok(rl), Ok(ol)) = (parse_layout(real), parse_layout(fake))
            && rl.session_id_len == ol.session_id_len
            && rl.session_id_len > 0
        {
            fake[44..44 + rl.session_id_len].copy_from_slice(&real[44..44 + rl.session_id_len]);
        }
    }
    if mods.padencap
        && let Err(e) = pad_to_size(fake, real.map_or(0, |r| r.len()))
    {
        log::debug!("padencap skipped: {e}");
    }
}

/// Replace the SNI hostname, adjusting every enclosing length field.
/// Leaves `fake` unchanged when the new name or any resulting length would
/// exceed the TLS u16 limits.
pub fn replace_sni(fake: &mut Vec<u8>, sni: &str) {
    let Ok(l) = parse_layout(fake) else {
        return;
    };
    let (Some(hoff), Some(hlenoff)) = (l.host_offset, l.host_len_offset) else {
        return;
    };
    let new = sni.as_bytes();
    let delta = new.len() as isize - l.host_len as isize;
    if new.len() > u16::MAX as usize {
        return;
    }
    let read_u16 = |off: usize| -> isize {
        u16::from_be_bytes([
            fake.get(off).copied().unwrap_or(0),
            fake.get(off + 1).copied().unwrap_or(0),
        ]) as isize
    };
    let name_list_old = l.name_list_len_offset.map_or(0, read_u16);
    let sni_ext_old = l.sni_ext_len_offset.map_or(0, read_u16);
    let ext_old = read_u16(l.ext_total_len_offset);
    let rec_old = read_u16(l.record_len_offset);
    for old in [name_list_old, sni_ext_old, ext_old, rec_old] {
        if !(0..=u16::MAX as isize).contains(&(old + delta)) {
            return;
        }
    }
    // Overwrite the common part in place, then splice/drain the size delta.
    let common = new.len().min(l.host_len);
    fake[hoff..hoff + common].copy_from_slice(&new[..common]);
    if new.len() > l.host_len {
        fake.splice(hoff + common..hoff + common, new[common..].to_vec());
    } else if new.len() < l.host_len {
        fake.drain(hoff + new.len()..hoff + l.host_len);
    }
    let put_u16 = |buf: &mut Vec<u8>, off: usize, v: usize| {
        if off + 2 <= buf.len() {
            let b = (v as u16).to_be_bytes();
            buf[off] = b[0];
            buf[off + 1] = b[1];
        }
    };
    put_u16(fake, hlenoff, new.len());
    if let Some(off) = l.name_list_len_offset {
        let old = u16::from_be_bytes([
            fake.get(off).copied().unwrap_or(0),
            fake.get(off + 1).copied().unwrap_or(0),
        ]);
        put_u16(fake, off, (old as isize + delta).max(0) as usize);
    }
    if let Some(off) = l.sni_ext_len_offset {
        let old = u16::from_be_bytes([
            fake.get(off).copied().unwrap_or(0),
            fake.get(off + 1).copied().unwrap_or(0),
        ]);
        put_u16(fake, off, (old as isize + delta).max(0) as usize);
    }
    {
        let old = u16::from_be_bytes([
            fake[l.ext_total_len_offset],
            fake[l.ext_total_len_offset + 1],
        ]);
        put_u16(
            fake,
            l.ext_total_len_offset,
            (old as isize + delta).max(0) as usize,
        );
    }
    {
        let old = u16::from_be_bytes([fake[l.record_len_offset], fake[l.record_len_offset + 1]]);
        put_u16(
            fake,
            l.record_len_offset,
            (old as isize + delta).max(0) as usize,
        );
    }
    let hs_off = l.handshake_len_offset;
    let old_hs = u32::from_be_bytes([0, fake[hs_off], fake[hs_off + 1], fake[hs_off + 2]]);
    let nv = ((old_hs as isize + delta).max(0)) as u32;
    fake[hs_off] = (nv >> 16) as u8;
    fake[hs_off + 1] = (nv >> 8) as u8;
    fake[hs_off + 2] = nv as u8;
}

/// Overwrite the SNI hostname with random lowercase/alnum text, keeping the
/// length constant (nfqws `fill_random_az`/`fill_random_az09` semantics).
pub fn randomize_sni(fake: &mut [u8], rng: &mut impl rand_bytes::Rng) {
    let Ok(l) = parse_layout(fake) else {
        return;
    };
    let Some(hoff) = l.host_offset else {
        return;
    };
    let slen = l.host_len;
    if slen == 0 || hoff + slen > fake.len() {
        return;
    }
    let b: u8 = b'a' + rng.bytes(1)[0] % 26;
    fake[hoff] = b;
    if slen >= 7 {
        for byte in &mut fake[hoff + 1..hoff + slen - 4] {
            *byte = match rng.bytes(1)[0] % 36 {
                d @ 0..=9 => b'0' + d,
                d => b'a' + (d - 10),
            };
        }
        fake[hoff + slen - 4] = b'.';
        let tlds: [&[u8; 3]; 5] = [b"com", b"net", b"org", b"dev", b"ioo"];
        fake[hoff + slen - 3..hoff + slen]
            .copy_from_slice(tlds[rng.bytes(1)[0] as usize % tlds.len()]);
    } else {
        for byte in &mut fake[hoff + 1..hoff + slen] {
            *byte = match rng.bytes(1)[0] % 36 {
                d @ 0..=9 => b'0' + d,
                d => b'a' + (d - 10),
            };
        }
    }
}

/// Grow or shrink the trailing padding extension so the fake has the same
/// total size as `target` (zapret2 `padencap`). Refuses targets that would
/// exceed the TLS u16 length limits and leaves `fake` unchanged then.
pub fn pad_to_size(fake: &mut Vec<u8>, target: usize) -> Result<()> {
    let l = parse_layout(fake)?;
    // Locate the padding extension (must be last for our crafted fakes).
    let pad = find_padding_ext(fake)?;
    let cur = fake.len() as isize;
    let delta = target as isize - cur;
    if delta == 0 {
        return Ok(());
    }
    let new_body = (delta + pad.body_len as isize).max(0) as usize;
    if new_body > u16::MAX as usize {
        return Err(Error::Packet(
            "padencap target exceeds the TLS length limits".into(),
        ));
    }
    let d2 = new_body as isize - pad.body_len as isize;
    let ext_old = u16::from_be_bytes([
        fake[l.ext_total_len_offset],
        fake[l.ext_total_len_offset + 1],
    ]) as isize;
    let rec_old =
        u16::from_be_bytes([fake[l.record_len_offset], fake[l.record_len_offset + 1]]) as isize;
    if !(0..=u16::MAX as isize).contains(&(ext_old + d2))
        || !(0..=u16::MAX as isize).contains(&(rec_old + d2))
    {
        return Err(Error::Packet(
            "padencap target exceeds the TLS length limits".into(),
        ));
    }
    fake.splice(
        pad.body_offset..pad.body_offset + pad.body_len,
        vec![0u8; new_body],
    );
    let put_u16 = |buf: &mut Vec<u8>, off: usize, v: usize| {
        let b = (v as u16).to_be_bytes();
        buf[off] = b[0];
        buf[off + 1] = b[1];
    };
    put_u16(fake, pad.len_offset, new_body);
    put_u16(fake, l.ext_total_len_offset, (ext_old + d2).max(0) as usize);
    put_u16(fake, l.record_len_offset, (rec_old + d2).max(0) as usize);
    let hs_off = l.handshake_len_offset;
    let old = u32::from_be_bytes([0, fake[hs_off], fake[hs_off + 1], fake[hs_off + 2]]);
    let nv = ((old as isize + d2).max(0)) as u32;
    fake[hs_off] = (nv >> 16) as u8;
    fake[hs_off + 1] = (nv >> 8) as u8;
    fake[hs_off + 2] = nv as u8;
    Ok(())
}

struct PaddingExt {
    len_offset: usize,
    body_offset: usize,
    body_len: usize,
}

fn find_padding_ext(ch: &[u8]) -> Result<PaddingExt> {
    let l = parse_layout(ch)?;
    let mut p = l.ext_total_len_offset + 2;
    let end = l.exts_end;
    while p + 4 <= end {
        let etype = u16::from_be_bytes([ch[p], ch[p + 1]]);
        let elen = u16::from_be_bytes([ch[p + 2], ch[p + 3]]) as usize;
        if p + 4 + elen > end {
            return Err(Error::Packet(
                "extension overruns the extensions block".into(),
            ));
        }
        if etype == TLS_EXT_PADDING {
            return Ok(PaddingExt {
                len_offset: p + 2,
                body_offset: p + 4,
                body_len: elen,
            });
        }
        p += 4 + elen;
    }
    Err(Error::Packet("no padding extension".into()))
}

/// Minimal helper trait so tests can inject deterministic randomness.
pub mod rand_bytes {
    pub trait Rng {
        fn bytes(&mut self, n: usize) -> Vec<u8>;

        /// Random IP identification value for generated packets.
        fn ip_id(&mut self) -> u16 {
            let b = self.bytes(2);
            u16::from_be_bytes([b[0], b[1]])
        }
    }

    /// OS-backed RNG.
    #[derive(Default)]
    pub struct OsRng;

    impl Rng for OsRng {
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            let mut buf = vec![0u8; n];
            let mut filled = 0usize;
            while filled < n {
                let ptr = unsafe { buf.as_mut_ptr().add(filled) };
                let rc = unsafe { libc::getrandom(ptr.cast(), n - filled, 0) };
                if rc >= 0 {
                    filled += rc as usize;
                } else {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != std::io::ErrorKind::Interrupted {
                        panic!("getrandom: {err}");
                    }
                }
            }
            buf
        }
    }

    /// Deterministic counter-based RNG for tests.
    #[derive(Default)]
    pub struct Counter(pub u64);

    impl Rng for Counter {
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n)
                .map(|i| ((self.0 + i as u64 * 31) % 256) as u8)
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Vec<u8> {
        craft_client_hello("www.microsoft.com", &mut rand_bytes::Counter(0))
            .expect("crafted client hello")
    }

    #[test]
    fn crafted_hello_parses() {
        let ch = fake();
        assert!(looks_like_client_hello(&ch));
        assert_eq!(client_hello_len(&ch), Some(ch.len()));
        assert_eq!(extract_sni(&ch).as_deref(), Some("www.microsoft.com"));
    }

    #[test]
    fn sni_replacement_adjusts_lengths() {
        let mut ch = fake();
        let orig = ch.clone();
        replace_sni(&mut ch, "fonts.google.com");
        assert_eq!(extract_sni(&ch).as_deref(), Some("fonts.google.com"));
        let delta = "fonts.google.com".len() as isize - "www.microsoft.com".len() as isize;
        let l0 = parse_layout(&orig).unwrap();
        let l1 = parse_layout(&ch).unwrap();
        let rl0 = u16::from_be_bytes([orig[3], orig[4]]);
        let rl1 = u16::from_be_bytes([ch[3], ch[4]]);
        assert_eq!(rl1 as isize, rl0 as isize + delta);
        assert_eq!(l1.total_len, ch.len());
        // ext total length grew by the same delta
        let e0 = u16::from_be_bytes([
            orig[l0.ext_total_len_offset],
            orig[l0.ext_total_len_offset + 1],
        ]);
        let e1 = u16::from_be_bytes([ch[l1.ext_total_len_offset], ch[l1.ext_total_len_offset + 1]]);
        assert_eq!(e1 as isize, e0 as isize + delta);
    }

    #[test]
    fn sni_replacement_shorter() {
        let mut ch = fake();
        replace_sni(&mut ch, "a.ru");
        assert_eq!(extract_sni(&ch).as_deref(), Some("a.ru"));
        assert_eq!(parse_layout(&ch).unwrap().total_len, ch.len());
    }

    #[test]
    fn randomize_sni_keeps_length_and_shape() {
        let mut ch = fake();
        let before = ch.clone();
        randomize_sni(&mut ch, &mut rand_bytes::Counter(7));
        assert_eq!(ch.len(), before.len());
        let sni = extract_sni(&ch).unwrap();
        assert_eq!(sni.len(), "www.microsoft.com".len());
        let first = sni.as_bytes()[0];
        assert!(first.is_ascii_lowercase());
        assert!(sni.contains('.'));
    }

    #[test]
    fn padencap_grows_to_real_size() {
        let real = fake();
        let mut small =
            craft_client_hello("a.ru", &mut rand_bytes::Counter(3)).expect("crafted client hello");
        assert!(small.len() < real.len());
        eprintln!(
            "DEBUG find pad: {:?}",
            find_padding_ext(&small)
                .map(|p| (p.len_offset, p.body_offset, p.body_len))
                .map_err(|e| e.to_string())
        );
        pad_to_size(&mut small, real.len()).expect("padencap fits");
        assert_eq!(small.len(), real.len());
        // Still a valid hello afterwards.
        assert_eq!(client_hello_len(&small), Some(small.len()));
        assert_eq!(extract_sni(&small).as_deref(), Some("a.ru"));
    }

    #[test]
    fn dupsid_copies_session_id() {
        let real = fake();
        let mut other = craft_client_hello("example.net", &mut rand_bytes::Counter(9))
            .expect("crafted client hello");
        let mods = TlsMods {
            dupsid: true,
            ..Default::default()
        };
        apply_tls_mods(&mut other, &mods, Some(&real), &mut rand_bytes::Counter(1));
        let rl = parse_layout(&real).unwrap();
        let ol = parse_layout(&other).unwrap();
        assert_eq!(rl.session_id_len, ol.session_id_len);
        assert_eq!(
            &real[44..44 + rl.session_id_len],
            &other[44..44 + ol.session_id_len]
        );
    }

    #[test]
    fn rnd_randomizes() {
        let mut a = fake();
        let mut b = fake();
        let mods = TlsMods {
            rnd: true,
            ..Default::default()
        };
        apply_tls_mods(&mut a, &mods, None, &mut rand_bytes::Counter(1));
        apply_tls_mods(&mut b, &mods, None, &mut rand_bytes::Counter(2));
        assert_ne!(a[11..43], b[11..43]);
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn tls_mods_chain() {
        let real = fake();
        let mut m = craft_client_hello("www.microsoft.com", &mut rand_bytes::Counter(5))
            .expect("crafted client hello");
        let mods = TlsMods {
            rnd: true,
            sni: Some("fonts.google.com".into()),
            dupsid: true,
            padencap: true,
            ..Default::default()
        };
        apply_tls_mods(&mut m, &mods, Some(&real), &mut rand_bytes::Counter(11));
        assert_eq!(m.len(), real.len());
        assert_eq!(extract_sni(&m).as_deref(), Some("fonts.google.com"));
        assert_eq!(client_hello_len(&m), Some(m.len()));
    }

    #[test]
    fn partial_record_not_detected() {
        let ch = fake();
        assert_eq!(client_hello_len(&ch[..ch.len() - 1]), None);
        assert!(!looks_like_client_hello(&[
            0x17, 0x03, 0x03, 0, 4, 1, 0, 0, 0
        ]));
    }

    #[test]
    fn truncated_record_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let rl = u16::from_be_bytes([m[3], m[4]]);
        m[3..5].copy_from_slice(&(rl.wrapping_add(1)).to_be_bytes());
        assert_eq!(client_hello_len(&m), None);
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn truncated_handshake_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let hl = u32::from_be_bytes([0, m[6], m[7], m[8]]);
        let nv = hl + 1;
        m[6..9].copy_from_slice(&nv.to_be_bytes()[1..]);
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn lying_extensions_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let l = parse_layout(&ch).unwrap();
        let old = u16::from_be_bytes([m[l.ext_total_len_offset], m[l.ext_total_len_offset + 1]]);
        m[l.ext_total_len_offset..l.ext_total_len_offset + 2]
            .copy_from_slice(&old.wrapping_add(1).to_be_bytes());
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn lying_extensions_len_beyond_buffer_does_not_panic() {
        let ch = fake();
        let mut m = ch.clone();
        let l = parse_layout(&ch).unwrap();
        m[l.ext_total_len_offset..l.ext_total_len_offset + 2]
            .copy_from_slice(&0xffffu16.to_be_bytes());
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn truncated_extension_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let l = parse_layout(&ch).unwrap();
        let p = l.sni_ext_len_offset.unwrap();
        let old = u16::from_be_bytes([m[p], m[p + 1]]);
        m[p..p + 2].copy_from_slice(&old.wrapping_add(1).to_be_bytes());
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn truncated_name_list_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let l = parse_layout(&ch).unwrap();
        let p = l.name_list_len_offset.unwrap();
        let old = u16::from_be_bytes([m[p], m[p + 1]]);
        m[p..p + 2].copy_from_slice(&old.wrapping_add(1).to_be_bytes());
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn truncated_name_len_rejected() {
        let ch = fake();
        let mut m = ch.clone();
        let l = parse_layout(&ch).unwrap();
        let p = l.host_len_offset.unwrap();
        let old = u16::from_be_bytes([m[p], m[p + 1]]);
        m[p..p + 2].copy_from_slice(&old.wrapping_add(1).to_be_bytes());
        assert!(parse_layout(&m).is_err());
    }

    #[test]
    fn crafted_hello_rejects_an_oversized_sni() {
        let ok = craft_client_hello(&"a".repeat(60_000), &mut rand_bytes::Counter(0));
        assert!(ok.is_ok());
        let too_big = craft_client_hello(&"a".repeat(65_500), &mut rand_bytes::Counter(0));
        assert!(too_big.is_err());
    }

    #[test]
    fn replace_sni_refuses_lengths_over_u16() {
        let mut ch = fake();
        replace_sni(&mut ch, &"a".repeat(60_000));
        assert_eq!(extract_sni(&ch).as_deref(), Some(&"a".repeat(60_000)[..]));
        replace_sni(&mut ch, &"a".repeat(65_500));
        assert_eq!(extract_sni(&ch).as_deref(), Some(&"a".repeat(60_000)[..]));
    }

    #[test]
    fn pad_to_size_rejects_an_oversized_padding_extension() {
        let mut ch = fake();
        let l = parse_layout(&ch).unwrap();
        let pad = find_padding_ext(&ch).unwrap();
        ch[pad.len_offset..pad.len_offset + 2].copy_from_slice(&0xFFFFu16.to_be_bytes());
        let before = ch.clone();
        assert!(pad_to_size(&mut ch, l.total_len + 10).is_err());
        assert_eq!(ch, before);
    }

    #[test]
    fn pad_to_size_accepts_trailing_bytes() {
        let mut ch = fake();
        ch.extend_from_slice(b"tail");
        let target = ch.len() + 10;
        pad_to_size(&mut ch, target).expect("pad fits");
        assert_eq!(ch.len(), target);
        assert_eq!(client_hello_len(&ch), Some(ch.len() - 4));
    }
}
