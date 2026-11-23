//! NFQUEUE engine: receives queued packets, maintains the flow table, and
//! executes strategy chains (fake / multisplit / multidisorder with seqovl),
//! with circular rotation between chains on failure signals.
//!
//! Original traffic is preferred: the queued packet is dropped only when a
//! replacement write covered the entire original payload and every write
//! succeeded; otherwise the original is accepted. TCP retransmissions are
//! never dropped. Shutdown stays fail-open, and a forced queue destruction
//! must not be assumed to be lossless.

use crate::error::Result;
use crate::flow::{Burst, Flow, FlowKey, L7};
use crate::packet::{OutBuilder, Packet};
use crate::rawsend::RawSender;
use crate::strategy::{Attack, Blob, Chain};
use crate::tls::rand_bytes::{OsRng, Rng};
use nfq::{Queue, Verdict};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

/// fwmark stamped on re-injected packets; the queue rules exclude it.
pub const DEFAULT_FWMARK: u32 = 0x4000_0000;

/// One stdout readiness line, printed right after the NFQUEUE bind and the
/// readiness callback both succeeded: a supervisor waits for
/// `<prefix><queue>` before treating the helper as up.
pub const READY_SENTINEL_PREFIX: &str = "v2ray-rs-antidpi: ready queue=";

const MAX_FLOWS: usize = 8192;

/// How long a flow entry survives without packets.
const FLOW_TTL: Duration = Duration::from_secs(60);

/// Fraction of MAX_FLOWS removed when pressure persists past the idle pass.
const EVICT_FRACTION: usize = 4;

/// Circular orchestrator settings.
#[derive(Debug, Clone)]
pub struct Rotation {
    pub fails_threshold: u32,
    pub window: Duration,
}

pub struct Engine {
    flows: HashMap<FlowKey, Flow>,
    tcp_chains: Vec<Chain>,
    udp_chains: Vec<Chain>,
    tcp_chain_idx: usize,
    udp_chain_idx: usize,
    failures: u32,
    window_start: Instant,
    rotation: Option<Rotation>,
    /// Raw re-inject socket; `None` only in tests, where every replacement
    /// write counts as failed and the original is accepted.
    sender: Option<RawSender>,
    fwmark: u32,
    rng: OsRng,
    /// When set, only fire for these hostnames. Entries are lowercased once
    /// at load; exact names and label-delimited suffixes match without
    /// per-comparison allocation.
    hosts: Option<HashSet<Vec<u8>>>,
}

/// Splits one chain list by target protocol, keeping the original order.
fn partition_chains(chains: Vec<Chain>) -> (Vec<Chain>, Vec<Chain>) {
    let tcp_chains: Vec<Chain> = chains
        .iter()
        .filter(|c| c.applies_to_tcp())
        .cloned()
        .collect();
    let udp_chains: Vec<Chain> = chains
        .iter()
        .filter(|c| c.applies_to_udp())
        .cloned()
        .collect();
    (tcp_chains, udp_chains)
}

/// Lowercase, strip surrounding dots, and deduplicate the hostlist once at
/// load.
fn normalize_hosts(list: Vec<String>) -> HashSet<Vec<u8>> {
    list.into_iter()
        .map(|h| {
            let h = h.to_ascii_lowercase();
            h.trim_matches('.').as_bytes().to_vec()
        })
        .filter(|h| !h.is_empty())
        .collect()
}

/// Exact-name and label-delimited suffix match against the normalized
/// allow set, without per-entry allocation.
fn host_matches(set: &HashSet<Vec<u8>>, host: &[u8]) -> bool {
    (0..host.len())
        .filter(|&i| i == 0 || host[i - 1] == b'.')
        .any(|i| set.contains(&host[i..]))
}

impl Engine {
    pub fn new(
        chains: Vec<Chain>,
        rotation: Option<Rotation>,
        hosts: Option<Vec<String>>,
    ) -> Result<Engine> {
        if chains.is_empty() {
            return Err(crate::error::Error::Strategy(
                "at least one strategy chain is required".into(),
            ));
        }
        let hosts = hosts.map(normalize_hosts);
        let sender = RawSender::new(DEFAULT_FWMARK)?;
        Self::with_sender(chains, rotation, hosts, Some(sender))
    }

    fn with_sender(
        chains: Vec<Chain>,
        rotation: Option<Rotation>,
        hosts: Option<HashSet<Vec<u8>>>,
        sender: Option<RawSender>,
    ) -> Result<Engine> {
        let (tcp_chains, udp_chains) = partition_chains(chains);
        Ok(Engine {
            flows: HashMap::new(),
            tcp_chains,
            udp_chains,
            tcp_chain_idx: 0,
            udp_chain_idx: 0,
            failures: 0,
            window_start: Instant::now(),
            rotation,
            sender,
            fwmark: DEFAULT_FWMARK,
            rng: OsRng,
            hosts,
        })
    }

