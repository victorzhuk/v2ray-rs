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
//!   that stops parsing at offset 0 (seqovl desync), arrives out of order, or
//!   outgrows the 4 KiB decision window makes the emulator give up and pass
//!   (DPI timeout).
//!
//! Additionally `--cid-hex` drops QUIC Initial datagrams whose DCID matches.
//! The per-flow table mirrors the engine's: at most 8192 resident entries,
//! idle ones dropped after 60 s and the oldest quarter evicted under
//! pressure; the stateless per-packet mode creates no entries.
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

/// Resident per-flow entries; mirrors the engine's table cap.
const MAX_FLOWS: usize = 8192;

/// How long a flow entry survives without packets.
const FLOW_TTL: Duration = Duration::from_secs(60);

/// Fraction of MAX_FLOWS removed when pressure persists past the idle pass.
const EVICT_FRACTION: usize = 4;

/// Largest reassembly window inspected; the existing decision limit.
const REASSEMBLE_CAP: usize = 4096;

/// Per-flow censor state.
struct FlowState {
    /// Flow judged blocked; every further data packet is dropped.
    blocked: bool,
    /// Flow judged allowed (latch / reassemble decided).
    allowed: bool,
    /// reassemble mode: stream buffer in arrival order.
    stream: Vec<u8>,
    packets: u32,
    /// Last packet seen for this flow; drives idle eviction.
    last_seen: Instant,
}

