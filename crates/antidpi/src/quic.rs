//! QUIC Initial detection and fake Initial crafting.
//!
//! The fake is a standalone datagram (~1200 B, RFC 9000 §14.1 Initial size)
//! with its own random connection id: the server never processes it, while
//! the DPI latches onto the first Initial it sees. Mirrors nfqws'
//! `fake:blob=quic_initial:repeats=N` usage.

use crate::tls::rand_bytes::Rng;

pub const QUIC_VERSION_V1: u32 = 0x0000_0001;
pub const QUIC_VERSION_V2: u32 = 0x6b33_43cf;

/// Long-header Initial of QUIC v1/v2: fixed bit set, type bits 0, known
/// version. (RFC 9000 §17.2.2: bits 4-5 of byte 0 are 0b00 for Initial.)
pub fn looks_like_quic_initial(data: &[u8]) -> bool {
    data.len() >= 6
        && data[0] & 0xC0 == 0xC0
        && data[0] & 0x30 == 0x00
        && matches!(
            u32::from_be_bytes([data[1], data[2], data[3], data[4]]),
            QUIC_VERSION_V1 | QUIC_VERSION_V2
        )
}

/// Destination connection id of a long-header packet.
pub fn extract_dcid(data: &[u8]) -> Option<&[u8]> {
    let len = *data.get(5)? as usize;
    data.get(6..6 + len)
}

/// Build a plausible 1200-byte QUIC v1 Initial datagram. Payload bytes after
/// the header are random: the server never decrypts them and DPI sees
/// ciphertext either way.
pub fn craft_quic_initial(rng: &mut impl Rng) -> Vec<u8> {
    let target = 1200usize;
    let dcid_len: u8 = 8;
    let scid_len: u8 = 8;
    let mut d = Vec::with_capacity(target);
    // Long header, Initial, PN length 4 (0b11) — byte 0: 0b11000000 | 0b11
    d.push(0xC3);
    d.extend_from_slice(&QUIC_VERSION_V1.to_be_bytes());
    d.push(dcid_len);
    d.extend_from_slice(&rng.bytes(dcid_len as usize));
    d.push(scid_len);
    d.extend_from_slice(&rng.bytes(scid_len as usize));
    // token length: varint 0
    d.push(0x00);
    // length varint covers packet number + ciphertext; the varint itself is
    // 2 bytes (0b01xxxxxxxxxxxxxx), so the remainder is what follows it.
    let remainder = target - d.len() - 2;
    let len_varint = 0x4000 | remainder as u16;
    d.extend_from_slice(&len_varint.to_be_bytes());
    // packet number (4 bytes) + ciphertext fill to exactly `target` bytes.
    d.extend_from_slice(&rng.bytes(remainder));
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::rand_bytes::Counter;

    #[test]
    fn crafted_initial_detected() {
        let d = craft_quic_initial(&mut Counter(0));
        assert_eq!(d.len(), 1200);
        assert!(looks_like_quic_initial(&d));
        let dcid = extract_dcid(&d).unwrap();
        assert_eq!(dcid.len(), 8);
    }

    #[test]
    fn rejects_version_negotiation_and_short() {
        assert!(!looks_like_quic_initial(&[0xC0, 0, 0, 0, 0, 8]));
        assert!(!looks_like_quic_initial(&[0xC3, 0, 0, 0, 1]));
    }

    #[test]
    fn rejects_short_header() {
        let mut d = craft_quic_initial(&mut Counter(1));
        d[0] &= !0x80;
        assert!(!looks_like_quic_initial(&d));
    }

    #[test]
    fn quic_v2_initial_is_detected_and_the_fixed_bit_is_required() {
        assert!(looks_like_quic_initial(&[0xC3, 0x6b, 0x33, 0x43, 0xCF, 8]));
        assert!(!looks_like_quic_initial(&[0x83, 0, 0, 0, 1, 8]));
    }
}
