# Anti-DPI Technique Registry

Classification of DPI-circumvention techniques for v2ray-rs, targeted first at
Russian ISP censorship (TSPU). Compiled 2026-11 from primary-source research of
[zapret2](https://github.com/bol-van/zapret2),
[nfqws2-keenetic](https://github.com/nfqws/nfqws2-keenetic),
[sni-spoofing-rust](https://github.com/therealaleph/sni-spoofing-rust) (MIT),
[Vane](https://github.com/lulushuz/Vane) (GPL — parameter design only),
[rust-DPI-http-proxy](https://github.com/vinhjaxt/rust-DPI-http-proxy) and
[FreeDPI-Windows](https://github.com/AlexZander85/ByeByeDPI-Windows) (ex
ByeByeDPI-Windows). zapret/Vane are GPL: their *technique knowledge* is used
here, never their code. sni-spoofing-rust is MIT and may be adapted with
attribution.

Strategy semantics below were verified against zapret2 sources
(`lua/zapret-antidpi.lua`, `lua/zapret-lib.lua`, `nfq2/protocol.c`,
`nfq2/darkmagic.c`) — those are the normative definitions this crate ports.

## 1. How Russian TSPU blocks (failure modes)

| # | Failure mode | Mechanism | Anti-DPI counter |
|---|--------------|-----------|------------------|
| F1 | Passive SNI match | First TLS ClientHello inspected; blocked domain ⇒ RST or silent drop | fake (latch), multisplit/disorder, seqovl |
| F2 | TLS record/length heuristics | Connection throttled when ClientHello size/fingerprint matches a proxy pattern | seqovl, multisplit, protocol-level fixes (REALITY) |
| F3 | QUIC drop | UDP/443 QUIC Initials dropped or heavily throttled | fake QUIC Initial (`repeats`), QUIC disorder |
| F4 | Reassembly DPI | DPI reassembles the stream before matching | disorder (reassembly fails/timeout), seqovl (desync), hostfakesplit |
| F5 | IP-range block | Server IP/subnet blocked at the router | **none** — only server relocation helps |
| F6 | Active probing / MITM | DPI actively connects or terminates TLS | none of these techniques; needs REALITY-grade server behavior |
| F7 | DNS poisoning | Bogus answers for blocked names | outside scope: v2ray-rs split DNS / DoH already handles it |

Anti-DPI targets F1-F4 when the relevant TLS or QUIC traffic reaches the
engine. It cannot fix IP-range blocks or active probing. Queue bypass and
startup ordering do not guarantee that the first handshake is processed.

## 2. Layer classification

- **L1 — Kernel packet level** (NFQUEUE and raw sockets; needs `cap_net_admin`
  and `cap_net_raw`). `v2ray-rs-antidpi` queues TCP and UDP port 443 traffic
  in the local output and prerouting paths, including eligible tunnel traffic.
  Plain HTTP on port 80 is not covered. The IPv6 nftables table exists only as
  a pass-through ownership artifact: IPv6 traffic is never queued or modified,
  and the port-443 interception described here applies to IPv4 only.
- **L2 — Local stream level** (userspace transparent/local proxy that splits
  the ClientHello on write; rootless; per-app coverage only; cannot intercept
  TUN-routed traffic of other processes). GreenTunnel-style. Reference:
  rust-DPI-http-proxy, tpws from zapret. Out of scope for v1; possible
  rootless fallback later.
- **L3 — Protocol level** (baked into the tunnel protocol itself: REALITY,
  Vision, uTLS fingerprints, ECH, padding). Already available in xray via
  config; no packet tricks needed. v2ray-rs already generates these.

## 3. Technique catalog (L1, NFQUEUE daemon)

The implementation queues TCP port 443 packets within the first 12 tracked
packets per direction and UDP port 443 packets. Re-injected packets carry
fwmark `0x40000000` and bypass the queue. With a hostlist, matching uses TLS
SNI or HTTP Host; QUIC Initials do not match. HTTP Host matching applies only
to traffic that reaches the engine, not plain HTTP on port 80. Both nftables
families are owned so packets leave the hooks deterministically, but the IPv6
table is a pass-through ownership artifact: it queues nothing, IPv6 traffic
is not intercepted or modified, and port-443 interception is IPv4-only.

The following catalog describes techniques, including techniques outside the
current implementation. It is not a statement of current traffic coverage.

### 3.1 Faking — `fake`

On the first data packet of a flow (TLS ClientHello / HTTP request / QUIC
Initial), send one or more **decoy packets** carrying the same 4-tuple and seq,
whose payload is a blob with an *allowed* SNI/host. The server discards the
decoy (it either fails the checksum, dies at the DPI hop by TTL, or arrives as
a duplicate/out-of-window copy); the DPI latches onto the decoy and does not
match the real one (F1, partially F2).

Fooling parameters applied to the decoy (all verified in `apply_fooling`,
zapret-lib.lua):

| Param | Mechanism | Why the server ignores the fake |
|-------|-----------|--------------------------------|
| `ip_ttl=N` / `ip6_ttl=N` | fixed TTL | dies before the server |
| `ip_autottl=delta,min-max` | TTL chosen so the fake dies *at the DPI hop* but would reach the server; TTL guessed from the observed reply TTL (see autottl below) | dies right after passing the DPI |
| `badsum` | wrong TCP checksum | kernel of any hops/endpoint drops it (DPI does not verify) |
| `tcp_seq=N` (badseq) | seq offset +N | out-of-window at the server |
| `tcp_ack=N` (badack) | ack offset +N | out-of-window; with `tcp_ts_up` Linux accepts it silently, TSPU chokes on it |
| `tcp_md5[=hex]` | TCP MD5-signature option | TSPU cannot validate the signature; Linux hosts without the secret ignore it |
| `tcp_flags_unset=ack` (datanoack) | ACK flag cleared | server drops; DPI sees a "request" |
| `ip_id=seq/rnd/zero` | IP ID pattern | defeats IP-ID based tracking |
| `repeats=N` | send N decoys | counteracts DPI dropping one copy |
| `tls_mod=rnd,rndsni,sni=HOST,dupsid,padencap` | mutate the fake ClientHello (below) | makes the decoy unique/size-matched |

`tls_mod` semantics (protocol.c `TLSMod`):

- `rnd` — randomize ClientHello.random (offset 11..43) and session id;
- `rndsni` — randomize the SNI hostname (alpha start, alnum body, 3-char TLD);
- `sni=HOST` — replace SNI text, adjusting record/handshake/extension/SNI
  lengths at all 5 offsets;
- `dupsid` — copy the *real* ClientHello's session id into the fake so the
  fake looks session-consistent to DPI that correlates them;
- `padencap` — grow the TLS padding extension (add one if absent) so the fake
  has the same total size as the real ClientHello (defeats size filtering).

**autottl algorithm** (zapret-lib.lua `autottl`): guess the reply-path hop
count from the observed incoming TTL (`>223 ⇒ base 255; 96<·≤128 ⇒ 128;
32<·≤64 ⇒ 64`); `fake_ttl = clamp(hops + delta, min, max)`; if
`delta<0` and `fake ≥ hops` → fail (fake would reach the server); if
`delta≥0` and `fake < hops` → fail (fake dies before the DPI). Cached per
flow+instance once discovered.

### 3.2 TCP segmentation — `multisplit` / `multidisorder`

Reassemble the outgoing message (typically the ClientHello), cut it at
position markers, and send each piece as its own TCP segment (packet with
payload slice, seq advanced accordingly), dropping the queued original.

- `multisplit pos=…` — segments sent **in order**; defeats single-packet
  ClientHello matching (F1).
- `multidisorder pos=…` — segments sent **out of order** (last piece first;
  first piece last); DPI reassembly fails or times out (F4); server
  reassembles fine.
- Position markers (protocol.c `PM_*`, resolved against the reassembled
  payload): absolute offset `N` / `-N`; `host` / `endhost` (start/end of
  hostname — HTTP host or TLS SNI); `sld` / `midsld` / `endsld` (start,
  middle, end of the second-level domain); `method` (HTTP); `sniext` (start of
  the SNI TLS extension); `extlen` (TLS extensions length field). A `+N/−N`
  suffix offsets the marker. Position 1 is never a valid split.
- `seqovl=N[:seqovl_pattern=BLOB]` — only on the first segment: lower its seq
  by N and prepend N bytes of pattern, so the server discards the overlap while
  the DPI's byte-offset tracking desyncs (F4). Must be smaller than the first
  split position and ≤ MSS.
- `fakedsplit` / `fakeddisorder` — each real segment surrounded by fake
  pattern-filled segments; `hostfakesplit` — splits only around the hostname
  (`[before][fake host][real host][fake host][after]`), defeating DPIs that
  reassemble but pin the first-seen host (F4).

### 3.3 Handshake tricks — `syndata`, `synack`, `wssize`

- `syndata` — carry payload (default 16 zero bytes, or a TLS blob) on the
  outgoing SYN; DPIs that only inspect post-handshake streams miss it.
- `synack` / `synack_split` — inject an ACK-flavored copy of the SYN, or split
  SYN,ACK into two packets (modes `syn|synack|acksyn`): the DPI sees an
  inconsistent handshake state.
- `wssize 1:6` — force a tiny receive window on the *server* side so the
  server itself fragments ServerHello/Cert (defeats DPI matching of server
  responses, e.g. QUIC cert throttling); must precede syndata in the profile;
  cuts off after the first ClientHello. `wsize` (SYN,ACK window rewrite) is
  largely outdated.

### 3.4 QUIC / UDP — `fake` + `udplen` + `ipfrag2`

- `fake blob=quic_initial` — decoy QUIC Initial packets (CID-spoofed,
  ~1200 B); no re-encryption needed since the server never processes them;
  typical `repeats=11` for F3. Works because TSPU latches the first Initial.
- `udplen=±N[:pattern]` — resize UDP length so size-fingerprinting of QUIC
  Initials fails.
- `ipfrag2[=pos[:order]]` — IP-layer fragmentation (frag offset 8 for UDP);
  defeats DPI parsers that do not reassemble IP fragments (F4). IPv6 uses a
  fragment extension header.

### 3.5 HTTP tampering (plain HTTP / F1-HTTP)

`http_hostcase` ("Host:" → "hOsT:" or spell), `http_domcase` (alternating
case of the hostname), `http_methodeol` (move CRLF before the method line,
requires a UA header), `http_unixeol` (LF-only EOL). All defeat naive
`Host:` regexes. Relevant only for plaintext HTTP; modern targets are TLS.

### 3.6 Orchestration — `circular`

`circular:fails=2:time=300:retrans=3:nld=2` wraps a numbered list of
strategies and rotates to the next one after N failures within the time
window, retrying from the first after the list is exhausted. ISP strategies
decay; rotation automates survival. nfqws2-keenetic ships it as the default
orchestrator with three TCP strategies (see §5).

### 3.7 Flow state model (no kernel conntrack dependency)

The daemon keeps its own per-flow table keyed by the 5-tuple: observed
reply TTL (for autottl), original seq/ack bases, seen ClientHello bytes,
per-strategy cutoff flags, first-N-packets window. Everything else
(dissection, matching, hostlists) is userspace. This is exactly portable to
Rust (zapret2's own daemon does it with a hash table, not netlink conntrack
APIs).

## 4. Non-NFQUEUE techniques (L2/L3) — registry completeness

| Technique | Layer | Notes |
|-----------|-------|-------|
| Out-of-window fake ClientHello (sni-spoofing-rust) | L1' (AF_PACKET, no NFQUEUE) | local forwarder watches the handshake via AF_PACKET and injects a fake with `seq = ISN+1-len(fake)`; DPI sees whitelisted SNI; server drops it as out-of-window. MIT code available. Alternative injection path for the same fake idea |
| ClientHello mid-hostname split at the proxy | L2 | stream-level split on write; rootless; per-app |
| Host header space/case tampering at proxy | L2 | plain HTTP only |
| REALITY/Vision | L3 | already in xray configs |
| uTLS fingerprint | L3 | Go-only (utls); Rust: rquest/reqwest-impersonate (messy publish state); not needed for packet-level work |
| ECH | L3 | `dig HTTPS` shows `ech=`; ECH domains are invisible to hostlists — use ipsets then |

## 5. Russia 2024-2025 reference strategies

From nfqws2-keenetic defaults (`etc/nfqws2/nfqws2.conf`) and its
[community discussion #2](https://github.com/nfqws/nfqws2-keenetic/discussions/2).
**Strategies are ISP-specific and decay** — treat as seeds, not truths.

- TCP 443 (typical TSPU TLS):
  1. `fake:blob=tls_clienthello:tls_mod=rnd,dupsid,sni=fonts.google.com:tcp_seq=10000` + `multisplit:pos=1,midsld:seqovl=1`
  2. `fake:blob=0x00000000:tcp_ack=-66000:repeats=2` + plain `multisplit`
  3. `hostfakesplit:host=ozon.ru:midhost=host-2:seqovl=sniext+3:badsum:tcp_md5`
  (wrapped in `circular:fails=2:time=300:retrans=3:nld=2`)
- QUIC 443: `fake:blob=quic_initial:repeats=11`
- HTTP 80: `http_methodeol:badsum`
- Community heavy variants: `seqovl` 681–20000 with `tcp_ts=-600000:tcp_ts_up:ip_id=zero`
  (Dom.ru), per-ISP strategy sets differ (Rostelecom/Dom.ru/MTS/Beeline).

These are upstream reference strategies, not a description of the current
firewall rules. Port 80 is not queued by the current implementation.

## 6. Rust portability notes

- `nfq` crate 0.2.6 (pure Rust over netlink, no libnetfilter_queue C dep) —
  receive/verdict/modify path; maintained 2026-11.
- `etherparse` 0.21 for L3/L4 parse+serialize with checksum helpers; hand-rolled
  TCP/IP/TLS parsers are also viable (sni-spoofing-rust proves ~400 lines).
- Raw IP re-injection: `IP_HDRINCL` raw sockets (needs `cap_net_raw`) +
  `SO_MARK` (needs `cap_net_admin`) for the loop guard — same choice as nfqws.
- Kernel-side needs: nftables NFQUEUE rules and the sysctl prerequisites in
  §7. The helper programs nftables through netlink.
- Not portable: WinDivert (Windows), ipfw/pf divert (BSD).

## 7. Operator reference

The app installs rules before starting the backend. The helper parses every
supplied strategy and loads the hostlist before any kernel mutation. Invalid
strategies or an unreadable hostlist fail before backend startup.
Parsing is strict: an unknown or duplicate parameter, a mistyped boolean,
`repeats` outside 1..=64, `seqovl` above 1500, or a malformed hex blob is an
error (`sni=` belongs inside `tls_mod=`). Chains are partitioned by protocol
when the engine starts. TCP chains carry any split or a non-QUIC fake.
UDP/QUIC chains carry a QUIC or raw-blob fake. The two lists rotate
independently, so the default chains protect TLS and QUIC from the first
flow of each.

```text
v2ray-rs-antidpi check --strategy S
v2ray-rs-antidpi install-rules --queue N [--strategy S]... [--hosts PATH] [--owner PID:START]
v2ray-rs-antidpi run --queue N --no-fw [--strategy S]... [--hosts PATH]
v2ray-rs-antidpi remove-rules [--sweep] [--owner PID:START]
```

`--strategy` is repeatable. `--owner` identifies the controller by PID and
process start ticks; when omitted, the helper uses its parent process. An
asserted owner is accepted only when it names the helper's actual parent
process (matching start ticks); recovery of a dead controller's state goes
through `remove-rules --sweep`, not a hand-written `--owner`.
`run --no-fw` leaves firewall installation to the caller. When a journal
exists, `run` checks its active phase, epoch, queue and recorded tables before
binding. With `--no-fw`, an absent journal is accepted for externally managed
rules.

After binding NFQUEUE, the engine prints exactly this line on stdout, with
`N` replaced by the queue number:

```text
v2ray-rs-antidpi: ready queue=N
```

The app treats an exit before readiness as a startup failure and an exit
after readiness as unexpected.

### Firewall ownership and recovery

The helper uses a hand-written nf_tables netlink client, not an `iptables`,
`ip6tables`, or `nft` subprocess. It creates an owned `v2ray_rs_<euid>` table in each IP family. The `ip` table
carries the output and prerouting hooks. The `ip6` table carries only the
session marker, because it is a pass-through ownership artifact. Cleanup deletes the
recorded tables. Queue bypass means traffic can pass without a listener.

The bounded, owner-only journal is
`$XDG_STATE_HOME/v2ray-rs/antidpi.v1.json`, or
`~/.local/state/v2ray-rs/antidpi.v1.json` by default.
`V2RAYRS_ANTIDPI_STATE` overrides the path. The file mode is `0600`.
The record contains `version`, `session_id`, `epoch` (boot ID and network
namespace device and inode), `pidns` (PID namespace device and inode;
records written before this field existed lack it), `owner` (PID and start
ticks), `phase`, `queue`, `fwmark`, `ipv4`, and `ipv6`. The journal
directory must be a real, caller-owned, owner-only directory: missing
components are created with mode `0700`, an unsuitable existing parent is
refused without being modified, and a symlinked parent is never followed.

Installation writes `applying` before table creation, records each installed
family, then writes `active`. Fresh intent requires both owned table names
to be absent; creation is exclusive (`NLM_F_EXCL`), so a same-named table is
never adopted whatever marker it carries, and a confirmed `EEXIST` drops
only the attempted family's unacquired obligation. A verified same-owner
reinstall performs no kernel mutation and repairs only a family flag the
journal failed to record. Removal writes `cleaning` before deleting
recorded tables. A failed removal, an ambiguous reply, or an exchange
timeout keeps the journal for retry; obligations are cleared only after a
removal confirms absence.

All netlink exchanges of a transaction share one monotonic deadline: five
seconds across an install, a three-second rollback reserve, eight seconds
for a removal. Each socket send and wait checks the remaining time; an
exhausted budget is a distinct timeout error and keeps the journal.

Normal removal refuses a foreign PID namespace, a different epoch or owner.
Installation refuses a foreign PID namespace, a different epoch, or a live
owner; it recovers a dead owner's recorded tables in the same epoch before
installing. Journal transactions serialize on one per-effective-UID,
per-network-namespace lock (`/run/user/<euid>`, or `/run/v2ray-rs-antidpi`
for euid 0) independent of any journal path override; the lock fails closed
when the runtime directory is unusable. `remove-rules --sweep` handles a
provably dead controller in the same boot, network namespace, and PID
namespace; a live controller never sweeps after engine death — it removes
its own recorded rules with its identity. A previous boot's stale record is
retired only when both owned table names are confirmed absent in the
current namespace, and table deletion across boots stays forbidden. A
same-boot journal from another network namespace is refused and names that
namespace. Records without matching PID-namespace evidence are preserved
for operator inspection, and tables without a journal are reported, never
deleted.

### Kernel prerequisites

The required values are `net.netfilter.nf_conntrack_acct=1` and
`net.netfilter.nf_conntrack_tcp_be_liberal=1`. The helper never writes
sysctls: `check`, `install-rules`, and `run` only report unmet prerequisites
and warn, keeping the nonfatal policy without changing namespace policy
implicitly.

## 8. Acceptance

Live-ISP A/B is not a release check. Run the `privileged-tests` suite with
disposable network and filesystem state. It exercises TLS handshakes through
throwaway network namespaces and a censor emulator, and includes an nftables
install/remove lifecycle test. The lifecycle test installs and removes the
owned `v2ray_rs_<euid>` tables in a private namespace. The traffic test
sends real TCP/443, UDP/443, and IPv6/443 traffic through rules that
`install-rules` installed. The suite can skip when privileged prerequisites are
unavailable; a skip is not acceptance evidence.