impl Default for FlowState {
    fn default() -> Self {
        Self {
            blocked: false,
            allowed: false,
            stream: Vec::new(),
            packets: 0,
            last_seen: Instant::now(),
        }
    }
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
        // per-packet (and any unknown mode) judges each segment on its own
        // and remembers nothing, so no per-flow entry is created.
        if !matches!(self.mode.as_str(), "latch" | "reassemble") {
            if self.mode.as_str() == "per-packet"
                && let Some(sni) = full_client_hello_sni(data)
                && sni_blocked(&self.block, &sni)
            {
                eprintln!("censor: per-packet block sni={sni}");
                return Verdict::Drop;
            }
            return Verdict::Accept;
        }
        // Enforce the cap at admission, not only in the main-loop pass:
        // a fresh flow at pressure evicts first (idle entries, then the
        // oldest quarter).
        if !self.flows.contains_key(&key) && self.flows.len() >= MAX_FLOWS {
            self.maybe_evict();
        }
        let state = self.flows.entry(key).or_default();
        state.last_seen = Instant::now();
        if state.blocked {
            return Verdict::Drop;
        }
        if state.allowed {
            return Verdict::Accept;
        }
        match self.mode.as_str() {
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
                // Bounded window: input past the decision limit is neither
                // buffered nor inspected; at most the remaining prefix is.
                let room = REASSEMBLE_CAP.saturating_sub(state.stream.len());
                let take = data.len().min(room);
                if take > 0 {
                    // Reserve exactly: amortized growth could round the
                    // retained buffer's allocation past the window.
                    if state.stream.len() + take > state.stream.capacity() {
                        state.stream.reserve_exact(take);
                    }
                    state.stream.extend_from_slice(&data[..take]);
                }
                state.packets += 1;
                let sni = full_client_hello_sni(&state.stream);
                if let Some(sni) = sni {
                    let verdict = if sni_blocked(&self.block, &sni) {
                        eprintln!("censor: reassemble block sni={sni}");
                        state.blocked = true;
                        Verdict::Drop
                    } else {
                        state.allowed = true;
                        Verdict::Accept
                    };
                    // The verdict is final and the buffered prefix is never
                    // read again: release its allocation.
                    state.stream = Vec::new();
                    return verdict;
                }
                if state.packets > 4 || state.stream.len() >= REASSEMBLE_CAP {
                    // Unparseable stream (seqovl desync / out-of-order
                    // arrival / window exhausted): give up like a DPI that
                    // times out.
                    eprintln!("censor: reassemble gave up, passing flow");
                    state.allowed = true;
                    state.stream = Vec::new();
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

    /// Bounds the flow table: idle entries go first, then the oldest ones,
    /// mirroring the engine's eviction policy.
    fn maybe_evict(&mut self) {
        if self.flows.len() < MAX_FLOWS {
            return;
        }
        self.flows.retain(|_, f| f.last_seen.elapsed() <= FLOW_TTL);
        if self.flows.len() >= MAX_FLOWS {
            let mut entries: Vec<(FlowKey, Instant)> =
                self.flows.iter().map(|(k, f)| (*k, f.last_seen)).collect();
            entries.sort_by_key(|(_, seen)| *seen);
            let drop_count = MAX_FLOWS / EVICT_FRACTION;
            for (key, _) in entries.into_iter().take(drop_count) {
                self.flows.remove(&key);
            }
        }
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
                guard.maybe_evict();
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

// Storage-level checks for the fixture's own bookkeeping: synthetic flow
// keys pushed straight through the table and the reassembly window, no
// NFQUEUE or network involved.
#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_antidpi::tls::craft_client_hello;
    use v2ray_rs_antidpi::tls::rand_bytes::Counter;

    fn censor(mode: &str) -> Censor {
        Censor {
            mode: mode.to_string(),
            block: ["blocked.example"]
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            cids: HashSet::new(),
            flows: HashMap::new(),
        }
    }

    fn key(n: u16) -> FlowKey {
        FlowKey::new(6, ([10, 0, 0, 1], n), ([10, 0, 0, 2], 443))
    }

    fn hello(sni: &str) -> Vec<u8> {
        craft_client_hello(sni, &mut Counter(0)).expect("crafted client hello")
    }

    #[test]
    fn stateless_mode_keeps_no_flow_entries() {
        let mut c = censor("per-packet");
        assert_eq!(
            c.handle_tcp(key(1), &hello("blocked.example")),
            Verdict::Drop
        );
        assert_eq!(c.handle_tcp(key(2), b"junk segment"), Verdict::Accept);
        assert!(c.flows.is_empty(), "stateless mode must not create entries");
        c.maybe_evict();

        let mut unknown = censor("bogus");
        assert_eq!(unknown.handle_tcp(key(3), b"junk segment"), Verdict::Accept);
        assert_eq!(
            unknown.handle_tcp(key(4), &hello("blocked.example")),
            Verdict::Accept
        );
        assert!(unknown.flows.is_empty());
    }

    #[test]
    fn eviction_drops_idle_then_oldest_entries() {
        let mut c = censor("latch");
        let total = MAX_FLOWS + 512;
        // Inserted directly: this test pins maybe_evict semantics, while
        // admission-time enforcement is covered separately below.
        for i in 0..total {
            c.flows.insert(key(i as u16 + 1), FlowState::default());
        }
        assert_eq!(c.flows.len(), total, "all synthetic flows are resident");

        // Half the flows idle past the TTL: the idle pass alone frees them.
        let base = Instant::now();
        for (n, k) in c
            .flows
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            let seen = if n % 2 == 0 {
                base - FLOW_TTL - Duration::from_secs(1)
            } else {
                base - Duration::from_secs(30)
            };
            c.flows.get_mut(&k).unwrap().last_seen = seen;
        }
        c.maybe_evict();
        assert_eq!(c.flows.len(), total / 2, "only fresh entries remain");
        assert!(c.flows.values().all(|f| f.last_seen.elapsed() <= FLOW_TTL));

        // Refill past the cap and stagger last_seen: the oldest quarter of
        // everything resident goes, regardless of key order.
        for i in total..(total + total / 2) {
            c.flows.insert(key(i as u16 + 1), FlowState::default());
        }
        assert!(c.flows.len() > MAX_FLOWS);
        let base = Instant::now() - Duration::from_secs(30);
        for (n, k) in c
            .flows
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            c.flows.get_mut(&k).unwrap().last_seen = base + Duration::from_nanos(n as u64);
        }
        c.maybe_evict();
        assert_eq!(
            c.flows.len(),
            MAX_FLOWS + (total - MAX_FLOWS) - MAX_FLOWS / EVICT_FRACTION,
            "oldest quarter dropped once the cap is exceeded"
        );
        let cutoff = base + Duration::from_nanos((MAX_FLOWS / EVICT_FRACTION) as u64);
        assert!(
            c.flows.values().all(|f| f.last_seen >= cutoff),
            "only entries newer than the evicted oldest quarter remain"
        );
        assert!(c.flows.len() <= MAX_FLOWS, "table stays bounded");
    }

    #[test]
    fn admission_enforces_the_cap_and_evicts_idle_then_oldest() {
        let mut c = censor("latch");

        // More than MAX_FLOWS distinct synthetic flows: the invariant
        // holds after every single admission, before any main-loop pass.
        let total = MAX_FLOWS + 1024;
        for i in 1..=total as u16 {
            assert_eq!(c.handle_tcp(key(i), b"junk segment"), Verdict::Accept);
            assert!(
                c.flows.len() <= MAX_FLOWS,
                "resident cap violated after admission {i}"
            );
        }

        // Age half the residents past the TTL, then push enough new
        // flows across the cap: the idle pass must serve the admission
        // before any fresh resident is touched.
        let base = Instant::now();
        let mut fresh_keys = HashSet::new();
        for (n, k) in c
            .flows
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            let seen = if n % 2 == 0 {
                base - FLOW_TTL - Duration::from_secs(1)
            } else {
                fresh_keys.insert(k);
                base - Duration::from_secs(30)
            };
            c.flows.get_mut(&k).unwrap().last_seen = seen;
        }
        let more = MAX_FLOWS / 2;
        let first_new = total as u16 + 1;
        for port in first_new..first_new + more as u16 {
            assert_eq!(c.handle_tcp(key(port), b"junk segment"), Verdict::Accept);
            assert!(
                c.flows.len() <= MAX_FLOWS,
                "resident cap violated after admission {port}"
            );
        }
        assert!(
            c.flows.values().all(|f| f.last_seen.elapsed() <= FLOW_TTL),
            "idle entries were reclaimed for admission before fresh ones"
        );
        for k in &fresh_keys {
            assert!(
                c.flows.contains_key(k),
                "fresh resident evicted although idle entries were available"
            );
        }

        // No idle entries remain: the next pressure crossing must evict
        // exactly the oldest quarter, and the flow that triggered it
        // must be resident afterwards.
        let base = Instant::now() - Duration::from_secs(30);
        let mut by_age: Vec<(FlowKey, Instant)> =
            c.flows.iter().map(|(k, f)| (*k, f.last_seen)).collect();
        by_age.sort_by_key(|(_, seen)| *seen);
        for (n, (k, _)) in by_age.iter().enumerate() {
            c.flows.get_mut(k).unwrap().last_seen = base + Duration::from_nanos(n as u64);
        }
        let oldest: HashSet<FlowKey> = by_age
            .into_iter()
            .take(MAX_FLOWS / EVICT_FRACTION)
            .map(|(k, _)| k)
            .collect();
        let mut port = first_new + more as u16;
        while oldest.iter().any(|k| c.flows.contains_key(k)) {
            assert_eq!(c.handle_tcp(key(port), b"junk segment"), Verdict::Accept);
            assert!(
                c.flows.len() <= MAX_FLOWS,
                "resident cap violated after admission {port}"
            );
            port += 1;
        }
        assert!(
            oldest.iter().all(|k| !c.flows.contains_key(k)),
            "the oldest quarter was evicted once no idle entry remained"
        );
        assert!(
            c.flows.contains_key(&key(port - 1)),
            "the admission that triggered eviction is resident"
        );
    }