    /// Gets or inserts the flow entry and stamps it as just seen.
    fn touch(&mut self, key: FlowKey) -> &mut Flow {
        let entry = self.flows.entry(key).or_default();
        entry.last_seen = Instant::now();
        entry
    }

    /// Blocking NFQUEUE loop; returns only on error.
    pub fn run(&mut self, queue_num: u16) -> Result<()> {
        self.run_interruptible(queue_num, || false)
    }

    /// NFQUEUE loop with a shutdown poll; non-blocking recv with a small
    /// sleep so SIGINT/SIGTERM are honored promptly. Packets still queued
    /// when the loop ends are released by the kernel: fail-open, without a
    /// zero-loss promise.
    pub fn run_interruptible(&mut self, queue_num: u16, shutdown: impl Fn() -> bool) -> Result<()> {
        self.run_interruptible_with_ready(queue_num, shutdown, Self::publish_ready)
    }

    /// Prints and flushes the READY sentinel: the single point where this
    /// engine announces itself as up.
    pub fn publish_ready(queue_num: u16) -> Result<()> {
        println!("{READY_SENTINEL_PREFIX}{queue_num}");
        let _ = std::io::stdout().flush();
        Ok(())
    }

    /// Like [`Self::run_interruptible`], but `publish_ready` owns readiness
    /// publication after the queue bind: the engine is announced as up only
    /// when this closure returns `Ok`, and any work tied to publication —
    /// such as releasing a startup transaction lock — must happen inside it,
    /// after the READY sentinel is out.
    ///
    /// The socket is fail-open: queued packets the kernel cannot deliver are
    /// accepted by the kernel, never held past the queue's loss budget. The
    /// loop polls the fd so shutdown signals are honored promptly.
    pub fn run_interruptible_with_ready(
        &mut self,
        queue_num: u16,
        shutdown: impl Fn() -> bool,
        publish_ready: impl FnOnce(u16) -> Result<()>,
    ) -> Result<()> {
        let mut queue = Queue::open().map_err(|e| crate::error::Error::Queue(e.to_string()))?;
        queue
            .bind(queue_num)
            .map_err(|e| crate::error::Error::Queue(e.to_string()))?;
        queue.set_fail_open(queue_num, true).map_err(|e| {
            crate::error::Error::Queue(format!("cannot enable fail-open on queue {queue_num}: {e}"))
        })?;
        publish_ready(queue_num)?;
        queue.set_nonblocking(true);
        log::info!("engine: listening on queue {queue_num}");
        loop {
            if shutdown() {
                log::info!("shutdown signal");
                return Ok(());
            }
            match queue.recv() {
                Ok(msg) => {
                    let outgoing = msg.get_outdev() != 0;
                    let nfmark = msg.get_nfmark();
                    // Parse the queue payload in place: no copy before the
                    // packet is understood.
                    let verdict = self.handle_packet(msg.get_payload(), outgoing, nfmark);
                    let mut msg = msg;
                    msg.set_verdict(verdict);
                    match queue.verdict(msg) {
                        Ok(()) => {}
                        Err(e)
                            if e.raw_os_error() == Some(libc::ENOSPC)
                                || e.raw_os_error() == Some(libc::ENOENT) =>
                        {
                            log::warn!("queue: skipped a message: {e}");
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(crate::error::Error::Queue(e.to_string())),
                    }
                    self.maybe_evict();
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e)
                    if e.raw_os_error() == Some(libc::ENOSPC)
                        || e.raw_os_error() == Some(libc::ENOENT) =>
                {
                    log::warn!("queue: skipped a message: {e}");
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let mut fds = [libc::pollfd {
                        fd: queue.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    }];
                    // A negative poll result ends this wait; the loop
                    // re-checks shutdown and retries on the next pass.
                    let _ = unsafe { libc::poll(fds.as_mut_ptr(), 1, 50) };
                }
                Err(e) => return Err(crate::error::Error::Queue(e.to_string())),
            }
        }
    }

    /// Probe whether this process holds the caps raw-send requires
    /// (cap_net_raw for the socket, cap_net_admin for SO_MARK).
    pub fn probe_caps() -> Result<()> {
        crate::rawsend::RawSender::new(DEFAULT_FWMARK).map(|_| ())
    }

    /// Classify one queued packet and decide its verdict. IPv6 and anything
    /// unparseable passes through unchanged.
    fn handle_packet(&mut self, payload: &[u8], outgoing: bool, nfmark: u32) -> Verdict {
        let Ok(pkt) = Packet::parse(payload) else {
            log::trace!("queued: pass unparseable ({} bytes)", payload.len());
            return Verdict::Accept;
        };
        if nfmark & self.fwmark != 0 {
            return Verdict::Accept;
        }
        let Some(key) = (match (&pkt.tcp, &pkt.udp) {
            (Some(t), _) => Some(FlowKey::new(6, (pkt.src, t.sport), (pkt.dst, t.dport))),
            (_, Some(u)) => Some(FlowKey::new(17, (pkt.src, u.sport), (pkt.dst, u.dport))),
            _ => None,
        }) else {
            return Verdict::Accept;
        };
        let proto = if pkt.tcp.is_some() { "tcp" } else { "udp" };
        log::trace!(
            "queued: {} {} {} -> {} ({} bytes)",
            if outgoing { "out" } else { "in" },
            proto,
            Ipv4Addr::from(pkt.src),
            Ipv4Addr::from(pkt.dst),
            payload.len()
        );

        if !outgoing {
            // Reply direction: record the reply TTL for autottl, count the
            // first RST of a fired flow as a rotation failure, otherwise pass.
            let entry = self.touch(key);
            if entry.reply_ttl.is_none() {
                entry.reply_ttl = Some(pkt.ttl);
            }
            if let Some(t) = &pkt.tcp
                && t.flags & crate::packet::TCP_FLAG_RST != 0
                && entry.fired
                && !entry.failure_counted
            {
                entry.failure_counted = true;
                self.note_failure(6);
            }
            return Verdict::Accept;
        }

        if pkt.is_syn() {
            return Verdict::Accept;
        }

        if let Some(t) = &pkt.tcp {
            if t.payload.is_empty() {
                return Verdict::Accept;
            }
            let burst = {
                let entry = self.touch(key);
                if entry.fired || entry.dead {
                    // Finished flows and retransmissions pass the original
                    // through; dropping a replay of a staged range would
                    // bet on an attempted replacement.
                    Burst::Abort
                } else {
                    entry.ingest_tcp(t.payload)
                }
            };
            match burst {
                Burst::Abort => Verdict::Accept,
                Burst::Fire(l7, data) => {
                    if !self.host_allowed(l7, &data) {
                        if let Some(entry) = self.flows.get_mut(&key) {
                            entry.dead = true;
                        }
                        log::debug!("host excluded; passing original through");
                        return Verdict::Accept;
                    }
                    // Stage-drop only when the replacement covered the
                    // entire original payload and every write succeeded.
                    let (fired, replaced) = self.execute_chain_tcp(&pkt, &data);
                    if fired && let Some(entry) = self.flows.get_mut(&key) {
                        entry.fired = true;
                    }
                    if replaced {
                        Verdict::Drop
                    } else {
                        Verdict::Accept
                    }
                }
            }
        } else if let Some(u) = &pkt.udp {
            // QUIC is inspected once and never staged: fakes only, the
            // original always passes. Non-QUIC datagrams never create state.
            if !crate::quic::looks_like_quic_initial(u.payload) {
                return Verdict::Accept;
            }
            let fresh = {
                let entry = self.touch(key);
                !entry.dead && !entry.fired
            };
            if fresh
                && self.execute_chain_udp(&pkt, key, u.payload)
                && let Some(entry) = self.flows.get_mut(&key)
            {
                entry.fired = true;
            }
            Verdict::Accept
        } else {
            Verdict::Accept
        }
    }

    /// Runs the active TCP chain's attacks against a staged TCP burst.
    /// Returns `(fired, replaced)`: `fired` marks the flow, `replaced`
    /// says a split replacement wrote every byte of the original payload;
    /// fake packets never count as replacement.
    fn execute_chain_tcp(&mut self, pkt: &Packet, data: &[u8]) -> (bool, bool) {
        if self.tcp_chains.is_empty() {
            return (false, false);
        }
        let chain = self.tcp_chains[self.tcp_chain_idx].clone();
        let t = match &pkt.tcp {
            Some(t) => t,
            None => return (false, false),
        };
        let reply_ttl = self
            .flows
            .get(&FlowKey::new(6, (pkt.src, t.sport), (pkt.dst, t.dport)))
            .and_then(|f| f.reply_ttl);
        let mut fired = false;
        let mut replaced = false;
        for attack in &chain.attacks {
            match attack {
                Attack::Fake(spec) => {
                    // Randomized blobs change per repeat; static blobs are
                    // materialized once.
                    let per_repeat = matches!(spec.blob, Blob::Quic)
                        || spec.tls_mods.rnd
                        || spec.tls_mods.rndsni;
                    let mut cached: Option<Vec<u8>> = None;
                    if !per_repeat {
                        match self.materialize_blob(&spec.blob, &spec.tls_mods, data) {
                            Some(p) => cached = Some(p),
                            None => continue,
                        }
                        fired = true;
                    }
                    let mut last_len = 0usize;
                    for _ in 0..spec.fooling.repeats {
                        let fake_payload = if per_repeat {
                            match self.materialize_blob(&spec.blob, &spec.tls_mods, data) {
                                Some(p) => {
                                    fired = true;
                                    p
                                }
                                None => continue,
                            }
                        } else {
                            cached.clone().expect("cached blob present")
                        };
                        last_len = fake_payload.len();
                        let b = OutBuilder::tcp(pkt.src, pkt.dst, t.sport, t.dport)
                            .payload(fake_payload.clone());
                        let b = crate::fooling::apply_fooling_tcp(
                            b,
                            &spec.fooling,
                            pkt,
                            reply_ttl,
                            self.rng.ip_id(),
                        );
                        let out = match b.build() {
                            Ok(out) => out,
                            Err(e) => {
                                log::warn!("fake build failed: {e}");
                                continue;
                            }
                        };
                        match self.sender.as_ref() {
                            Some(sender) => {
                                if let Err(e) = sender.send(&out.bytes, Ipv4Addr::from(pkt.dst)) {
                                    log::warn!("fake send failed: {e}");
                                }
                            }
                            None => log::debug!("no raw sender; fake skipped"),
                        }
                    }
                    log::debug!(
                        "fake sent ({} bytes x{}), ttl {}",
                        last_len,
                        spec.fooling.repeats,
                        spec.fooling.resolve_ttl(reply_ttl)
                    );
                }
                Attack::Split(spec) => {
                    let positions = crate::positions::resolve_multi(data, &spec.markers);
                    if positions.is_empty() {
                        continue;
                    }
                    fired = true;
                    let (disorder, seqovl, pattern) =
                        (spec.disorder, spec.seqovl, spec.seqovl_pattern.clone());
                    let piece_ids: Vec<u16> =
                        (0..=positions.len()).map(|_| self.rng.ip_id()).collect();
                    if self.send_segments(
                        pkt,
                        data,
                        &positions,
                        seqovl,
                        pattern.as_deref(),
                        disorder,
                        &piece_ids,
                    ) {
                        replaced = true;
                    }
                }
            }
        }
        (fired, replaced)
    }

    /// Sends the active UDP chain's fake packets for a QUIC Initial. The
    /// original is never staged for UDP, so there is no drop decision to
    /// make. Returns whether at least one fake was attempted.
    fn execute_chain_udp(&mut self, pkt: &Packet, key: FlowKey, data: &[u8]) -> bool {
        if !self.host_allowed(L7::QuicInitial, data) || self.udp_chains.is_empty() {
            return false;
        }
        let chain = self.udp_chains[self.udp_chain_idx].clone();
        let reply_ttl = self.flows.get(&key).and_then(|f| f.reply_ttl);
        let Some(u) = &pkt.udp else { return false };
        let mut attempted = false;
        for attack in &chain.attacks {
            let Attack::Fake(spec) = attack else { continue };
            let mut cached: Option<Vec<u8>> = None;
            match &spec.blob {
                Blob::Quic => {}
                _ => match self.materialize_blob(&spec.blob, &spec.tls_mods, data) {
                    Some(p) => cached = Some(p),
                    None => continue,
                },
            }
            attempted = true;
            for _ in 0..spec.fooling.repeats {
                let payload = match &spec.blob {
                    Blob::Quic => crate::quic::craft_quic_initial(&mut self.rng),
                    _ => cached.clone().expect("cached blob present"),
                };
                let out = match OutBuilder::udp(pkt.src, pkt.dst, u.sport, u.dport)
                    .payload(payload.clone())
                    .ttl(spec.fooling.resolve_ttl(reply_ttl))
                    .ip_id(self.rng.ip_id())
                    .badsum(spec.fooling.badsum)
                    .build()
                {
                    Ok(out) => out,
                    Err(e) => {
                        log::warn!("quic fake build failed: {e}");
                        continue;
                    }
                };
                match self.sender.as_ref() {
                    Some(sender) => {
                        if let Err(e) = sender.send(&out.bytes, Ipv4Addr::from(pkt.dst)) {
                            log::warn!("quic fake send failed: {e}");
                        }
                    }
                    None => log::debug!("no raw sender; quic fake skipped"),
                }
            }
            log::debug!("quic fake sent x{}", spec.fooling.repeats);
        }
        attempted
    }

    /// Sends the split segments (in order for multisplit, last-piece-first
    /// for disorder), applying seqovl+pattern to the leading piece, exactly
    /// mirroring zapret2 `multisplit`/`multidisorder_send`. Returns whether
    /// every piece was written in full; any failed or short write means the
    /// original must be accepted instead of the staged drop.
    #[allow(clippy::too_many_arguments)]
    fn send_segments(
        &mut self,
        pkt: &Packet,
        data: &[u8],
        positions: &[usize],
        seqovl: usize,
        pattern: Option<&[u8]>,
        disorder: bool,
        piece_ids: &[u16],
    ) -> bool {
        let Some(t) = &pkt.tcp else { return false };
        let base = t.seq;
        let n = positions.len();
        let mk_pattern = |need: usize| -> Vec<u8> {
            let pat: &[u8] = match pattern {
                Some(p) if !p.is_empty() => p,
                _ => &[0],
            };
            (0..need).map(|i| pat[i % pat.len()]).collect()
        };
        let send_piece =
            |sender: &Option<RawSender>, i: usize, start: usize, end: usize, ovl: usize| -> bool {
                let piece = &data[start..end];
                let mut payload = Vec::with_capacity(ovl + piece.len());
                if ovl > 0 {
                    payload.extend_from_slice(&mk_pattern(ovl));
                }
                payload.extend_from_slice(piece);
                let out = match OutBuilder::tcp(pkt.src, pkt.dst, t.sport, t.dport)
                    .seq(base.wrapping_add(start as u32).wrapping_sub(ovl as u32))
                    .ack(t.ack)
                    .flags(t.flags)
                    .window(t.window)
                    .options(t.options)
                    .payload(payload)
                    .ttl(pkt.ttl)
                    .ip_id(piece_ids.get(i).copied().unwrap_or(0))
                    .build()
                {
                    Ok(out) => out,
                    Err(e) => {
                        log::warn!("piece {i} build failed: {e}");
                        return false;
                    }
                };
                let Some(sender) = sender.as_ref() else {
                    log::warn!("no raw sender; piece {i} not sent");
                    return false;
                };
                match sender.send(&out.bytes, Ipv4Addr::from(pkt.dst)) {
                    Ok(n) if n == out.bytes.len() => {
                        log::debug!("piece {i} sent {n} bytes");
                        true
                    }
                    Ok(n) => {
                        log::warn!("piece {i} short write: {n}/{} bytes", out.bytes.len());
                        false
                    }
                    Err(e) => {
                        log::warn!("piece {i} write failed: {e}");
                        false
                    }
                }
            };

        let mut all_full = true;
        if disorder {
            // zapret2 multidisorder_send: pieces from last to first; the
            // second piece (i==1) carries seqovl-1.
            for i in (0..=n).rev() {
                let start = if i == 0 { 0 } else { positions[i - 1] };
                let end = if i < n { positions[i] } else { data.len() };
                let ovl = if i == 1 && seqovl > 0 && seqovl < positions[0] {
                    seqovl - 1
                } else {
                    0
                };
                all_full &= send_piece(&self.sender, i, start, end, ovl);
            }
        } else {
            for i in 0..=n {
                let start = if i == 0 { 0 } else { positions[i - 1] };
                let end = if i < n { positions[i] } else { data.len() };
                let ovl = if i == 0 && seqovl > 0 && seqovl < positions[0] {
                    seqovl
                } else {
                    0
                };
                all_full &= send_piece(&self.sender, i, start, end, ovl);
            }
        }
        log::debug!(
            "segments: {} pieces, seqovl {seqovl}, disorder {disorder}, complete {all_full}",
            n + 1
        );
        all_full
    }

    fn materialize_blob(
        &mut self,
        blob: &Blob,
        mods: &crate::tls::TlsMods,
        real: &[u8],
    ) -> Option<Vec<u8>> {
        match blob {
            Blob::Tls(sni) => {
                if !crate::tls::looks_like_client_hello(real) {
                    return None;
                }
                let mut payload = match crate::tls::craft_client_hello(sni, &mut self.rng) {
                    Ok(p) => p,
                    Err(_) => return None,
                };
                crate::tls::apply_tls_mods(&mut payload, mods, Some(real), &mut self.rng);
                Some(payload)
            }
            Blob::Quic => {
                if !crate::quic::looks_like_quic_initial(real) {
                    return None;
                }
                Some(crate::quic::craft_quic_initial(&mut self.rng))
            }
            Blob::Hex(b) => Some(b.clone()),
        }
    }

    fn host_allowed(&self, l7: L7, data: &[u8]) -> bool {
        let Some(set) = &self.hosts else { return true };
        let mut host = match l7 {
            L7::TlsClientHello => crate::tls::extract_sni(data).map(String::into_bytes),
            L7::HttpRequest => crate::positions::host_range(data)
                .and_then(|(o, l)| data.get(o..o + l))
                .map(<[u8]>::to_vec),
            L7::QuicInitial | L7::Unknown => None,
        };
        match &mut host {
            Some(host) => {
                host.make_ascii_lowercase();
                if l7 == L7::HttpRequest
                    && let Some(colon) = host.iter().rposition(|&b| b == b':')
                {
                    host.truncate(colon);
                }
                host_matches(set, host)
            }
            None => false,
        }
    }

    fn note_failure(&mut self, proto: u8) {
        let Some(rot) = &self.rotation else { return };
        if self.window_start.elapsed() > rot.window {
            self.failures = 0;
            self.window_start = Instant::now();
        }
        self.failures += 1;
        if self.failures >= rot.fails_threshold {
            self.failures = 0;
            self.window_start = Instant::now();
            let (idx, len, list) = match proto {
                6 => (&mut self.tcp_chain_idx, self.tcp_chains.len(), "tcp"),
                17 => (&mut self.udp_chain_idx, self.udp_chains.len(), "udp"),
                _ => return,
            };
            if len == 0 {
                return;
            }
            *idx = (*idx + 1) % len;
            log::info!("circular: rotating {list} to chain {}", *idx);
        }
    }

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

// Pure-logic pieces are covered in their own modules; the engine needs a
// live NFQUEUE + raw sockets for full I/O, exercised by the privileged
// tests. Verdict decisions are unit-tested below with a sender-less engine,
// where every replacement write counts as failed and the original passes.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{TCP_FLAG_ACK, TCP_FLAG_PSH};
    use crate::strategy::parse_chain;
    use crate::tls::{craft_client_hello, rand_bytes::Counter};

    fn test_engine(chains: Vec<Chain>, hosts: Option<Vec<String>>) -> Engine {
        Engine::with_sender(chains, None, hosts.map(normalize_hosts), None).expect("test engine")
    }

    fn split_chain() -> Vec<Chain> {
        vec![parse_chain("multisplit:pos=1,midsld").expect("valid chain")]
    }

    fn tcp_data_packet(seq: u32, payload: &[u8]) -> Vec<u8> {
        OutBuilder::tcp([10, 0, 0, 2], [93, 184, 216, 34], 50000, 443)
            .seq(seq)
            .ack(1)
            .flags(TCP_FLAG_PSH | TCP_FLAG_ACK)
            .payload(payload)
            .build()
            .expect("valid test packet")
            .bytes
    }

    fn ipv6_tcp_loopback() -> Vec<u8> {
        let payload = b"loopback";
        let mut b = Vec::with_capacity(40 + 20 + payload.len());
        b.extend_from_slice(&[0x60, 0, 0, 0]);
        b.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
        b.push(6); // next header: TCP
        b.push(64); // hop limit
        b.extend_from_slice(&[0u8; 16]); // ::1 source
        b.extend_from_slice(&[0u8; 15]); // ::1 destination
        b.push(1);
        b.extend_from_slice(&50000u16.to_be_bytes());
        b.extend_from_slice(&443u16.to_be_bytes());
        b.extend_from_slice(&1000u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.push(0x50); // data offset 5
        b.push(TCP_FLAG_PSH | TCP_FLAG_ACK);
        b.extend_from_slice(&[0xff, 0xff]); // window
        b.extend_from_slice(&[0, 0, 0, 0]); // checksum + urgent
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn readiness_sentinel_matches_the_supervisor_contract() {
        assert_eq!(
            format!("{READY_SENTINEL_PREFIX}{}", 200u16),
            "v2ray-rs-antidpi: ready queue=200"
        );
    }

    #[test]
    fn empty_chain_list_rejected_before_sockets() {
        assert!(matches!(
            Engine::new(Vec::new(), None, None),
            Err(crate::error::Error::Strategy(_))
        ));
    }

    #[test]
    fn ipv6_loopback_passthrough_unchanged() {
        let bytes = ipv6_tcp_loopback();
        assert!(Packet::parse(&bytes).is_err(), "IPv6 stays unparseable");
        let mut e = test_engine(split_chain(), Some(vec!["example.com".into()]));
        assert_eq!(e.handle_packet(&bytes, true, 0), Verdict::Accept);
        assert_eq!(e.handle_packet(&bytes, false, 0), Verdict::Accept);
        assert!(e.flows.is_empty(), "no flow state was created for IPv6");
    }

    #[test]
    fn fwmarked_and_unparseable_packets_pass() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let bytes = tcp_data_packet(1000, &ch);
        let mut e = test_engine(split_chain(), Some(vec!["example.com".into()]));
        assert_eq!(
            e.handle_packet(&bytes, true, DEFAULT_FWMARK),
            Verdict::Accept
        );
        assert_eq!(e.handle_packet(&[0u8; 8], true, 0), Verdict::Accept);
        assert!(e.flows.is_empty());
    }

    #[test]
    fn excluded_host_round_trips_byte_for_byte() {
        let ch =
            craft_client_hello("excluded.example", &mut Counter(0)).expect("crafted client hello");
        let bytes = tcp_data_packet(1000, &ch);
        let mut e = test_engine(split_chain(), Some(vec!["allowed.example".into()]));
        assert_eq!(e.handle_packet(&bytes, true, 0), Verdict::Accept);
        let entry = e.flows.values().next().unwrap();
        assert!(entry.dead, "excluded flow stops being inspected");
    }

    #[test]
    fn failed_replacement_accepts_original() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let bytes = tcp_data_packet(1000, &ch);
        // The test engine has no raw sender: every replacement write fails,
        // so the staged packet must be accepted.
        let mut e = test_engine(split_chain(), Some(vec!["example.com".into()]));
        assert_eq!(e.handle_packet(&bytes, true, 0), Verdict::Accept);
        assert!(e.flows.values().next().unwrap().fired);
    }

    #[test]
    fn retransmission_after_attempted_replacement_passes() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let bytes = tcp_data_packet(1000, &ch);
        let mut e = test_engine(split_chain(), Some(vec!["example.com".into()]));
        assert_eq!(e.handle_packet(&bytes, true, 0), Verdict::Accept);
        assert!(e.flows.values().next().unwrap().fired);
        // The TCP stack retransmits the staged range; it must pass.
        assert_eq!(e.handle_packet(&bytes, true, 0), Verdict::Accept);
    }

