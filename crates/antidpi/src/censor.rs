//! `v2ray-rs-antidpi-censor` — test-only censor emulator, a miniature TSPU.
//!
//! Models three DPI behaviors against a blocklist, selected by `--mode`:
//!
//! - `per-packet`: a complete ClientHello record inside one segment with a
//!   blocked SNI ⇒ drop that segment.
//! - `latch`: the first ClientHello on a flow decides the flow's verdict
//!   (defeated by a decoy carrying an allowed SNI).
//! - `reassemble`: naive in-order stream reassembly; a complete ClientHello
//!   parsed at stream offset 0 with a blocked SNI ⇒ block the flow. A stream
//!   that stops parsing at offset 0 (seqovl desync) or arrives out of order
//!   makes the emulator give up and pass (DPI timeout).
//!
//! Additionally `--cid-hex` drops QUIC Initial datagrams whose DCID matches.
//! Used only by `tests/privileged.rs` inside throwaway network namespaces.

use clap::Parser;
use nfq::{Queue, Verdict};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use v2ray_rs_antidpi::flow::FlowKey;
use v2ray_rs_antidpi::packet::Packet;
use v2ray_rs_antidpi::{quic, tls};

#[derive(Parser)]
struct Args {
    /// NFQUEUE number.
    #[arg(long, default_value_t = 201)]
    queue: u16,
    /// Censor behavior model: per-packet | latch | reassemble
    #[arg(long, default_value = "per-packet")]
    mode: String,
    /// Blocked hostnames (repeatable).
    #[arg(long = "block")]
    block: Vec<String>,
    /// QUIC destination CIDs to drop (hex, repeatable).
    #[arg(long = "cid-hex")]
    cid_hex: Vec<String>,
    /// Seconds to run (0 = forever).
    #[arg(long, default_value_t = 0)]
    for_secs: u64,
}

/// Per-flow censor state.
#[derive(Default)]
struct FlowState {
    /// Flow judged blocked; every further data packet is dropped.
    blocked: bool,
    /// Flow judged allowed (latch / reassemble decided).
    allowed: bool,
    /// reassemble mode: stream buffer in arrival order.
    stream: Vec<u8>,
    packets: u32,
}

/// The censor: dispatches per-mode on each queued packet.
struct Censor {
    mode: String,
    block: HashSet<String>,
    cids: HashSet<Vec<u8>>,
    flows: HashMap<FlowKey, FlowState>,
}

impl Censor {
    fn handle_tcp(&mut self, key: FlowKey, data: &[u8]) -> Verdict {
        if data.is_empty() {
            return Verdict::Accept;
        }
        let state = self.flows.entry(key).or_default();
        if state.blocked {
            return Verdict::Drop;
        }
        if state.allowed {
            return Verdict::Accept;
        }
        match self.mode.as_str() {
            "per-packet" => {
                if let Some(sni) = full_client_hello_sni(data)
                    && sni_blocked(&self.block, &sni)
                {
                    eprintln!("censor: per-packet block sni={sni}");
                    return Verdict::Drop;
                }
                Verdict::Accept
            }
            "latch" => {
                if let Some(sni) = full_client_hello_sni(data) {
                    if sni_blocked(&self.block, &sni) {
                        eprintln!("censor: latch block sni={sni}");
                        state.blocked = true;
                        return Verdict::Drop;
                    }
                    state.allowed = true;
                }
                Verdict::Accept
            }
            "reassemble" => {
                state.stream.extend_from_slice(data);
                state.packets += 1;
                let sni = full_client_hello_sni(&state.stream);
                if let Some(sni) = sni {
                    if sni_blocked(&self.block, &sni) {
                        eprintln!("censor: reassemble block sni={sni}");
                        state.blocked = true;
                        return Verdict::Drop;
                    }
                    state.allowed = true;
                    return Verdict::Accept;
                }
                if state.packets > 4 || state.stream.len() > 4096 {
                    // Unparseable stream (seqovl desync / out-of-order
                    // arrival): give up like a DPI that times out.
                    eprintln!("censor: reassemble gave up, passing flow");
                    state.allowed = true;
                }
                Verdict::Accept
            }
            _ => Verdict::Accept,
        }
    }