    #[test]
    fn reassemble_pending_capacity_stays_within_the_window() {
        let mut c = censor("reassemble");
        // A 4000-byte pending window forces a reallocation that amortized
        // growth would round to 6000; the retained buffer must stay
        // within the decision limit.
        let filler = vec![0xa5u8; 3000];
        assert_eq!(c.handle_tcp(key(1), &filler), Verdict::Accept);
        let filler = vec![0x5au8; 1000];
        assert_eq!(c.handle_tcp(key(1), &filler), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert_eq!(st.stream.len(), 4000, "stream is pending near the cap");
        assert!(
            st.stream.capacity() <= REASSEMBLE_CAP,
            "pending capacity must not round past the window"
        );

        // Approaching the limit with a near-exact segment stays bounded.
        assert_eq!(c.handle_tcp(key(1), &[0xffu8; 95]), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert_eq!(st.stream.len(), REASSEMBLE_CAP - 1);
        assert!(st.stream.capacity() <= REASSEMBLE_CAP);

        // The byte that exhausts the window takes the existing
        // undecidable outcome and releases the buffer.
        assert_eq!(c.handle_tcp(key(1), b"x"), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert!(st.allowed, "window exhaustion passes the flow");
        assert_eq!(st.stream.capacity(), 0, "give-up clears the buffer");
    }

    #[test]
    fn latch_decides_first_hello_and_keeps_verdict() {
        let mut c = censor("latch");
        assert_eq!(
            c.handle_tcp(key(1), &hello("allowed.example")),
            Verdict::Accept
        );
        assert!(c.flows.get(&key(1)).unwrap().allowed);
        assert_eq!(
            c.handle_tcp(key(1), &hello("blocked.example")),
            Verdict::Accept,
            "allowed latch sticks"
        );
        assert_eq!(c.handle_tcp(key(1), b"more"), Verdict::Accept);

        let mut c = censor("latch");
        assert_eq!(
            c.handle_tcp(key(2), &hello("blocked.example")),
            Verdict::Drop
        );
        assert!(c.flows.get(&key(2)).unwrap().blocked);
        assert_eq!(c.handle_tcp(key(2), b"more"), Verdict::Drop);
    }

    #[test]
    fn reassemble_decides_small_flows_and_releases_the_buffer() {
        let mut c = censor("reassemble");
        let ch = hello("blocked.example");
        let (head, tail) = ch.split_at(ch.len() / 2);
        assert_eq!(c.handle_tcp(key(1), head), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert!(!st.allowed, "partial stream stays under inspection");
        assert_eq!(c.handle_tcp(key(1), tail), Verdict::Drop);
        let st = c.flows.get(&key(1)).unwrap();
        assert!(st.blocked);
        assert_eq!(
            st.stream.capacity(),
            0,
            "terminal decision clears the buffer"
        );
        assert_eq!(c.handle_tcp(key(1), b"more"), Verdict::Drop);

        let mut c = censor("reassemble");
        assert_eq!(
            c.handle_tcp(key(2), &hello("allowed.example")),
            Verdict::Accept
        );
        let st = c.flows.get(&key(2)).unwrap();
        assert!(st.allowed);
        assert_eq!(st.stream.capacity(), 0);
    }

    #[test]
    fn reassemble_caps_the_window_and_oversize_passes() {
        let mut c = censor("reassemble");
        let filler = vec![0xa5u8; 3000];
        assert_eq!(c.handle_tcp(key(1), &filler), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert_eq!(st.stream.len(), 3000);
        assert!(!st.allowed, "still inside the window, still inspecting");

        // The next segment overruns the window: only the remaining 1096
        // bytes are inspected, then the existing undecidable outcome
        // applies (two packets cannot trigger the packet-count give-up).
        let filler = vec![0x5au8; 5000];
        assert_eq!(c.handle_tcp(key(1), &filler), Verdict::Accept);
        let st = c.flows.get(&key(1)).unwrap();
        assert!(
            st.allowed,
            "input past the window is undecidable and passes"
        );
        assert_eq!(st.stream.capacity(), 0, "give-up clears the buffer");
        assert_eq!(
            c.handle_tcp(key(1), b"more"),
            Verdict::Accept,
            "decided flow short-circuits"
        );
    }
}