    #[test]
    fn stream_round_trip_byte_for_byte() {
        let hosts = Some(vec!["allowed.example".into()]);

        // Excluded host.
        let ch =
            craft_client_hello("excluded.example", &mut Counter(0)).expect("crafted client hello");
        let mut e = test_engine(split_chain(), hosts.clone());
        let bytes = tcp_data_packet(1000, &ch);
        let mut delivered = Vec::new();
        if e.handle_packet(&bytes, true, 0) == Verdict::Accept {
            delivered.extend_from_slice(&ch);
        }
        assert_eq!(delivered, ch);

        // Incomplete record split across two packets.
        let ch =
            craft_client_hello("excluded2.example", &mut Counter(0)).expect("crafted client hello");
        let mut e = test_engine(split_chain(), hosts.clone());
        let mut delivered = Vec::new();
        for (n, seg) in [&ch[..200], &ch[200..]].into_iter().enumerate() {
            let bytes = tcp_data_packet(1000 + 200 * n as u32, seg);
            if e.handle_packet(&bytes, true, 0) == Verdict::Accept {
                delivered.extend_from_slice(seg);
            }
        }
        assert_eq!(delivered, ch);

        // Coalesced record: ClientHello followed by more data.
        let mut coalesced =
            craft_client_hello("excluded3.example", &mut Counter(0)).expect("crafted client hello");
        coalesced.extend_from_slice(b"\x17\x03\x03\x00\x04tail");
        let mut e = test_engine(split_chain(), hosts.clone());
        let bytes = tcp_data_packet(1000, &coalesced);
        let mut delivered = Vec::new();
        if e.handle_packet(&bytes, true, 0) == Verdict::Accept {
            delivered.extend_from_slice(&coalesced);
        }
        assert_eq!(delivered, coalesced);

        // Injected send failure on an allowed host: the original passes.
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut e = test_engine(split_chain(), hosts);
        let bytes = tcp_data_packet(1000, &ch);
        let mut delivered = Vec::new();
        if e.handle_packet(&bytes, true, 0) == Verdict::Accept {
            delivered.extend_from_slice(&ch);
        }
        assert_eq!(delivered, ch);
    }

