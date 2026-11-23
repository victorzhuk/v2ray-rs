//! Per-flow state and the first-packet staging decision.
//!
//! Original traffic is always preferred. A staged drop is possible only
//! when a single queued packet carries the complete L7 message a strategy
//! chain will replace in full. Bursts that span several packets, start
//! with an incomplete record, exceed the size cap, or carry extra data
//! beyond the record are ambiguous: their originals pass through and the
//! flow stops being inspected. No burst buffer is retained, so there is
//! nothing to leak when inspection is skipped. Retransmissions are never
//! dropped on the basis of an attempted replacement; the engine accepts
//! the original whenever a replacement write fails or falls short.

use std::time::Instant;

/// Canonical flow key: endpoints sorted so reply-direction packets map to
/// the same entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlowKey {
    pub proto: u8,
    pub lo: ([u8; 4], u16),
    pub hi: ([u8; 4], u16),
}

impl FlowKey {
    pub fn new(proto: u8, a: ([u8; 4], u16), b: ([u8; 4], u16)) -> FlowKey {
        if a <= b {
            FlowKey {
                proto,
                lo: a,
                hi: b,
            }
        } else {
            FlowKey {
                proto,
                lo: b,
                hi: a,
            }
        }
    }
}

/// L7 message type detected on the outgoing burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L7 {
    TlsClientHello,
    HttpRequest,
    QuicInitial,
    Unknown,
}

/// Result of ingesting an outgoing TCP data packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Burst {
    /// One queued packet carries the complete L7 message. This is the only
    /// staged-drop window: the engine may drop the packet after its
    /// replacement writes covered the entire original payload; otherwise
    /// it accepts the original.
    Fire(L7, Vec<u8>),
    /// Pass the original through and stop inspecting this flow.
    Abort,
}

#[derive(Debug)]
pub struct Flow {
    pub reply_ttl: Option<u8>,
    /// The chain was attempted for this flow; further packets (including
    /// retransmissions of the staged range) pass through.
    pub fired: bool,
    /// Inspection gave up or was not applicable; everything passes.
    pub dead: bool,
    /// Last packet seen for this flow; drives idle eviction.
    pub last_seen: Instant,
    /// The reply RST of this flow already counted as a rotation failure.
    pub failure_counted: bool,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            reply_ttl: None,
            fired: false,
            dead: false,
            last_seen: Instant::now(),
            failure_counted: false,
        }
    }
}

/// Largest first packet considered for staging. The bound is enforced
/// before any copy, so oversized payloads are parsed in place and pass
/// through.
const REASM_CAP: usize = 16 * 1024;

/// Cheap HTTP request-head probe: a known method verb at the start.
fn is_http_request_head(payload: &[u8]) -> bool {
    payload.starts_with(b"GET ")
        || payload.starts_with(b"POST ")
        || payload.starts_with(b"HEAD ")
        || payload.starts_with(b"PUT ")
        || payload.starts_with(b"DELETE ")
        || payload.starts_with(b"OPTIONS ")
}

impl Flow {
    /// Ingest one outgoing TCP data packet. The payload is borrowed for
    /// parsing and copied only when the burst can be staged for
    /// replacement.
    pub fn ingest_tcp(&mut self, payload: &[u8]) -> Burst {
        if self.fired || self.dead {
            return Burst::Abort;
        }
        if payload.is_empty() {
            return Burst::Abort;
        }
        if payload.len() > REASM_CAP {
            self.dead = true;
            return Burst::Abort;
        }
        let tls = crate::tls::looks_like_client_hello(payload);
        let complete = if tls {
            // Only a record that fits entirely in this packet can be
            // replaced; a record continuing into later packets makes the
            // burst multi-packet, and those originals must pass.
            crate::tls::client_hello_len(payload) == Some(payload.len())
        } else {
            is_http_request_head(payload) && crate::positions::http_host_range(payload).is_some()
        };
        if !complete {
            self.dead = true;
            return Burst::Abort;
        }
        let l7 = if tls {
            L7::TlsClientHello
        } else {
            L7::HttpRequest
        };
        Burst::Fire(l7, payload.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::{craft_client_hello, rand_bytes::Counter};

    #[test]
    fn single_packet_client_hello_is_stageable() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut f = Flow::default();
        assert_eq!(
            f.ingest_tcp(&ch),
            Burst::Fire(L7::TlsClientHello, ch.clone())
        );
        assert!(!f.dead);
    }

    #[test]
    fn split_record_passes_originals_through() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut f = Flow::default();
        // Incomplete first segment: never staged, and the burst is now
        // multi-packet, so every original passes.
        assert_eq!(f.ingest_tcp(&ch[..200]), Burst::Abort);
        assert!(f.dead);
        assert_eq!(f.ingest_tcp(&ch[200..]), Burst::Abort);
    }

    #[test]
    fn retransmission_after_fire_passes() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut f = Flow::default();
        assert!(matches!(f.ingest_tcp(&ch), Burst::Fire(..)));
        // A TCP retransmission of the staged range must never be dropped
        // on the basis of an attempted replacement.
        f.fired = true;
        assert_eq!(f.ingest_tcp(&ch), Burst::Abort);
    }

    #[test]
    fn coalesced_record_passes_through() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut coalesced = ch.clone();
        coalesced.extend_from_slice(b"\x17\x03\x03\x00\x04tail");
        let mut f = Flow::default();
        // A replacement would cover only the record, so the packet with
        // the extra tail passes through whole.
        assert_eq!(f.ingest_tcp(&coalesced), Burst::Abort);
        assert!(f.dead);
    }

    #[test]
    fn oversized_first_packet_passes_through() {
        let mut f = Flow::default();
        let big = vec![0u8; REASM_CAP + 1];
        assert_eq!(f.ingest_tcp(&big), Burst::Abort);
        assert!(f.dead);
    }

    #[test]
    fn non_l7_first_packet_stops_inspection() {
        let mut f = Flow::default();
        assert_eq!(
            f.ingest_tcp(b"\x17\x03\x03\x00\x10garbagegarbage"),
            Burst::Abort
        );
        assert!(f.dead);
    }

    #[test]
    fn http_request_with_host_is_stageable() {
        let req = b"GET / HTTP/1.1\r\nHost: example.org\r\n\r\n".as_slice();
        let mut f = Flow::default();
        assert_eq!(
            f.ingest_tcp(req),
            Burst::Fire(L7::HttpRequest, req.to_vec())
        );
    }

    #[test]
    fn http_head_without_host_passes_through() {
        let req = b"GET / HTTP/1.1\r\n\r\n".as_slice();
        let mut f = Flow::default();
        assert_eq!(f.ingest_tcp(req), Burst::Abort);
        assert!(f.dead);
    }

    #[test]
    fn flow_key_canonical() {
        let a = FlowKey::new(6, ([1, 2, 3, 4], 5000), ([9, 9, 9, 9], 443));
        let b = FlowKey::new(6, ([9, 9, 9, 9], 443), ([1, 2, 3, 4], 5000));
        assert_eq!(a, b);
    }
}