    fn handle_udp(&mut self, data: &[u8]) -> Verdict {
        if data.is_empty() {
            return Verdict::Accept;
        }
        if quic::looks_like_quic_initial(data)
            && let Some(dcid) = quic::extract_dcid(data)
            && self.cids.contains(dcid)
        {
            eprintln!("censor: quic initial drop by cid");
            return Verdict::Drop;
        }
        Verdict::Accept
    }
}

/// SNI when `data` holds a complete ClientHello record. Returns an owned
/// string since it is parsed from a local slice.
fn full_client_hello_sni(data: &[u8]) -> Option<String> {
    if !tls::looks_like_client_hello(data) {
        return None;
    }
    let len = tls::client_hello_len(data)?;
    tls::extract_sni(&data[..len])
}

/// Suffix-aware blocklist match against a free function so it can borrow
/// `block` while the per-flow entry is mutably borrowed.
fn sni_blocked(block: &HashSet<String>, sni: &str) -> bool {
    block.iter().any(|b| {
        sni.eq_ignore_ascii_case(b) || sni.to_ascii_lowercase().ends_with(&format!(".{b}"))
    })
}

fn decode_hex(h: &str) -> Option<Vec<u8>> {
    let h = h.trim().trim_start_matches("0x").replace('_', "");
    if h.is_empty() || !h.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(h.len() / 2);
    for i in 0..h.len() / 2 {
        let byte = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

fn is_would_block(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::WouldBlock
}

fn main() {
    let args = Args::parse();
    let censor = std::sync::Mutex::new(Censor {
        mode: args.mode.clone(),
        block: args.block.iter().map(|s| s.to_ascii_lowercase()).collect(),
        cids: args.cid_hex.iter().filter_map(|h| decode_hex(h)).collect(),
        flows: HashMap::new(),
    });
    let deadline = if args.for_secs > 0 {
        Some(Instant::now() + Duration::from_secs(args.for_secs))
    } else {
        None
    };

    let mut queue = Queue::open().expect("censor: nfqueue open");
    queue.bind(args.queue).expect("censor: nfqueue bind");
    queue.set_nonblocking(true);
    eprintln!(
        "censor: mode={} queue={} block={:?} cids={}",
        args.mode,
        args.queue,
        args.block,
        args.cid_hex.len()
    );

    loop {
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            break;
        }
        match queue.recv() {
            Ok(msg) => {
                let payload = msg.get_payload().to_vec();
                let mut guard = censor.lock().unwrap();
                let verdict = match Packet::parse(&payload) {
                    Ok(pkt) => {
                        let key = match (&pkt.tcp, &pkt.udp) {
                            (Some(t), _) => {
                                Some(FlowKey::new(6, (pkt.src, t.sport), (pkt.dst, t.dport)))
                            }
                            (_, Some(u)) => {
                                Some(FlowKey::new(17, (pkt.src, u.sport), (pkt.dst, u.dport)))
                            }
                            _ => None,
                        };
                        match key {
                            Some(k) if pkt.udp.is_some() => {
                                let data = pkt.udp.as_ref().unwrap().payload;
                                guard.handle_udp(data)
                            }
                            Some(k) => {
                                let data = pkt.tcp.as_ref().unwrap().payload;
                                guard.handle_tcp(k, data)
                            }
                            None => Verdict::Accept,
                        }
                    }
                    Err(_) => Verdict::Accept,
                };
                drop(guard);
                let mut msg = msg;
                msg.set_verdict(verdict);
                queue.verdict(msg).expect("censor: verdict");
            }
            Err(e) => {
                if is_would_block(&e) {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                eprintln!("censor: queue error: {e:?}");
                std::process::exit(2);
            }
        }
    }
}