    #[test]
    fn hostlist_matches_exact_and_label_suffix() {
        let set = normalize_hosts(vec!["example.com".to_string()]);
        assert!(host_matches(&set, b"example.com"));
        assert!(host_matches(&set, b"www.example.com"));
        assert!(!host_matches(&set, b"badexample.com"));
        assert!(!host_matches(&set, b"ample.com"));
        assert!(!host_matches(&set, b"example.org"));
        assert!(!host_matches(&set, b""));
    }

    #[test]
    fn hostlist_normalized_lowercase_and_deduped() {
        let set = normalize_hosts(vec![
            "Example.COM".into(),
            ".Example.COM.".into(),
            "www.example.com".into(),
            "example.com".into(),
            "".into(),
        ]);
        assert_eq!(set.len(), 2);
        assert!(set.contains(b"example.com".as_slice()));
        assert!(set.contains(b"www.example.com".as_slice()));
    }

    fn udp_quic_packet(payload: &[u8]) -> Vec<u8> {
        OutBuilder::udp([10, 0, 0, 2], [93, 184, 216, 34], 50000, 443)
            .payload(payload)
            .build()
            .expect("valid test packet")
            .bytes
    }

    #[test]
    fn default_chains_protect_tcp_and_quic_from_the_first_flow() {
        let chains: Vec<Chain> = crate::strategy::DEFAULT_CHAINS
            .iter()
            .map(|s| parse_chain(s).expect("valid default chain"))
            .collect();
        let (tcp, udp) = partition_chains(chains);
        assert_eq!(tcp.len(), 2);
        assert_eq!(udp.len(), 1);
        let Attack::Fake(spec) = &udp[0].attacks[0] else {
            panic!("expected fake")
        };
        assert!(matches!(spec.blob, Blob::Quic));

        let mut e = test_engine(
            crate::strategy::DEFAULT_CHAINS
                .iter()
                .map(|s| parse_chain(s).expect("valid default chain"))
                .collect(),
            None,
        );
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        assert_eq!(
            e.handle_packet(&tcp_data_packet(1000, &ch), true, 0),
            Verdict::Accept
        );
        assert!(
            e.flows.values().next().unwrap().fired,
            "TCP ClientHello fires on the first flow"
        );

        let mut e = test_engine(
            crate::strategy::DEFAULT_CHAINS
                .iter()
                .map(|s| parse_chain(s).expect("valid default chain"))
                .collect(),
            None,
        );
        let init = crate::quic::craft_quic_initial(&mut Counter(0));
        assert_eq!(
            e.handle_packet(&udp_quic_packet(&init), true, 0),
            Verdict::Accept
        );
        assert!(
            e.flows.values().next().unwrap().fired,
            "QUIC Initial fires on the first flow"
        );
    }

    #[test]
    fn quic_only_chains_never_consume_tcp_flows() {
        let mut e = test_engine(
            vec![parse_chain("fake:blob=quic:repeats=11").unwrap()],
            None,
        );
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        assert_eq!(
            e.handle_packet(&tcp_data_packet(1000, &ch), true, 0),
            Verdict::Accept
        );
        assert!(!e.flows.values().next().unwrap().fired);
    }

    #[test]
    fn non_quic_udp_creates_no_flow_state() {
        let mut e = test_engine(split_chain(), None);
        assert_eq!(
            e.handle_packet(&udp_quic_packet(b"hello"), true, 0),
            Verdict::Accept
        );
        assert!(e.flows.is_empty());
    }

    #[test]
    fn rst_counts_once_per_flow() {
        let ch =
            craft_client_hello("www.example.com", &mut Counter(0)).expect("crafted client hello");
        let mut e = test_engine(split_chain(), None);
        e.rotation = Some(Rotation {
            fails_threshold: 10,
            window: Duration::from_secs(60),
        });
        assert_eq!(
            e.handle_packet(&tcp_data_packet(1000, &ch), true, 0),
            Verdict::Accept
        );
        assert!(e.flows.values().next().unwrap().fired);
        let rst = OutBuilder::tcp([93, 184, 216, 34], [10, 0, 0, 2], 443, 50000)
            .flags(crate::packet::TCP_FLAG_RST)
            .build()
            .expect("valid test packet")
            .bytes;
        for _ in 0..3 {
            assert_eq!(e.handle_packet(&rst, false, 0), Verdict::Accept);
        }
        assert_eq!(e.failures, 1);
    }

    #[test]
    fn eviction_drops_idle_then_oldest_entries() {
        let mut e = test_engine(split_chain(), None);
        for i in 0..MAX_FLOWS {
            let key = FlowKey::new(6, ([10, 0, 0, 1], i as u16 + 1), ([10, 0, 0, 2], 2));
            let entry = e.flows.entry(key).or_default();
            if i % 2 == 0 {
                entry.last_seen = Instant::now() - FLOW_TTL - Duration::from_secs(1);
            }
            if i % 3 == 0 {
                entry.fired = true;
            }
        }
        e.maybe_evict();
        assert_eq!(e.flows.len(), MAX_FLOWS / 2, "only fresh entries remain");
        assert!(e.flows.values().all(|f| f.last_seen.elapsed() <= FLOW_TTL));

        let base = Instant::now() - Duration::from_secs(30);
        for (n, key) in e
            .flows
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            e.flows.get_mut(&key).unwrap().last_seen = base + Duration::from_nanos(n as u64);
        }
        for i in (0..MAX_FLOWS).skip(e.flows.len()) {
            let key = FlowKey::new(17, ([10, 1, 0, 1], i as u16 + 1), ([10, 1, 0, 2], 2));
            e.flows.entry(key).or_default();
        }
        for (n, key) in e
            .flows
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            e.flows.get_mut(&key).unwrap().last_seen = base + Duration::from_nanos(n as u64);
        }
        e.maybe_evict();
        assert_eq!(e.flows.len(), MAX_FLOWS - MAX_FLOWS / EVICT_FRACTION);
        assert!(!e.flows.is_empty());
    }

    #[test]
    fn http_host_port_is_stripped_for_hostlists() {
        let req = b"GET / HTTP/1.1\r\nHost: example.com:8080\r\n\r\n";
        let mut e = test_engine(split_chain(), Some(vec!["example.com".into()]));
        assert_eq!(
            e.handle_packet(&tcp_data_packet(1000, req), true, 0),
            Verdict::Accept
        );
        assert!(
            e.flows.values().next().unwrap().fired,
            "Host with port matches the hostlist"
        );
    }
}
