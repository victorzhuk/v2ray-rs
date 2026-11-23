//! Hand-rolled NFPROTO nf_tables netlink client (libc only).
//!
//! Installs the per-family `v2ray_rs_<euid>` table with two mangle-priority
//! base chains (`output` on LOCAL_OUT, `prerouting` on PRE_ROUTING) that jump
//! into a regular `guard` chain. The guard chain mirrors the iptables
//! semantics of `fw.rs`: a fwmark bypass, a conntrack packet-count gate on
//! the first 12 packets (original direction for `tcp dport`, reply direction
//! for `tcp sport`), and NFQUEUE verdicts with bypass for tcp/udp port 443.
//!
//! Ownership: each table is created once with `NLM_F_CREATE | NLM_F_EXCL`
//! and carries the journal's session UUID as opaque raw bytes in
//! `NFTA_TABLE_USERDATA`. Deletion probes the object with GETTABLE first,
//! refuses a missing marker, a foreign marker, or a family/name mismatch,
//! and then deletes by the 64-bit kernel handle (`NFTA_TABLE_HANDLE`, big
//! endian) instead of the name. `EEXIST` is a refusal naming the existing
//! object, never a replacement.
//!
//! Encodings derive from `/usr/include/linux/netlink.h`,
//! `/usr/include/linux/netfilter{,/}` (`nfnetlink.h`, `nf_tables.h`,
//! `netfilter.h`, `netfilter_ipv4.h`, `nf_conntrack_tuple_common.h`).
//! Attribute numbers and enum values come from those headers verbatim. The
//! host-vs-network byte order of individual NLA_U32 attributes follows the
//! kernel/nftables wire convention: register ids (`DREG`/`SREG`) and the
//! bitwise length are read with `nla_get_be32` in the kernel and sent
//! big-endian; hook numbers, priorities, chain policies, verdict codes,
//! expression keys and cmp ops are big-endian. Register data blobs
//! (`NFTA_DATA_VALUE` inside immediate, bitwise mask/xor, and cmp data) are
//! raw bytes. The kernel stores `meta mark` in
//! host byte order, so its blobs travel native-endian. It stores
//! `ct packets` in host byte order too, so the TCP rules pass it through a
//! `byteorder` expression and compare the network-order result against
//! big-endian bounds. Payload-derived port comparisons stay network-order.
//! Wire choices are exercised against the real kernel by the privileged
//! install/remove and traffic tests in `tests/privileged.rs`. Every
//! mismatch surfaces as an errno that names the offending operation. Table,
//! chain, and expression names are sent with one trailing NUL, matching
//! `NLA_STRING` and `nla_strscpy`. Reply handling consumes correlated acks
//! across datagrams until every ack-flagged request is answered and any
//! required GETTABLE result arrived, rejecting wrong senders, truncation,
//! and incomplete success under an absolute receive deadline.
//!
//! IPv6 note: the `ip6` table is an ownership artifact only. It carries the
//! session marker and no chains or rules, so IPv6 traffic never reaches
//! NFQUEUE and the IPv4-only engine. Only the `ip` table queues port 443.

use crate::error::{Error, Result};
use std::collections::BTreeSet;
use std::io::{self};
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};
use uuid::Uuid;

// /usr/include/linux/netlink.h
const NETLINK_NETFILTER: libc::c_int = 12;
const NLMSG_ERROR: u16 = 0x2;
const NLMSG_MIN_TYPE: u16 = 0x10;
const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_ACK: u16 = 0x04;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const NLM_F_APPEND: u16 = 0x800;
const NLA_ALIGNTO: usize = 4;

// /usr/include/linux/netfilter/nfnetlink.h
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = NLMSG_MIN_TYPE;
const NFNL_MSG_BATCH_END: u16 = NLMSG_MIN_TYPE + 1;
const NFNETLINK_V0: u8 = 0;

// /usr/include/linux/netfilter/nf_tables.h — message types.
const NFT_MSG_NEWTABLE: u16 = 0;
const NFT_MSG_GETTABLE: u16 = 1;
const NFT_MSG_DELTABLE: u16 = 2;
const NFT_MSG_NEWCHAIN: u16 = 3;
const NFT_MSG_NEWRULE: u16 = 6;

// /usr/include/linux/netfilter/nf_tables.h — table attributes.
const NFTA_TABLE_NAME: u16 = 1;
const NFTA_TABLE_HANDLE: u16 = 4;
const NFTA_TABLE_PAD: u16 = 5;
const NFTA_TABLE_USERDATA: u16 = 6;

// /usr/include/linux/netfilter/nf_tables.h — chain attributes.
const NFTA_CHAIN_TABLE: u16 = 1;
const NFTA_CHAIN_NAME: u16 = 3;
const NFTA_CHAIN_HOOK: u16 = 4;
const NFTA_CHAIN_POLICY: u16 = 5;
const NFTA_CHAIN_TYPE: u16 = 7;

// /usr/include/linux/netfilter/nf_tables.h — hook attributes.
const NFTA_HOOK_HOOKNUM: u16 = 1;
const NFTA_HOOK_PRIORITY: u16 = 2;

// /usr/include/linux/netfilter/nf_tables.h — rule attributes.
const NFTA_RULE_TABLE: u16 = 1;
const NFTA_RULE_CHAIN: u16 = 2;
const NFTA_RULE_EXPRESSIONS: u16 = 4;

// /usr/include/linux/netfilter/nf_tables.h — expression/data/verdict nesting.
const NFTA_LIST_ELEM: u16 = 1;
const NFTA_EXPR_NAME: u16 = 1;
const NFTA_EXPR_DATA: u16 = 2;
const NFTA_DATA_VALUE: u16 = 1;
const NFTA_DATA_VERDICT: u16 = 2;
const NFTA_VERDICT_CODE: u16 = 1;
const NFTA_VERDICT_CHAIN: u16 = 2;

// /usr/include/linux/netfilter/nf_tables.h — expression attributes.
const NFTA_IMMEDIATE_DREG: u16 = 1;
const NFTA_IMMEDIATE_DATA: u16 = 2;
const NFTA_BITWISE_SREG: u16 = 1;
const NFTA_BITWISE_DREG: u16 = 2;
const NFTA_BITWISE_LEN: u16 = 3;
const NFTA_BITWISE_MASK: u16 = 4;
const NFTA_BITWISE_XOR: u16 = 5;
const NFTA_CMP_SREG: u16 = 1;
const NFTA_CMP_OP: u16 = 2;
const NFTA_CMP_DATA: u16 = 3;
const NFTA_PAYLOAD_DREG: u16 = 1;
const NFTA_PAYLOAD_BASE: u16 = 2;
const NFTA_PAYLOAD_OFFSET: u16 = 3;
const NFTA_PAYLOAD_LEN: u16 = 4;
const NFTA_META_DREG: u16 = 1;
const NFTA_META_KEY: u16 = 2;
const NFTA_CT_DREG: u16 = 1;
const NFTA_CT_KEY: u16 = 2;
const NFTA_CT_DIRECTION: u16 = 3;
const NFTA_QUEUE_NUM: u16 = 1;
const NFTA_QUEUE_FLAGS: u16 = 3;
const NFTA_BYTEORDER_SREG: u16 = 1;
const NFTA_BYTEORDER_DREG: u16 = 2;
const NFTA_BYTEORDER_OP: u16 = 3;
const NFTA_BYTEORDER_LEN: u16 = 4;
const NFTA_BYTEORDER_SIZE: u16 = 5;

// /usr/include/linux/netfilter/nf_tables.h — enum values.
const NFT_REG_VERDICT: u32 = 0;
const NFT_REG32_00: u32 = 8;
const NFT_META_MARK: u32 = 3;
const NFT_META_L4PROTO: u32 = 16;
const NFT_CT_PKTS: u32 = 14;
const NFT_CMP_EQ: u32 = 0;
const NFT_CMP_LTE: u32 = 3;
const NFT_CMP_GTE: u32 = 5;
const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;
const NFT_QUEUE_FLAG_BYPASS: u16 = 0x01;
const NFT_JUMP: i32 = -3;
const NFT_BYTEORDER_HTON: u32 = 1;

// /usr/include/linux/netfilter/netfilter.h
const NF_INET_PRE_ROUTING: u32 = 0;
const NF_INET_LOCAL_OUT: u32 = 3;
const NF_ACCEPT: u32 = 1;
const NFPROTO_IPV4: u8 = 2;
const NFPROTO_IPV6: u8 = 10;

// /usr/include/linux/netfilter_ipv4.h
const NF_IP_PRI_MANGLE: i32 = -150;

// /usr/include/linux/netfilter/nf_conntrack_tuple_common.h
const IP_CT_DIR_ORIGINAL: u8 = 0;
const IP_CT_DIR_REPLY: u8 = 1;

/// First tracked packets per direction that reach the queue; mirrors the
/// iptables `--connbytes 1:12` gate the chain replaces.
const CT_PACKET_LOW: u64 = 1;
const CT_PACKET_HIGH: u64 = 12;

const GUARD_CHAIN: &str = "guard";
const OUTPUT_CHAIN: &str = "output";
const PREROUTING_CHAIN: &str = "prerouting";

/// Absolute receive budget for one transact, backing the per-socket
/// SO_RCVTIMEO: incomplete reply sets are refused, never awaited forever.
const RECV_DEADLINE: Duration = Duration::from_secs(10);

/// Total monotonic budget for one full install operation (both family
/// installs, fresh probes, and the rollback window together).
const INSTALL_BUDGET: Duration = Duration::from_secs(5);
/// Total monotonic budget for one full removal or sweep operation.
const REMOVAL_BUDGET: Duration = Duration::from_secs(8);
/// Kernel time reserved inside [`INSTALL_BUDGET`] for the rollback drain:
/// install exchanges may never consume this window.
pub(crate) const ROLLBACK_RESERVE: Duration = Duration::from_secs(3);

/// One monotonic deadline shared by every exchange of a firewall
/// operation. A received reply never extends it and there is no
/// per-exchange reset: the only way to get budget is to still have some.
pub(crate) struct Budget {
    deadline: Instant,
}

impl Budget {
    /// Budget for a full install, with [`ROLLBACK_RESERVE`] held back for
    /// the rollback drain.
    pub(crate) fn install() -> Budget {
        Budget::with_total(INSTALL_BUDGET)
    }

    /// Budget for a full removal or sweep.
    pub(crate) fn removal() -> Budget {
        Budget::with_total(REMOVAL_BUDGET)
    }

    /// Budget for one read-only probe outside a bounded operation.
    pub(crate) fn probe() -> Budget {
        Budget::with_total(RECV_DEADLINE)
    }

    fn with_total(total: Duration) -> Budget {
        Budget {
            deadline: Instant::now() + total,
        }
    }

    /// Wait bound for the next exchange: at most `default`, never dipping
    /// into `reserve` of the remaining budget, and `None` once exhausted.
    fn slice(&self, default: Duration, reserve: Duration) -> Option<Duration> {
        let remaining = self.deadline.checked_duration_since(Instant::now())?;
        let wait = remaining.checked_sub(reserve)?.min(default);
        (wait > Duration::ZERO).then_some(wait)
    }

    #[cfg(test)]
    fn remaining(&self) -> Option<Duration> {
        self.deadline.checked_duration_since(Instant::now())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    V4,
    V6,
}

impl Family {
    fn nfproto(self) -> u8 {
        match self {
            Family::V4 => NFPROTO_IPV4,
            Family::V6 => NFPROTO_IPV6,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Family::V4 => "ipv4",
            Family::V6 => "ipv6",
        }
    }
}

/// Per-UID table name: the kernel objects are host-global while the state
/// journal is per user, so one user's cleanup must never delete another
/// user's live table.
pub(crate) fn table_name() -> String {
    format!("v2ray_rs_{}", unsafe { libc::geteuid() })
}

/// Installs the table for `session` as a fresh create: the owned name must
/// be confirmed absent first, and the table is then created with
/// `NLM_F_CREATE | NLM_F_EXCL` plus the marker. A confirmed `EEXIST` is the
/// typed [`Error::Conflict`], never adoption: a pre-existing table is not
/// inspected for a matching marker and is never kept or reused.
pub(crate) fn install(
    family: Family,
    queue: u16,
    fwmark: u32,
    session: Uuid,
    budget: &Budget,
) -> Result<()> {
    install_with(
        family,
        queue,
        fwmark,
        &table_name(),
        &session,
        budget,
        &mut |batch| bounded_transport(budget, ROLLBACK_RESERVE, batch),
    )
}

/// Removes the table only when it carries this session's marker, deleting by
/// the verified kernel handle and confirming absence afterwards.
pub(crate) fn remove(family: Family, session: Uuid, budget: &Budget) -> Result<()> {
    remove_with(family, &table_name(), &session, budget, &mut |batch| {
        bounded_transport(budget, Duration::ZERO, batch)
    })
}

/// True only when the table exists and matches family, name, the 16-byte
/// session marker, and a kernel handle. Confirmed absence is Ok(false); a
/// markerless or foreign-marked table is a refusal, never a false.
pub(crate) fn present(
    family: Family,
    session: Uuid,
    budget: &Budget,
    reserve: Duration,
) -> Result<bool> {
    present_with(
        family,
        &table_name(),
        &session,
        budget,
        reserve,
        &mut |batch| bounded_transport(budget, reserve, batch),
    )
}

/// True when a table of this family exists under the owned name, with no
/// marker validation: fresh-create intent must refuse any same-named table,
/// whoever owns it.
pub(crate) fn name_present(family: Family, budget: &Budget, reserve: Duration) -> Result<bool> {
    name_present_with(family, &table_name(), budget, reserve, &mut |batch| {
        bounded_transport(budget, reserve, batch)
    })
}

/// One real transport exchange bounded by the operation budget: the socket
/// receive timeout is the remaining budget slice, so a delayed reply can
/// never extend the deadline.
fn bounded_transport(budget: &Budget, reserve: Duration, batch: &[u8]) -> io::Result<Vec<u8>> {
    let wait = budget.slice(RECV_DEADLINE, reserve).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "operation budget exhausted before the exchange",
        )
    })?;
    Netlink::open()?.transact(batch, wait)
}

/// Reply transport, injectable so batch/reply handling is testable without a
/// socket or CAP_NET_ADMIN. One call per exchange: it sends the batch and
/// returns the accumulated reply stream (netlink messages are
/// self-delimiting, so multiple datagrams concatenate cleanly).
type Transact<'a> = dyn FnMut(&[u8]) -> io::Result<Vec<u8>> + 'a;

fn io_to_err(e: io::Error) -> Error {
    Error::Io(e)
}

/// Runs one exchange under the operation budget: the remaining time is
/// checked before the send, and a transport that reports an expired wait
/// maps to the typed [`Error::TimedOut`]. A received reply never extends
/// the budget.
fn exchange(
    budget: &Budget,
    reserve: Duration,
    batch: &[u8],
    tx: &mut Transact,
) -> Result<Vec<u8>> {
    let _ = budget.slice(RECV_DEADLINE, reserve).ok_or_else(|| {
        Error::TimedOut(
            "nft operation budget exhausted before the exchange; kernel state is uncertain and the journal is kept for retry".into(),
        )
    })?;
    match tx(batch) {
        Ok(buf) => Ok(buf),
        Err(e) if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock => {
            Err(Error::TimedOut(format!(
                "nft exchange did not complete within the remaining operation budget: {e}"
            )))
        }
        Err(e) => Err(io_to_err(e)),
    }
}

fn nft_type(msg: u16) -> u16 {
    (NFNL_SUBSYS_NFTABLES << 8) | msg
}

/// Which requests the current exchange carries, by sequence number.
type Requests = Vec<(u32, u16)>;

/// Feed outcome for one reply datagram.
enum Feed {
    /// Every awaited ack arrived and any required GETTABLE result decoded.
    Complete,
    /// More datagrams are needed before the exchange is judged.
    NeedMore,
    /// A kernel error or an unusable reply ended the exchange.
    Failed(String),
}

/// One decoded GETTABLE result: family byte, table name, opaque marker
/// bytes, and the 64-bit kernel handle.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TableInfo {
    family: u8,
    name: String,
    userdata: Option<Vec<u8>>,
    handle: u64,
}

/// Accumulates one exchange's replies until the awaited acks and any
/// GETTABLE result arrived, a kernel error ends it, or the receive deadline
/// stops the caller.
struct Replies {
    requests: Requests,
    pending: BTreeSet<u32>,
    want_table: bool,
    table: Option<TableInfo>,
    absent: bool,
    /// (failed request type, raw negative errno, description)
    error: Option<(u16, i32, String)>,
    buf: Vec<u8>,
}

impl Replies {
    fn new(requests: Requests, acks: &[u32], want_table: bool) -> Self {
        Self {
            requests,
            pending: acks.iter().copied().collect(),
            want_table,
            table: None,
            absent: false,
            error: None,
            buf: Vec::new(),
        }
    }

    fn fail(&mut self, failed_type: u16, errno: i32, what: String) -> Feed {
        self.error = Some((failed_type, errno, what.clone()));
        Feed::Failed(what)
    }

    fn complete(&self) -> bool {
        if self.error.is_some() {
            return true;
        }
        self.pending.is_empty() && (!self.want_table || self.table.is_some() || self.absent)
    }

    /// Feeds one reply datagram into the exchange. Correlates acks by the
    /// echoed request sequence, records a confirmed DELTABLE/GETTABLE
    /// absence, decodes a GETTABLE table record, and stops at the first
    /// kernel error or unusable record.
    fn feed(&mut self, datagram: &[u8]) -> Feed {
        if let Some((_, _, what)) = &self.error {
            return Feed::Failed(what.clone());
        }
        self.buf.extend_from_slice(datagram);
        let messages = match strict_messages(datagram) {
            Ok(m) => m,
            Err(what) => return self.fail(0, 0, what),
        };
        for msg in messages {
            if msg.type_ == NLMSG_ERROR {
                if msg.payload.len() < 4 {
                    return self.fail(0, 0, "error record below nlmsgerr size".into());
                }
                let errno = i32::from_ne_bytes([
                    msg.payload[0],
                    msg.payload[1],
                    msg.payload[2],
                    msg.payload[3],
                ]);
                if errno == 0 {
                    // Ack: nlmsgerr carries the echoed request header at
                    // offset 4; nlmsg_seq sits at payload[12..16].
                    if msg.payload.len() < 16 {
                        return self.fail(
                            0,
                            0,
                            "ack record without the echoed request header".into(),
                        );
                    }
                    let seq = u32::from_ne_bytes([
                        msg.payload[12],
                        msg.payload[13],
                        msg.payload[14],
                        msg.payload[15],
                    ]);
                    if !self.pending.remove(&seq) {
                        return self.fail(0, 0, format!("ack for unrequested sequence {seq}"));
                    }
                    continue;
                }
                let failed_type = if msg.payload.len() >= 10 {
                    u16::from_ne_bytes([msg.payload[8], msg.payload[9]])
                } else {
                    0
                };
                let failed_seq = if msg.payload.len() >= 16 {
                    u32::from_ne_bytes([
                        msg.payload[12],
                        msg.payload[13],
                        msg.payload[14],
                        msg.payload[15],
                    ])
                } else {
                    0
                };
                if errno == -libc::ENOENT
                    && (failed_type == nft_type(NFT_MSG_DELTABLE)
                        || failed_type == nft_type(NFT_MSG_GETTABLE))
                {
                    // The addressed object is confirmed absent; the ack for
                    // its sequence never arrives.
                    self.absent = true;
                    self.pending.remove(&failed_seq);
                    continue;
                }
                let name =
                    msg_type_name(failed_request_type(failed_seq, failed_type, &self.requests));
                let io = io::Error::from_raw_os_error(-errno);
                return self.fail(failed_type, errno, format!("kernel rejected {name}: {io}"));
            }
            if self.want_table && msg.type_ == nft_type(NFT_MSG_NEWTABLE) {
                match decode_table(msg.payload) {
                    Ok(info) => self.table = Some(info),
                    Err(what) => return self.fail(0, 0, what),
                }
            }
        }
        if self.complete() {
            Feed::Complete
        } else {
            Feed::NeedMore
        }
    }
}

fn msg_type_name(t: u16) -> &'static str {
    match t {
        t if t == nft_type(NFT_MSG_DELTABLE) => "DELTABLE",
        t if t == nft_type(NFT_MSG_NEWTABLE) => "NEWTABLE",
        t if t == nft_type(NFT_MSG_NEWCHAIN) => "NEWCHAIN",
        t if t == nft_type(NFT_MSG_NEWRULE) => "NEWRULE",
        t if t == nft_type(NFT_MSG_GETTABLE) => "GETTABLE",
        t if t == NFNL_MSG_BATCH_BEGIN => "BATCH_BEGIN",
        t if t == NFNL_MSG_BATCH_END => "BATCH_END",
        _ => "request",
    }
}

fn failed_request_type(failed_seq: u32, failed_type: u16, requests: &Requests) -> u16 {
    if failed_type != 0 {
        return failed_type;
    }
    requests
        .iter()
        .find(|(seq, _)| *seq == failed_seq)
        .map(|(_, t)| *t)
        .unwrap_or(0)
}

fn seq_names(pending: &BTreeSet<u32>, requests: &Requests) -> String {
    pending
        .iter()
        .map(|seq| {
            msg_type_name(
                requests
                    .iter()
                    .find(|(s, _)| s == seq)
                    .map(|(_, t)| *t)
                    .unwrap_or(0),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// GETTABLE lookup: Ok(None) only after a confirmed ENOENT. Errors on any
/// other kernel errno, an unusable reply, or a family/name mismatch. The
/// decoded handle is required: a table without one cannot be deleted safely.
fn lookup_with(
    family: Family,
    table: &str,
    budget: &Budget,
    reserve: Duration,
    tx: &mut Transact,
) -> Result<Option<TableInfo>> {
    let msg = Msg::new(nft_type(NFT_MSG_GETTABLE), NLM_F_REQUEST | NLM_F_ACK, 1)
        .with_nfgen(family.nfproto())
        .attr_bytes(NFTA_TABLE_NAME, &nul_name(table));
    let mut acc = Replies::new(vec![(1, nft_type(NFT_MSG_GETTABLE))], &[1], true);
    let stream = exchange(budget, reserve, &msg.into_raw(), tx)?;
    match acc.feed(&stream) {
        Feed::Complete => {}
        Feed::NeedMore => {
            return Err(Error::Firewall(format!(
                "nft lookup {}: incomplete GETTABLE reply",
                family.label()
            )));
        }
        Feed::Failed(what) => {
            return Err(Error::Firewall(format!(
                "nft lookup {}: {what}",
                family.label()
            )));
        }
    }
    if acc.absent {
        return Ok(None);
    }
    let info = acc
        .table
        .expect("a complete GETTABLE exchange carries the table record");
    if info.family != family.nfproto() {
        return Err(Error::Firewall(format!(
            "nft lookup {}: GETTABLE returned family byte {}",
            family.label(),
            info.family
        )));
    }
    if info.name != table {
        return Err(Error::Firewall(format!(
            "nft lookup {}: GETTABLE returned table '{}'",
            family.label(),
            info.name
        )));
    }
    Ok(Some(info))
}

/// Marker gate shared by install, remove, and present: the present table
/// must carry exactly this session's 16 opaque bytes. Refusals name the
/// operation, family, and table.
fn check_marker(
    op: &str,
    family: Family,
    table: &str,
    session: &Uuid,
    info: &TableInfo,
) -> Result<()> {
    match &info.userdata {
        None => Err(Error::Firewall(format!(
            "nft {op} {}: table '{table}' carries no session marker; refusing to adopt it",
            family.label()
        ))),
        Some(marker) if marker.as_slice() != session.as_bytes() => Err(Error::Firewall(format!(
            "nft {op} {}: table '{table}' carries a different session marker; refusing to touch it",
            family.label()
        ))),
        Some(_) => Ok(()),
    }
}

fn install_with(
    family: Family,
    queue: u16,
    fwmark: u32,
    table: &str,
    session: &Uuid,
    budget: &Budget,
    tx: &mut Transact,
) -> Result<()> {
    // Fresh create only: the exclusive-create batch is sent unconditionally.
    // A same-named table is never adopted, whatever marker it carries; a
    // concurrent creator surfaces as the typed EEXIST conflict below.
    let plan = InstallPlan::new(family, queue, fwmark, table, session);
    let batch = plan.build();
    let mut acc = collector_for(&batch);
    let stream = exchange(budget, ROLLBACK_RESERVE, &batch, tx)?;
    acc.feed(&stream);
    install_verdict(family, table, &acc)
}

/// Install outcome: EEXIST on NEWTABLE refuses with the existing object
/// named; any other kernel error or a missing ack fails the install.
fn install_verdict(family: Family, table: &str, acc: &Replies) -> Result<()> {
    if let Some((failed_type, errno, what)) = &acc.error {
        if *failed_type == nft_type(NFT_MSG_NEWTABLE) && *errno == -libc::EEXIST {
            return Err(Error::Conflict(format!(
                "nft install {}: table '{table}' already exists ({}); refusing to replace it",
                family.label(),
                io::Error::from_raw_os_error(-errno)
            )));
        }
        return Err(Error::Firewall(format!(
            "nft install {}: {what}",
            family.label()
        )));
    }
    if !acc.pending.is_empty() {
        return Err(Error::Firewall(format!(
            "nft install {}: incomplete reply set; missing acks for {}",
            family.label(),
            seq_names(&acc.pending, &acc.requests)
        )));
    }
    Ok(())
}

fn remove_with(
    family: Family,
    table: &str,
    session: &Uuid,
    budget: &Budget,
    tx: &mut Transact,
) -> Result<()> {
    let Some(info) = lookup_with(family, table, budget, Duration::ZERO, tx)? else {
        // Confirmed absence is already the desired state.
        return Ok(());
    };
    check_marker("remove", family, table, session, &info)?;

    let mut seq = 1;
    let mut batch = Msg::new(NFNL_MSG_BATCH_BEGIN, NLM_F_REQUEST, seq);
    seq += 1;
    batch.nfgen(0, NFNETLINK_V0, NFNL_SUBSYS_NFTABLES);
    let mut raw = batch.into_raw();
    let mut del = Msg::new(nft_type(NFT_MSG_DELTABLE), NLM_F_REQUEST | NLM_F_ACK, seq)
        .with_nfgen(family.nfproto());
    // 64-bit attributes are 8-byte aligned on the wire the way nla_put_be64
    // does it: a zero-length NFTA_TABLE_PAD filler goes in front when the
    // value would land misaligned in the kernel skb.
    if !(raw.len() + del.buf.len()).is_multiple_of(8) {
        del = del.attr_bytes(NFTA_TABLE_PAD, &[]);
    }
    del = del.attr_bytes(NFTA_TABLE_HANDLE, &info.handle.to_be_bytes());
    raw.extend(del.into_raw());
    raw.extend(
        Msg::new(NFNL_MSG_BATCH_END, NLM_F_REQUEST, seq + 1)
            .with_nfgen(0)
            .into_raw(),
    );
    let mut acc = Replies::new(
        vec![
            (1, NFNL_MSG_BATCH_BEGIN),
            (2, nft_type(NFT_MSG_DELTABLE)),
            (3, NFNL_MSG_BATCH_END),
        ],
        &[2],
        false,
    );
    let stream = exchange(budget, Duration::ZERO, &raw, tx)?;
    match acc.feed(&stream) {
        Feed::Complete => {
            if let Some(err) = acc.error.as_ref()
                && !acc.absent
            {
                return Err(Error::Firewall(format!(
                    "nft remove {}: {}",
                    family.label(),
                    err.2
                )));
            }
        }
        Feed::NeedMore => {
            return Err(Error::Firewall(format!(
                "nft remove {}: incomplete DELTABLE reply",
                family.label()
            )));
        }
        Feed::Failed(what) => {
            return Err(Error::Firewall(format!(
                "nft remove {}: {what}",
                family.label()
            )));
        }
    }

    // Confirm the deletion landed: the same-named object must be gone.
    if lookup_with(family, table, budget, Duration::ZERO, tx)?.is_some() {
        return Err(Error::Firewall(format!(
            "nft remove {}: table '{table}' is still present after DELTABLE",
            family.label()
        )));
    }
    Ok(())
}

fn present_with(
    family: Family,
    table: &str,
    session: &Uuid,
    budget: &Budget,
    reserve: Duration,
    tx: &mut Transact,
) -> Result<bool> {
    match lookup_with(family, table, budget, reserve, tx)? {
        None => Ok(false),
        Some(info) => {
            check_marker("present", family, table, session, &info)?;
            Ok(true)
        }
    }
}

/// Injectable body of [`name_present`]: a GETTABLE absence is Ok(false),
/// any decodable table record under the name is Ok(true). No marker gate.
fn name_present_with(
    family: Family,
    table: &str,
    budget: &Budget,
    reserve: Duration,
    tx: &mut Transact,
) -> Result<bool> {
    Ok(lookup_with(family, table, budget, reserve, tx)?.is_some())
}

/// Create-only install plan: NEWTABLE with the session marker, then the
/// chains and rules. No leading DELTABLE — an existing table is never
/// replaced by name. The `ip6` batch carries the marked table only.
struct InstallPlan<'a> {
    family: Family,
    queue: u16,
    fwmark: u32,
    table: &'a str,
    session: &'a Uuid,
}

impl<'a> InstallPlan<'a> {
    fn new(family: Family, queue: u16, fwmark: u32, table: &'a str, session: &'a Uuid) -> Self {
        Self {
            family,
            queue,
            fwmark,
            table,
            session,
        }
    }

    fn build(&self) -> Vec<u8> {
        let mut seq = 1;
        let mut batch = Msg::new(NFNL_MSG_BATCH_BEGIN, NLM_F_REQUEST, seq);
        seq += 1;
        batch.nfgen(0, NFNETLINK_V0, NFNL_SUBSYS_NFTABLES);
        let mut raw = batch.into_raw();
        raw.extend(
            Msg::new(
                nft_type(NFT_MSG_NEWTABLE),
                NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
                seq,
            )
            .with_nfgen(self.family.nfproto())
            .attr_bytes(NFTA_TABLE_NAME, &nul_name(self.table))
            // The session marker travels as opaque bytes.
            .attr_bytes(NFTA_TABLE_USERDATA, self.session.as_bytes())
            .into_raw(),
        );
        seq += 1;
        if self.family == Family::V6 {
            // Ownership artifact only: the marked table, nothing else.
            raw.extend(
                Msg::new(NFNL_MSG_BATCH_END, NLM_F_REQUEST, seq)
                    .with_nfgen(0)
                    .into_raw(),
            );
            return raw;
        }
        // The jump target exists before the jumps reference it; one
        // transaction makes the order irrelevant to the kernel.
        raw.extend(self.chain_msg(seq, GUARD_CHAIN, None).into_raw());
        seq += 1;
        raw.extend(
            self.chain_msg(seq, OUTPUT_CHAIN, Some(NF_INET_LOCAL_OUT))
                .into_raw(),
        );
        seq += 1;
        raw.extend(
            self.chain_msg(seq, PREROUTING_CHAIN, Some(NF_INET_PRE_ROUTING))
                .into_raw(),
        );
        seq += 1;
        raw.extend(
            self.rule_msg(seq, OUTPUT_CHAIN, &[immediate_jump(GUARD_CHAIN)])
                .into_raw(),
        );
        seq += 1;
        raw.extend(
            self.rule_msg(seq, PREROUTING_CHAIN, &[immediate_jump(GUARD_CHAIN)])
                .into_raw(),
        );
        seq += 1;
        raw.extend(
            self.rule_msg(seq, GUARD_CHAIN, &guard_rule_exprs(self.fwmark))
                .into_raw(),
        );
        seq += 1;
        for (direction, offset) in [
            (IP_CT_DIR_ORIGINAL, 2u32), // tcp dport: original direction
            (IP_CT_DIR_REPLY, 0u32),    // tcp sport: reply direction
        ] {
            raw.extend(
                self.rule_msg(
                    seq,
                    GUARD_CHAIN,
                    &tcp_rule_exprs(direction, offset, self.queue),
                )
                .into_raw(),
            );
            seq += 1;
        }
        for offset in [2u32, 0u32] {
            raw.extend(
                self.rule_msg(seq, GUARD_CHAIN, &udp_rule_exprs(offset, self.queue))
                    .into_raw(),
            );
            seq += 1;
        }
        raw.extend(
            Msg::new(NFNL_MSG_BATCH_END, NLM_F_REQUEST, seq)
                .with_nfgen(0)
                .into_raw(),
        );
        raw
    }

    fn chain_msg(&self, seq: u32, name: &str, hook: Option<u32>) -> Msg {
        let mut m = Msg::new(
            nft_type(NFT_MSG_NEWCHAIN),
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
            seq,
        )
        .with_nfgen(self.family.nfproto())
        .attr_bytes(NFTA_CHAIN_TABLE, &nul_name(self.table))
        .attr_bytes(NFTA_CHAIN_NAME, &nul_name(name));
        if let Some(hooknum) = hook {
            let mut hook_nest = Nest::new();
            hook_nest.attr_u32be(NFTA_HOOK_HOOKNUM, hooknum);
            hook_nest.attr_u32be(NFTA_HOOK_PRIORITY, NF_IP_PRI_MANGLE as u32);
            m = m.attr_raw(NFTA_CHAIN_HOOK, &hook_nest.into_raw());
            m = m.attr_u32be(NFTA_CHAIN_POLICY, NF_ACCEPT);
            m = m.attr_bytes(NFTA_CHAIN_TYPE, b"filter\0");
        }
        m
    }

    fn rule_msg(&self, seq: u32, chain: &str, exprs: &[Vec<u8>]) -> Msg {
        let mut list = Nest::new();
        for expr in exprs {
            list.attr_raw(NFTA_LIST_ELEM, expr);
        }
        Msg::new(
            nft_type(NFT_MSG_NEWRULE),
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_APPEND,
            seq,
        )
        .with_nfgen(self.family.nfproto())
        .attr_bytes(NFTA_RULE_TABLE, &nul_name(self.table))
        .attr_bytes(NFTA_RULE_CHAIN, &nul_name(chain))
        .attr_raw(NFTA_RULE_EXPRESSIONS, &list.into_raw())
    }
}

/// String attributes are NUL-terminated on the wire (NLA_STRING, matched by
/// the kernel with nla_strscpy).
fn nul_name(name: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(name.len() + 1);
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    bytes
}

/// Attribute stream without a netlink header: the payload of one nested
/// attribute.
struct Nest {
    buf: Vec<u8>,
}

impl Nest {
    fn new() -> Nest {
        Nest {
            buf: Vec::with_capacity(32),
        }
    }

    fn start_attr(&mut self, type_: u16) -> usize {
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf.extend_from_slice(&type_.to_ne_bytes());
        self.buf.len() - 4
    }

    fn end_attr(&mut self, offset: usize) {
        let len = (self.buf.len() - offset) as u16;
        self.buf[offset..offset + 2].copy_from_slice(&len.to_ne_bytes());
        let pad = (NLA_ALIGNTO - (self.buf.len() % NLA_ALIGNTO)) % NLA_ALIGNTO;
        self.buf.extend(std::iter::repeat_n(0u8, pad));
    }

    fn attr_bytes(&mut self, type_: u16, bytes: &[u8]) -> &mut Self {
        let off = self.start_attr(type_);
        self.buf.extend_from_slice(bytes);
        self.end_attr(off);
        self
    }

    fn attr_raw(&mut self, type_: u16, nested: &[u8]) -> &mut Self {
        self.attr_bytes(type_, nested)
    }

    fn attr_u32be(&mut self, type_: u16, v: u32) -> &mut Self {
        self.attr_bytes(type_, &v.to_be_bytes())
    }

    fn attr_u16be(&mut self, type_: u16, v: u16) -> &mut Self {
        self.attr_bytes(type_, &v.to_be_bytes())
    }

    fn attr_u8(&mut self, type_: u16, v: u8) -> &mut Self {
        self.attr_bytes(type_, &[v])
    }

    fn into_raw(self) -> Vec<u8> {
        self.buf
    }
}

/// One netlink message: nlmsghdr + optional nfgenmsg + attributes.
struct Msg {
    buf: Vec<u8>,
}

impl Msg {
    fn new(type_: u16, flags: u16, seq: u32) -> Msg {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_len backpatched
        buf.extend_from_slice(&type_.to_ne_bytes());
        buf.extend_from_slice(&flags.to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_pid
        Msg { buf }
    }

    fn nfgen(&mut self, family: u8, version: u8, res_id: u16) -> &mut Self {
        self.buf.push(family);
        self.buf.push(version);
        self.buf.extend_from_slice(&res_id.to_be_bytes());
        self
    }

    fn with_nfgen(mut self, family: u8) -> Msg {
        self.nfgen(family, 0, NFNL_SUBSYS_NFTABLES);
        self
    }

    fn start_attr(&mut self, type_: u16) -> usize {
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf.extend_from_slice(&type_.to_ne_bytes());
        self.buf.len() - 4
    }

    fn end_attr(&mut self, offset: usize) {
        let len = (self.buf.len() - offset) as u16;
        self.buf[offset..offset + 2].copy_from_slice(&len.to_ne_bytes());
        let pad = (NLA_ALIGNTO - (self.buf.len() % NLA_ALIGNTO)) % NLA_ALIGNTO;
        self.buf.extend(std::iter::repeat_n(0u8, pad));
    }

    fn attr_bytes(mut self, type_: u16, bytes: &[u8]) -> Self {
        let off = self.start_attr(type_);
        self.buf.extend_from_slice(bytes);
        self.end_attr(off);
        self
    }

    fn attr_raw(self, type_: u16, nested: &[u8]) -> Self {
        self.attr_bytes(type_, nested)
    }

    fn attr_u32be(self, type_: u16, v: u32) -> Self {
        self.attr_bytes(type_, &v.to_be_bytes())
    }

    fn into_raw(mut self) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[0..4].copy_from_slice(&len.to_ne_bytes());
        self.buf
    }
}

/// One expression element: NFTA_LIST_ELEM { NFTA_EXPR_NAME, NFTA_EXPR_DATA }.
fn expr(name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut elem = Nest::new();
    elem.attr_bytes(NFTA_EXPR_NAME, name);
    elem.attr_raw(NFTA_EXPR_DATA, data);
    elem.into_raw()
}

fn data_value(bytes: &[u8]) -> Vec<u8> {
    let mut d = Nest::new();
    d.attr_bytes(NFTA_DATA_VALUE, bytes);
    d.into_raw()
}

fn verdict(code: i32, chain: Option<&str>) -> Vec<u8> {
    let mut v = Nest::new();
    v.attr_u32be(NFTA_VERDICT_CODE, code as u32);
    if let Some(c) = chain {
        v.attr_bytes(NFTA_VERDICT_CHAIN, &nul_name(c));
    }
    let mut d = Nest::new();
    d.attr_raw(NFTA_DATA_VERDICT, &v.into_raw());
    d.into_raw()
}

fn immediate_jump(chain: &str) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_IMMEDIATE_DREG, NFT_REG_VERDICT);
    data.attr_raw(NFTA_IMMEDIATE_DATA, &verdict(NFT_JUMP, Some(chain)));
    expr(b"immediate\0", &data.into_raw())
}

/// Converts a register from host to network byte order in place. The kernel
/// stores `ct packets` in host order and compares registers with `memcmp`.
fn byteorder_hton(reg: u32, size: u32) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_BYTEORDER_SREG, reg);
    data.attr_u32be(NFTA_BYTEORDER_DREG, reg);
    data.attr_u32be(NFTA_BYTEORDER_OP, NFT_BYTEORDER_HTON);
    data.attr_u32be(NFTA_BYTEORDER_LEN, size);
    data.attr_u32be(NFTA_BYTEORDER_SIZE, size);
    expr(b"byteorder\0", &data.into_raw())
}

fn meta_load(dreg: u32, key: u32) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_META_DREG, dreg);
    data.attr_u32be(NFTA_META_KEY, key);
    expr(b"meta\0", &data.into_raw())
}

/// The kernel reads SREG, DREG, and the bitwise length with nla_get_be32,
/// so those go on the wire big-endian; the mask and xor are register data
/// blobs and travel in host order (reference: `nft -j list table`).
fn bitwise_mask(sreg: u32, dreg: u32, len: u32, mask: u32) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_BITWISE_SREG, sreg);
    data.attr_u32be(NFTA_BITWISE_DREG, dreg);
    data.attr_u32be(NFTA_BITWISE_LEN, len);
    data.attr_raw(NFTA_BITWISE_MASK, &data_value(&mask.to_ne_bytes()));
    data.attr_raw(NFTA_BITWISE_XOR, &data_value(&0u32.to_ne_bytes()));
    expr(b"bitwise\0", &data.into_raw())
}

fn cmp(sreg: u32, op: u32, value: &[u8]) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_CMP_SREG, sreg);
    data.attr_u32be(NFTA_CMP_OP, op);
    data.attr_raw(NFTA_CMP_DATA, &data_value(value));
    expr(b"cmp\0", &data.into_raw())
}

fn payload_load(dreg: u32, offset: u32, len: u32) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_PAYLOAD_DREG, dreg);
    data.attr_u32be(NFTA_PAYLOAD_BASE, NFT_PAYLOAD_TRANSPORT_HEADER);
    data.attr_u32be(NFTA_PAYLOAD_OFFSET, offset);
    data.attr_u32be(NFTA_PAYLOAD_LEN, len);
    expr(b"payload\0", &data.into_raw())
}

fn ct_pkts(direction: u8, dreg: u32) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u32be(NFTA_CT_DREG, dreg);
    data.attr_u32be(NFTA_CT_KEY, NFT_CT_PKTS);
    data.attr_u8(NFTA_CT_DIRECTION, direction);
    expr(b"ct\0", &data.into_raw())
}

fn queue_stmt(queue: u16) -> Vec<u8> {
    let mut data = Nest::new();
    data.attr_u16be(NFTA_QUEUE_NUM, queue);
    data.attr_u16be(NFTA_QUEUE_FLAGS, NFT_QUEUE_FLAG_BYPASS);
    expr(b"queue\0", &data.into_raw())
}

/// meta mark & fwmark == fwmark accept
fn guard_rule_exprs(fwmark: u32) -> Vec<Vec<u8>> {
    let mut accept = Nest::new();
    accept.attr_u32be(NFTA_IMMEDIATE_DREG, NFT_REG_VERDICT);
    accept.attr_raw(NFTA_IMMEDIATE_DATA, &verdict(NF_ACCEPT as i32, None));
    vec![
        meta_load(NFT_REG32_00, NFT_META_MARK),
        bitwise_mask(NFT_REG32_00, NFT_REG32_00, 4, fwmark),
        cmp(NFT_REG32_00, NFT_CMP_EQ, &fwmark.to_ne_bytes()),
        expr(b"immediate\0", &accept.into_raw()),
    ]
}

/// `ct packets` 1..=12 (direction) converted in place to network order
/// before the big-endian bounds compare, l4proto tcp, dport/sport 443,
/// queue bypass.
fn tcp_rule_exprs(direction: u8, port_offset: u32, queue: u16) -> Vec<Vec<u8>> {
    vec![
        ct_pkts(direction, NFT_REG32_00),
        byteorder_hton(NFT_REG32_00, 8),
        cmp(NFT_REG32_00, NFT_CMP_GTE, &CT_PACKET_LOW.to_be_bytes()),
        cmp(NFT_REG32_00, NFT_CMP_LTE, &CT_PACKET_HIGH.to_be_bytes()),
        meta_load(NFT_REG32_00, NFT_META_L4PROTO),
        cmp(NFT_REG32_00, NFT_CMP_EQ, &[6]),
        payload_load(NFT_REG32_00, port_offset, 2),
        cmp(NFT_REG32_00, NFT_CMP_EQ, &443u16.to_be_bytes()),
        queue_stmt(queue),
    ]
}

/// l4proto udp, dport/sport 443, queue bypass (no conntrack gate).
fn udp_rule_exprs(port_offset: u32, queue: u16) -> Vec<Vec<u8>> {
    vec![
        meta_load(NFT_REG32_00, NFT_META_L4PROTO),
        cmp(NFT_REG32_00, NFT_CMP_EQ, &[17]),
        payload_load(NFT_REG32_00, port_offset, 2),
        cmp(NFT_REG32_00, NFT_CMP_EQ, &443u16.to_be_bytes()),
        queue_stmt(queue),
    ]
}

/// Walks a reply buffer as aligned netlink messages, rejecting truncated
/// headers, short lengths, and missing message padding.
fn strict_messages(buf: &[u8]) -> std::result::Result<Vec<NlMsg<'_>>, String> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < buf.len() {
        if off + 16 > buf.len() {
            return Err("truncated netlink header".into());
        }
        let len = u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        if len < 16 {
            return Err(format!("netlink length {len} below header size"));
        }
        if off + len > buf.len() {
            return Err("truncated netlink message".into());
        }
        out.push(NlMsg {
            type_: u16::from_ne_bytes([buf[off + 4], buf[off + 5]]),
            flags: u16::from_ne_bytes([buf[off + 6], buf[off + 7]]),
            seq: u32::from_ne_bytes([buf[off + 8], buf[off + 9], buf[off + 10], buf[off + 11]]),
            payload: &buf[off + 16..off + len],
        });
        off += (len + NLA_ALIGNTO - 1) & !(NLA_ALIGNTO - 1);
        if off > buf.len() {
            return Err("truncated netlink padding".into());
        }
    }
    Ok(out)
}

struct NlMsg<'a> {
    type_: u16,
    flags: u16,
    seq: u32,
    payload: &'a [u8],
}

/// Walks a netlink message payload as aligned nlattr records, rejecting
/// truncated headers, short lengths, and missing attribute padding.
fn parse_attrs(payload: &[u8]) -> std::result::Result<Vec<(u16, &[u8])>, String> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < payload.len() {
        if off + 4 > payload.len() {
            return Err("truncated attribute header".into());
        }
        let len = u16::from_ne_bytes([payload[off], payload[off + 1]]) as usize;
        if len < 4 {
            return Err(format!("attribute length {len} below header size"));
        }
        let type_ = u16::from_ne_bytes([payload[off + 2], payload[off + 3]]);
        if off + len > payload.len() {
            return Err("truncated attribute payload".into());
        }
        out.push((type_, &payload[off + 4..off + len]));
        off += (len + NLA_ALIGNTO - 1) & !(NLA_ALIGNTO - 1);
        if off > payload.len() {
            return Err("truncated attribute padding".into());
        }
    }
    Ok(out)
}

/// Decodes a NEWTABLE reply record: nfgenmsg family byte, NUL-terminated
/// name, opaque marker bytes, and the required 64-bit kernel handle.
fn decode_table(payload: &[u8]) -> std::result::Result<TableInfo, String> {
    if payload.len() < 4 {
        return Err("NEWTABLE reply below nfgenmsg size".into());
    }
    let family = payload[0];
    let mut name: Option<String> = None;
    let mut userdata: Option<Vec<u8>> = None;
    let mut handle: Option<u64> = None;
    for (type_, value) in parse_attrs(&payload[4..])? {
        match type_ {
            NFTA_TABLE_NAME => {
                let bytes = value.strip_suffix(&[0u8]).unwrap_or(value);
                let decoded = String::from_utf8(bytes.to_vec())
                    .map_err(|_| "NEWTABLE name is not UTF-8".to_string())?;
                name = Some(decoded);
            }
            NFTA_TABLE_USERDATA => userdata = Some(value.to_vec()),
            NFTA_TABLE_HANDLE => {
                let bytes: [u8; 8] = value
                    .try_into()
                    .map_err(|_| "NFTA_TABLE_HANDLE is not 8 bytes".to_string())?;
                handle = Some(u64::from_be_bytes(bytes));
            }
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "NEWTABLE reply carries no name".to_string())?;
    let handle = handle.ok_or_else(|| "NEWTABLE reply carries no kernel handle".to_string())?;
    Ok(TableInfo {
        family,
        name,
        userdata,
        handle,
    })
}

/// Reply expectations derived from the batch itself: every message flagged
/// NLM_F_ACK awaits an ack, a GETTABLE awaits its result record.
fn collector_for(batch: &[u8]) -> Replies {
    let mut requests: Requests = Vec::new();
    let mut acks: Vec<u32> = Vec::new();
    let mut want_table = false;
    for msg in strict_messages(batch).expect("built batches parse") {
        requests.push((msg.seq, msg.type_));
        if msg.flags & NLM_F_ACK != 0 {
            acks.push(msg.seq);
        }
        if msg.type_ == nft_type(NFT_MSG_GETTABLE) {
            want_table = true;
        }
    }
    Replies::new(requests, &acks, want_table)
}

struct Netlink {
    fd: RawFd,
}

impl Netlink {
    fn open() -> io::Result<Netlink> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                NETLINK_NETFILTER,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // nl_pad is an opaque Padding<u16> in this libc; only zeroed
        // construction is possible, which matches the wire requirement.
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::c_ushort;
        addr.nl_pid = 0;
        addr.nl_groups = 0;
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(Netlink { fd })
    }

    /// Sends one batch, then consumes reply datagrams until every
    /// ack-flagged request in the batch is answered and any required
    /// GETTABLE result arrived. Three rules govern the exchange: a valid
    /// kernel `NLMSG_ERROR` is data, not a transport failure (the batch
    /// stream is returned and the caller classifies the errno); the socket
    /// receive timeout shrinks with the remaining budget before every
    /// datagram, so a delayed reply can never extend the caller's budget;
    /// and `EINTR` on send or receive repeats while time remains. A wrong
    /// sender, a truncated datagram, or the absolute deadline ends the
    /// call; incomplete success is an error, never a result.
    fn transact(&mut self, batch: &[u8], wait: Duration) -> io::Result<Vec<u8>> {
        let deadline = Instant::now() + wait;
        let iov = libc::iovec {
            iov_base: batch.as_ptr() as *mut libc::c_void,
            iov_len: batch.len(),
        };
        let mut msg = unsafe { std::mem::zeroed::<libc::msghdr>() };
        msg.msg_name = std::ptr::null_mut();
        msg.msg_namelen = 0;
        msg.msg_iov = &iov as *const libc::iovec as *mut libc::iovec;
        msg.msg_iovlen = 1;
        msg.msg_control = std::ptr::null_mut();
        msg.msg_controllen = 0;
        msg.msg_flags = 0;
        loop {
            let sent = unsafe { libc::sendmsg(self.fd, &msg, 0) };
            if sent >= 0 {
                if sent as usize != batch.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "short netlink send",
                    ));
                }
                break;
            }
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted
                && !deadline.saturating_duration_since(Instant::now()).is_zero()
            {
                continue;
            }
            return Err(e);
        }
        let mut acc = collector_for(batch);
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(receive_deadline_passed());
            }
            let timeout = recv_timeout(remaining);
            let rc = unsafe {
                libc::setsockopt(
                    self.fd,
                    libc::SOL_SOCKET,
                    libc::SO_RCVTIMEO,
                    &timeout as *const libc::timeval as *const libc::c_void,
                    std::mem::size_of::<libc::timeval>() as libc::socklen_t,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
            let mut iov = libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };
            let mut msg = unsafe { std::mem::zeroed::<libc::msghdr>() };
            msg.msg_name = &mut addr as *mut libc::sockaddr_nl as *mut libc::c_void;
            msg.msg_namelen = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
            msg.msg_iov = &mut iov as *mut libc::iovec;
            msg.msg_iovlen = 1;
            msg.msg_control = std::ptr::null_mut();
            msg.msg_controllen = 0;
            msg.msg_flags = 0;
            let n = unsafe { libc::recvmsg(self.fd, &mut msg, 0) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
                    return Err(receive_deadline_passed());
                }
                return Err(e);
            }
            if msg.msg_flags & libc::MSG_TRUNC != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "netlink reply datagram truncated",
                ));
            }
            // Kernel replies carry nl_pid 0; anything else is not ours.
            if addr.nl_pid != 0 {
                return Err(io::Error::other(format!(
                    "netlink reply from non-kernel sender pid {}",
                    addr.nl_pid
                )));
            }
            match acc.feed(&buf[..n as usize]) {
                Feed::Complete => return Ok(acc.buf),
                Feed::Failed(what) => match acc.error {
                    // A valid kernel NLMSG_ERROR is data: the callers
                    // classify the errno from the accumulated stream.
                    Some((_, errno, _)) if errno != 0 => return Ok(acc.buf),
                    _ => return Err(io::Error::other(what)),
                },
                Feed::NeedMore => {}
            }
        }
    }
}

/// Per-datagram receive timeout: a zero remaining duration installs one
/// microsecond, since `{0, 0}` means an infinite wait on `SO_RCVTIMEO`.
fn recv_timeout(remaining: Duration) -> libc::timeval {
    let mut tv = libc::timeval {
        tv_sec: remaining.as_secs() as libc::c_long,
        tv_usec: libc::suseconds_t::from(remaining.subsec_micros()),
    };
    if tv.tv_sec == 0 && tv.tv_usec == 0 {
        tv.tv_usec = 1;
    }
    tv
}

fn receive_deadline_passed() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "receive deadline passed before complete replies",
    )
}

impl Drop for Netlink {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

// ---------------------------------------------------------------------------
// Tests: structural/decoded equivalence against the UAPI contract. No live
// kernel or CAP_NET_ADMIN is involved; the reply transport is scripted.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    fn session16() -> Uuid {
        Uuid::from_bytes([0x5a; 16])
    }

    fn msg_list(buf: &[u8]) -> Vec<NlMsg<'_>> {
        strict_messages(buf).expect("test replies parse")
    }

    /// Attributes of a nested attribute payload (no netlink header).
    fn attr_map(payload: &[u8]) -> Vec<(u16, &[u8])> {
        parse_attrs(payload).expect("test attributes parse")
    }

    /// Attributes of an nftables message payload; nfgenmsg occupies the
    /// first four bytes of every message this module sends.
    fn msg_attrs(payload: &[u8]) -> Vec<(u16, &[u8])> {
        parse_attrs(&payload[4..]).expect("test attributes parse")
    }

    fn find<'a>(attrs: &[(u16, &'a [u8])], type_: u16) -> &'a [u8] {
        match attrs.iter().find(|(t, _)| *t == type_) {
            Some((_, v)) => v,
            None => panic!(
                "attribute {type_:#06x} missing; present: {:?}",
                attrs
                    .iter()
                    .map(|(t, _)| format!("{t:#06x}"))
                    .collect::<Vec<_>>()
            ),
        }
    }

    fn u32be(v: &[u8]) -> u32 {
        u32::from_be_bytes([v[0], v[1], v[2], v[3]])
    }

    fn u16be(v: &[u8]) -> u16 {
        u16::from_be_bytes([v[0], v[1]])
    }

    fn u64be(v: &[u8]) -> u64 {
        u64::from_be_bytes([v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]])
    }

    /// Decodes expression element `index` of a rule into (name, data attrs).
    fn expr_of(payload: &[u8], index: usize) -> (Vec<u8>, Vec<(u16, &[u8])>) {
        let list = find(&msg_attrs(payload), NFTA_RULE_EXPRESSIONS);
        let elems = attr_map(list);
        assert!(elems.len() > index, "expression index in bounds");
        let elem = elems[index].1;
        let ea = attr_map(elem);
        (
            find(&ea, NFTA_EXPR_NAME).to_vec(),
            attr_map(find(&ea, NFTA_EXPR_DATA)),
        )
    }

    fn expr_count(payload: &[u8]) -> usize {
        let list = find(&msg_attrs(payload), NFTA_RULE_EXPRESSIONS);
        attr_map(list).len()
    }

    /// FIFO provider: replies are returned in listed order, one per exchange.
    struct ScriptedTx {
        replies: RefCell<VecDeque<Vec<u8>>>,
        seen: RefCell<Vec<Vec<u8>>>,
    }

    impl ScriptedTx {
        fn new(replies: Vec<Vec<u8>>) -> ScriptedTx {
            ScriptedTx {
                replies: RefCell::new(replies.into()),
                seen: RefCell::new(Vec::new()),
            }
        }

        fn call(&self, batch: &[u8]) -> io::Result<Vec<u8>> {
            self.seen.borrow_mut().push(batch.to_vec());
            match self.replies.borrow_mut().pop_front() {
                Some(r) => Ok(r),
                None => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "no scripted reply",
                )),
            }
        }
    }

    /// NLMSG_ERROR carrying nlmsgerr { error, failed nlmsghdr }.
    fn err_reply(errno: i32, failed_type: u16, failed_seq: u32) -> Vec<u8> {
        let mut m = Msg::new(NLMSG_ERROR, 0, failed_seq);
        m.buf.extend_from_slice(&errno.to_ne_bytes());
        m.buf.extend_from_slice(&0u32.to_ne_bytes()); // failed msg nlmsg_len
        m.buf.extend_from_slice(&failed_type.to_ne_bytes());
        m.buf.extend_from_slice(&0u16.to_ne_bytes()); // failed msg flags
        m.buf.extend_from_slice(&failed_seq.to_ne_bytes());
        m.buf.extend_from_slice(&0u32.to_ne_bytes()); // failed msg pid
        m.into_raw()
    }

    fn ok_acks(first: u32, last: u32) -> Vec<u8> {
        let mut batch = Vec::new();
        for s in first..=last {
            batch.extend(err_reply(0, 0, s));
        }
        batch
    }

    /// NEWTABLE reply record the way the kernel fills it: string name,
    /// optional marker, and a 64-bit handle preceded by NFTA_TABLE_PAD when
    /// the value would land misaligned (nla_put_be64 semantics).
    fn table_reply(
        family: Family,
        name: &[u8],
        userdata: Option<&[u8]>,
        handle: Option<u64>,
    ) -> Vec<u8> {
        let mut m = Msg::new(nft_type(NFT_MSG_NEWTABLE), 0, 1).with_nfgen(family.nfproto());
        m = m.attr_bytes(NFTA_TABLE_NAME, name);
        if let Some(u) = userdata {
            m = m.attr_bytes(NFTA_TABLE_USERDATA, u);
        }
        if let Some(h) = handle {
            if !m.buf.len().is_multiple_of(8) {
                m = m.attr_bytes(NFTA_TABLE_PAD, &[]);
            }
            m = m.attr_bytes(NFTA_TABLE_HANDLE, &h.to_be_bytes());
        }
        m.into_raw()
    }

    /// A full GETTABLE answer: the NEWTABLE record plus the terminating
    /// zero-errno ack for the GETTABLE sequence.
    fn gettable_reply(
        family: Family,
        name: &[u8],
        userdata: Option<&[u8]>,
        handle: Option<u64>,
    ) -> Vec<u8> {
        let mut stream = table_reply(family, name, userdata, handle);
        stream.extend(err_reply(0, 0, 1));
        stream
    }

    #[test]
    fn table_name_uses_euid_suffix() {
        let name = table_name();
        assert!(name.starts_with("v2ray_rs_"));
        assert_eq!(name, format!("v2ray_rs_{}", unsafe { libc::geteuid() }));
        assert!(name.len() < 256);
    }

    #[test]
    fn batch_framing_and_message_sequence() {
        let session = session16();
        let plan = InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_1000", &session);
        let batch = plan.build();
        let msgs = msg_list(&batch);
        let types: Vec<u16> = msgs.iter().map(|m| m.type_).collect();
        assert_eq!(
            types,
            vec![
                NFNL_MSG_BATCH_BEGIN,
                nft_type(NFT_MSG_NEWTABLE),
                nft_type(NFT_MSG_NEWCHAIN),
                nft_type(NFT_MSG_NEWCHAIN),
                nft_type(NFT_MSG_NEWCHAIN),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                nft_type(NFT_MSG_NEWRULE),
                NFNL_MSG_BATCH_END,
            ],
            "NEWTABLE + 3 NEWCHAIN + 2 jump NEWRULE + 5 guard NEWRULE + END"
        );
        // Batch control frames: NFNETLINK_V0 (0), nftables res_id big-endian.
        let begin = &msgs[0];
        assert_eq!(&begin.payload[..2], &[0, 0]);
        assert_eq!(&begin.payload[2..], &NFNL_SUBSYS_NFTABLES.to_be_bytes());
        let end = &msgs[12];
        assert_eq!(end.type_, NFNL_MSG_BATCH_END);
        assert_eq!(&end.payload[2..], &NFNL_SUBSYS_NFTABLES.to_be_bytes());
        // Sequence numbers are consecutive from 1; per-message flags follow
        // the kernel convention: control bare REQUEST, creates CREATE|EXCL,
        // rules CREATE|APPEND, everything nft carries an ack request.
        for (i, m) in msgs.iter().enumerate() {
            assert_eq!(m.seq, (i + 1) as u32);
        }
        assert_eq!(msgs[0].flags, NLM_F_REQUEST);
        assert_eq!(
            msgs[1].flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
            "create-only, never replace"
        );
        assert_eq!(
            msgs[5].flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_APPEND
        );
        assert_eq!(msgs[12].flags, NLM_F_REQUEST);
        // Every nftables message carries the family in nfgenmsg byte 0.
        for m in &msgs[1..12] {
            assert_eq!(m.payload[0], NFPROTO_IPV4);
        }
        // The table carries the session marker as raw bytes.
        let attrs = msg_attrs(msgs[1].payload);
        assert_eq!(find(&attrs, NFTA_TABLE_NAME), b"v2ray_rs_1000\0");
        assert_eq!(find(&attrs, NFTA_TABLE_USERDATA), &[0x5a; 16]);
    }

    #[test]
    fn exact_hex_newtable_frame() {
        let session = session16();
        let plan = InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_1000", &session);
        let batch = plan.build();
        // BATCH_BEGIN: len=20, type=0x10, flags=0x0001, seq=1, pid=0,
        // nfgenmsg { family 0, version 0, res_id htons(10) }.
        let expect_begin: [u8; 20] = [
            0x14, 0x00, 0x00, 0x00, 0x10, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x0A,
        ];
        assert_eq!(&batch[0..20], &expect_begin);
        // NEWTABLE: header + nfgen(NFPROTO_IPV4, 0, res_id) + NLA_STRING name
        // "v2ray_rs_1000\0" (14 bytes, padded to 20) + NLA_BINARY marker
        // (16 bytes): message len 60.
        let name = b"v2ray_rs_1000\0";
        let mut expect_new = Vec::new();
        expect_new.extend_from_slice(&60u32.to_ne_bytes());
        expect_new.extend_from_slice(&nft_type(NFT_MSG_NEWTABLE).to_ne_bytes());
        expect_new.extend_from_slice(
            &(NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL).to_ne_bytes(),
        );
        expect_new.extend_from_slice(&2u32.to_ne_bytes());
        expect_new.extend_from_slice(&0u32.to_ne_bytes());
        expect_new.extend_from_slice(&[NFPROTO_IPV4, 0]);
        expect_new.extend_from_slice(&NFNL_SUBSYS_NFTABLES.to_be_bytes());
        expect_new.extend_from_slice(&((4 + name.len()) as u16).to_ne_bytes());
        expect_new.extend_from_slice(&NFTA_TABLE_NAME.to_ne_bytes());
        expect_new.extend_from_slice(name);
        expect_new.extend_from_slice(&[0, 0]); // attr padding
        expect_new.extend_from_slice(&(20u16).to_ne_bytes());
        expect_new.extend_from_slice(&NFTA_TABLE_USERDATA.to_ne_bytes());
        expect_new.extend_from_slice(&[0x5a; 16]);
        assert_eq!(&batch[20..20 + expect_new.len()], &expect_new[..]);
    }

    #[test]
    fn all_attributes_four_byte_aligned() {
        let session = session16();
        let plan = InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_1000", &session);
        let batch = plan.build();
        let mut off = 0usize;
        while off + 16 <= batch.len() {
            let len =
                u32::from_ne_bytes([batch[off], batch[off + 1], batch[off + 2], batch[off + 3]])
                    as usize;
            assert!(
                len >= 16 && len.is_multiple_of(4),
                "nlmsg len {len} aligned"
            );
            assert!(off + len <= batch.len(), "message fits batch");
            // nfgenmsg occupies the first four payload bytes of every
            // message in these batches, control frames included.
            let mut a = off + 20;
            while a + 4 <= off + len {
                let alen = u16::from_ne_bytes([batch[a], batch[a + 1]]) as usize;
                assert!(alen >= 4, "attr length covers header");
                let type_ = u16::from_ne_bytes([batch[a + 2], batch[a + 3]]);
                assert!(type_ != 0, "no UNSPEC attribute types");
                let padded = (alen + 3) & !3;
                assert!(a + padded <= off + len, "attr fits message");
                for b in &batch[a + alen..a + padded] {
                    assert_eq!(*b, 0, "attribute padding is zero");
                }
                a += padded;
            }
            assert_eq!(a, off + len, "attributes exactly fill message");
            off += len;
        }
        assert_eq!(off, batch.len(), "messages exactly fill batch");
    }

    #[test]
    fn base_chain_hook_priority_policy_type() {
        let session = session16();
        let plan = InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_1000", &session);
        let batch = plan.build();
        let msgs = msg_list(&batch);
        // seq 3: guard (regular), seq 4: output (LOCAL_OUT), 5: prerouting.
        let guard = &msgs[2];
        let ga = msg_attrs(guard.payload);
        assert_eq!(find(&ga, NFTA_CHAIN_TABLE), b"v2ray_rs_1000\0");
        assert_eq!(find(&ga, NFTA_CHAIN_NAME), b"guard\0");
        assert!(
            ga.iter().all(|(t, _)| *t != NFTA_CHAIN_HOOK),
            "regular chain has no hook"
        );
        let output = &msgs[3];
        let oa = msg_attrs(output.payload);
        assert_eq!(find(&oa, NFTA_CHAIN_NAME), b"output\0");
        let hook = find(&oa, NFTA_CHAIN_HOOK);
        let ha = attr_map(hook);
        assert_eq!(
            find(&ha, NFTA_HOOK_HOOKNUM),
            &NF_INET_LOCAL_OUT.to_be_bytes()
        );
        assert_eq!(find(&ha, NFTA_HOOK_PRIORITY), &(-150i32).to_be_bytes());
        assert_eq!(find(&oa, NFTA_CHAIN_POLICY), &NF_ACCEPT.to_be_bytes());
        assert_eq!(find(&oa, NFTA_CHAIN_TYPE), b"filter\0");
        let pre = &msgs[4];
        let pa = msg_attrs(pre.payload);
        let hook = find(&pa, NFTA_CHAIN_HOOK);
        let ha = attr_map(hook);
        assert_eq!(
            find(&ha, NFTA_HOOK_HOOKNUM),
            &NF_INET_PRE_ROUTING.to_be_bytes()
        );
        assert_eq!(find(&ha, NFTA_HOOK_PRIORITY), &(-150i32).to_be_bytes());
    }

    #[test]
    fn regular_chain_rules_match_frozen_semantics() {
        let session = session16();
        let plan = InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_1000", &session);
        let batch = plan.build();
        let msgs = msg_list(&batch);
        for m in &msgs[7..12] {
            let a = msg_attrs(m.payload);
            assert_eq!(find(&a, NFTA_RULE_TABLE), b"v2ray_rs_1000\0");
            assert_eq!(find(&a, NFTA_RULE_CHAIN), b"guard\0");
        }
        // Rule 1: meta mark & fwmark == fwmark accept, no queue.
        let rule = &msgs[7];
        assert_eq!(expr_count(rule.payload), 4);
        let exprs: Vec<_> = (0..4).map(|i| expr_of(rule.payload, i)).collect();
        assert_eq!(exprs[0].0, b"meta\0");
        assert_eq!(u32be(find(&exprs[0].1, NFTA_META_DREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[0].1, NFTA_META_KEY)), NFT_META_MARK);
        assert_eq!(exprs[1].0, b"bitwise\0");
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BITWISE_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BITWISE_DREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BITWISE_LEN)), 4);
        assert_eq!(
            find(
                &attr_map(find(&exprs[1].1, NFTA_BITWISE_MASK)),
                NFTA_DATA_VALUE
            ),
            &0x4000_0000u32.to_ne_bytes()
        );
        assert_eq!(
            find(
                &attr_map(find(&exprs[1].1, NFTA_BITWISE_XOR)),
                NFTA_DATA_VALUE
            ),
            &0u32.to_ne_bytes()
        );
        assert_eq!(exprs[2].0, b"cmp\0");
        assert_eq!(u32be(find(&exprs[2].1, NFTA_CMP_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[2].1, NFTA_CMP_OP)), NFT_CMP_EQ);
        assert_eq!(
            find(&attr_map(find(&exprs[2].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &0x4000_0000u32.to_ne_bytes()
        );
        assert_eq!(exprs[3].0, b"immediate\0");
        assert_eq!(
            u32be(find(&exprs[3].1, NFTA_IMMEDIATE_DREG)),
            NFT_REG_VERDICT
        );
        let vd = attr_map(find(
            &attr_map(find(&exprs[3].1, NFTA_IMMEDIATE_DATA)),
            NFTA_DATA_VERDICT,
        ));
        assert_eq!(u32be(find(&vd, NFTA_VERDICT_CODE)), NF_ACCEPT);
        assert!(
            vd.iter().all(|(t, _)| *t != NFTA_VERDICT_CHAIN),
            "accept verdict has no chain"
        );

        // Rule 2: ct original packets 1..=12 converted to network order,
        // tcp dport 443, queue bypass.
        let rule = &msgs[8];
        assert_eq!(expr_count(rule.payload), 9);
        let exprs: Vec<_> = (0..9).map(|i| expr_of(rule.payload, i)).collect();
        assert_eq!(exprs[0].0, b"ct\0");
        assert_eq!(u32be(find(&exprs[0].1, NFTA_CT_DREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[0].1, NFTA_CT_KEY)), NFT_CT_PKTS);
        assert_eq!(find(&exprs[0].1, NFTA_CT_DIRECTION), &[IP_CT_DIR_ORIGINAL]);
        assert_eq!(exprs[1].0, b"byteorder\0");
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BYTEORDER_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BYTEORDER_DREG)), NFT_REG32_00);
        assert_eq!(
            u32be(find(&exprs[1].1, NFTA_BYTEORDER_OP)),
            NFT_BYTEORDER_HTON
        );
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BYTEORDER_LEN)), 8);
        assert_eq!(u32be(find(&exprs[1].1, NFTA_BYTEORDER_SIZE)), 8);
        assert_eq!(exprs[2].0, b"cmp\0");
        assert_eq!(u32be(find(&exprs[2].1, NFTA_CMP_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[2].1, NFTA_CMP_OP)), NFT_CMP_GTE);
        assert_eq!(
            find(&attr_map(find(&exprs[2].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &[0, 0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(exprs[3].0, b"cmp\0");
        assert_eq!(u32be(find(&exprs[3].1, NFTA_CMP_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[3].1, NFTA_CMP_OP)), NFT_CMP_LTE);
        assert_eq!(
            find(&attr_map(find(&exprs[3].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &[0, 0, 0, 0, 0, 0, 0, 12]
        );
        assert_eq!(exprs[4].0, b"meta\0");
        assert_eq!(u32be(find(&exprs[4].1, NFTA_META_DREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&exprs[4].1, NFTA_META_KEY)), NFT_META_L4PROTO);
        assert_eq!(exprs[5].0, b"cmp\0");
        assert_eq!(
            find(&attr_map(find(&exprs[5].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &[6]
        );
        assert_eq!(exprs[6].0, b"payload\0");
        assert_eq!(u32be(find(&exprs[6].1, NFTA_PAYLOAD_DREG)), NFT_REG32_00);
        assert_eq!(
            u32be(find(&exprs[6].1, NFTA_PAYLOAD_BASE)),
            NFT_PAYLOAD_TRANSPORT_HEADER
        );
        assert_eq!(u32be(find(&exprs[6].1, NFTA_PAYLOAD_OFFSET)), 2);
        assert_eq!(u32be(find(&exprs[6].1, NFTA_PAYLOAD_LEN)), 2);
        assert_eq!(exprs[7].0, b"cmp\0");
        assert_eq!(
            find(&attr_map(find(&exprs[7].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &443u16.to_be_bytes()
        );
        assert_eq!(exprs[8].0, b"queue\0");
        assert_eq!(u16be(find(&exprs[8].1, NFTA_QUEUE_NUM)), 200);
        assert_eq!(
            u16be(find(&exprs[8].1, NFTA_QUEUE_FLAGS)),
            NFT_QUEUE_FLAG_BYPASS
        );

        // Rule 3: sport variant gates on the reply direction, offset 0.
        let rule = &msgs[9];
        assert_eq!(expr_count(rule.payload), 9);
        let exprs: Vec<_> = (0..9).map(|i| expr_of(rule.payload, i)).collect();
        assert_eq!(find(&exprs[0].1, NFTA_CT_DIRECTION), &[IP_CT_DIR_REPLY]);
        assert_eq!(u32be(find(&exprs[6].1, NFTA_PAYLOAD_OFFSET)), 0);
        assert_eq!(
            find(&attr_map(find(&exprs[7].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &443u16.to_be_bytes()
        );

        // Rules 4/5: udp, no conntrack gate.
        let rule = &msgs[10];
        assert_eq!(expr_count(rule.payload), 5);
        let exprs: Vec<_> = (0..5).map(|i| expr_of(rule.payload, i)).collect();
        assert_eq!(
            find(&attr_map(find(&exprs[1].1, NFTA_CMP_DATA)), NFTA_DATA_VALUE),
            &[17]
        );
        assert_eq!(u32be(find(&exprs[2].1, NFTA_PAYLOAD_OFFSET)), 2);
        assert_eq!(u16be(find(&exprs[4].1, NFTA_QUEUE_NUM)), 200);
        let rule = &msgs[11];
        assert_eq!(expr_count(rule.payload), 5);
        let exprs: Vec<_> = (0..5).map(|i| expr_of(rule.payload, i)).collect();
        assert_eq!(u32be(find(&exprs[2].1, NFTA_PAYLOAD_OFFSET)), 0);

        // Base chains jump to the guard chain.
        for (i, chain) in [
            (5usize, b"output\0".as_slice()),
            (6usize, b"prerouting\0".as_slice()),
        ] {
            let a = msg_attrs(msgs[i].payload);
            assert_eq!(find(&a, NFTA_RULE_CHAIN), chain);
            assert_eq!(expr_count(msgs[i].payload), 1);
            let (name, data) = expr_of(msgs[i].payload, 0);
            assert_eq!(name, b"immediate\0");
            let imm = attr_map(find(&data, NFTA_IMMEDIATE_DATA));
            let vd = attr_map(find(&imm, NFTA_DATA_VERDICT));
            assert_eq!(u32be(find(&vd, NFTA_VERDICT_CODE)), NFT_JUMP as u32);
            assert_eq!(find(&vd, NFTA_VERDICT_CHAIN), b"guard\0");
        }
    }

    #[test]
    fn v6_batch_carries_only_the_marked_table() {
        let batch =
            InstallPlan::new(Family::V6, 200, 0x4000_0000, "v2ray_rs_1000", &session16()).build();
        let msgs = msg_list(&batch);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].type_, NFNL_MSG_BATCH_BEGIN);
        assert_eq!(msgs[1].type_, nft_type(NFT_MSG_NEWTABLE));
        assert_eq!(msgs[1].payload[0], NFPROTO_IPV6);
        let attrs = msg_attrs(msgs[1].payload);
        assert_eq!(find(&attrs, NFTA_TABLE_NAME), b"v2ray_rs_1000\0");
        assert_eq!(find(&attrs, NFTA_TABLE_USERDATA), &[0x5a; 16]);
        assert_eq!(msgs[2].type_, NFNL_MSG_BATCH_END);
        assert_eq!(msgs[2].seq, 3);
        assert!(
            msgs.iter().all(|m| m.type_ != nft_type(NFT_MSG_NEWCHAIN)),
            "no chains in the ip6 batch"
        );
        assert!(
            msgs.iter().all(|m| m.type_ != nft_type(NFT_MSG_NEWRULE)),
            "no rules in the ip6 batch"
        );
    }

    #[test]
    fn v6_install_waits_for_the_table_ack_only() {
        let tx = ScriptedTx::new(vec![ok_acks(2, 2)]);
        install_with(
            Family::V6,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .expect("the v6 install needs only the table ack");
        assert_eq!(
            msg_list(&tx.seen.borrow()[0]).len(),
            3,
            "BEGIN, NEWTABLE, END"
        );

        // An ack for a sequence the three-message batch never carried is
        // protocol garbage and fails the install.
        let tx = ScriptedTx::new(vec![ok_acks(2, 12)]);
        let err = install_with(
            Family::V6,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("unrequested sequence 3"), "{err}");
    }

    #[test]
    fn install_sends_one_unconditional_exclusive_create_batch() {
        // No leading GETTABLE: the create-only batch goes out as-is.
        let tx = ScriptedTx::new(vec![ok_acks(2, 12)]);
        let session = session16();
        install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session,
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .expect("install succeeds");
        let seen = tx.seen.borrow();
        assert_eq!(seen.len(), 1, "one exclusive-create batch, no lookup");
        let batch = msg_list(&seen[0]);
        assert_eq!(batch.len(), 13);
        assert_eq!(batch[1].type_, nft_type(NFT_MSG_NEWTABLE));
        let attrs = msg_attrs(batch[1].payload);
        assert_eq!(find(&attrs, NFTA_TABLE_USERDATA), &[0x5a; 16]);
        assert_eq!(find(&attrs, NFTA_TABLE_NAME), b"t\0");
    }

    #[test]
    fn name_present_ignores_the_marker() {
        // Confirmed absence under the owned name.
        let tx = ScriptedTx::new(vec![err_reply(
            -libc::ENOENT,
            nft_type(NFT_MSG_GETTABLE),
            1,
        )]);
        assert!(
            !name_present_with(
                Family::V4,
                "t",
                &Budget::probe(),
                Duration::ZERO,
                &mut |b| tx.call(b)
            )
            .unwrap()
        );
        // Any table under the name counts, whatever marker it carries:
        // fresh-create intent must refuse it, not adopt it.
        let tx = ScriptedTx::new(vec![gettable_reply(
            Family::V4,
            b"t\0",
            Some(&[0x01; 16]),
            Some(7),
        )]);
        assert!(
            name_present_with(
                Family::V4,
                "t",
                &Budget::probe(),
                Duration::ZERO,
                &mut |b| tx.call(b)
            )
            .unwrap()
        );
    }

    #[test]
    fn install_refuses_eexist_without_replacing() {
        // A concurrent creator wins the race: EEXIST is a typed conflict
        // naming the existing object, and there is no retry.
        let tx = ScriptedTx::new(vec![err_reply(
            -libc::EEXIST,
            nft_type(NFT_MSG_NEWTABLE),
            2,
        )]);
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "typed conflict: {err}");
        let msg = format!("{err}");
        assert!(msg.contains("nft install"), "op named: {msg}");
        assert!(msg.contains("already exists"), "{msg}");
        assert!(msg.contains("'t'"), "existing object named: {msg}");
        assert_eq!(tx.seen.borrow().len(), 1, "no retry after EEXIST");
    }

    #[test]
    fn install_rejects_missing_and_unrequested_acks() {
        // The last rule ack never arrives: incomplete success is refused,
        // naming the unacknowledged message.
        let tx = ScriptedTx::new(vec![ok_acks(2, 11)]);
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("incomplete"), "{msg}");
        assert!(msg.contains("NEWRULE"), "offending message named: {msg}");

        // An ack for a sequence we never sent is protocol garbage.
        let mut replies = ok_acks(2, 12);
        replies.extend(err_reply(0, 0, 99));
        let tx = ScriptedTx::new(vec![replies]);
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &Budget::probe(),
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("unrequested sequence 99"),
            "{err}"
        );
    }

    #[test]
    fn replies_accumulate_acks_across_datagrams() {
        let requests = vec![
            (2, nft_type(NFT_MSG_NEWTABLE)),
            (3, nft_type(NFT_MSG_NEWCHAIN)),
        ];
        // Two datagrams, one ack each: only the second completes.
        let mut acc = Replies::new(requests.clone(), &[2, 3], false);
        assert!(matches!(acc.feed(&ok_acks(2, 2)), Feed::NeedMore));
        assert_eq!(acc.pending.len(), 1);
        assert!(matches!(acc.feed(&ok_acks(3, 3)), Feed::Complete));

        // Fragmented acks within one stream, out of order: correlation is
        // by echoed sequence, not by position.
        let mut acc = Replies::new(requests, &[2, 3], false);
        let mut mixed = ok_acks(3, 3);
        mixed.extend(ok_acks(2, 2));
        assert!(matches!(acc.feed(&mixed), Feed::Complete));
    }

    #[test]
    fn replies_reject_kernel_errors_truncation_and_unusable_records() {
        // Kernel refusal names the offending message.
        let mut acc = Replies::new(vec![(2, nft_type(NFT_MSG_NEWTABLE))], &[2], false);
        assert!(matches!(
            acc.feed(&err_reply(-libc::EPERM, nft_type(NFT_MSG_NEWTABLE), 2)),
            Feed::Failed(_)
        ));

        // A truncated message record is refused instead of silently dropped.
        let mut acc = Replies::new(vec![(2, nft_type(NFT_MSG_NEWTABLE))], &[2], false);
        let mut truncated = ok_acks(2, 2);
        truncated.truncate(truncated.len() - 2);
        assert!(matches!(acc.feed(&truncated), Feed::Failed(_)));

        // An error record without the full nlmsgerr payload is unusable.
        let mut acc = Replies::new(vec![(2, nft_type(NFT_MSG_NEWTABLE))], &[2], false);
        assert!(matches!(
            acc.feed(&[
                0x10, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0
            ]),
            Feed::Failed(_)
        ));

        // After a failure the exchange stays failed for further datagrams.
        let mut acc = Replies::new(vec![(2, nft_type(NFT_MSG_NEWTABLE))], &[2], false);
        let _ = acc.feed(&err_reply(-libc::EPERM, nft_type(NFT_MSG_NEWTABLE), 2));
        assert!(matches!(acc.feed(&ok_acks(2, 2)), Feed::Failed(_)));
    }

    #[test]
    fn remove_probes_then_deletes_by_handle_then_confirms() {
        let session = session16();
        let present = gettable_reply(
            Family::V4,
            b"t\0",
            Some(session.as_bytes()),
            Some(0xA1B2_C3D4_5E6F_7081),
        );
        let tx = ScriptedTx::new(vec![present, ok_acks(2, 2), {
            // The confirming GETTABLE sees the table is gone.
            err_reply(-libc::ENOENT, nft_type(NFT_MSG_GETTABLE), 1)
        }]);
        remove_with(Family::V4, "t", &session, &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .expect("verified removal");
        let seen = tx.seen.borrow();
        assert_eq!(seen.len(), 3, "GETTABLE, DELTABLE batch, confirm GETTABLE");
        let del = msg_list(&seen[1]);
        assert_eq!(del.len(), 3, "BEGIN, DELTABLE, END");
        assert_eq!(del[1].type_, nft_type(NFT_MSG_DELTABLE));
        let attrs = msg_attrs(del[1].payload);
        assert_eq!(
            u64be(find(&attrs, NFTA_TABLE_HANDLE)),
            0xA1B2_C3D4_5E6F_7081,
            "delete goes by the verified big-endian handle"
        );
        assert!(
            attrs.iter().all(|(t, _)| *t != NFTA_TABLE_NAME),
            "handle-only delete: no name attribute"
        );
        let confirm = msg_list(&seen[2]);
        assert_eq!(confirm[0].type_, nft_type(NFT_MSG_GETTABLE));
    }

    #[test]
    fn remove_treats_confirmed_absence_as_clean() {
        let tx = ScriptedTx::new(vec![err_reply(
            -libc::ENOENT,
            nft_type(NFT_MSG_GETTABLE),
            1,
        )]);
        remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .expect("absent table is clean");
        assert_eq!(tx.seen.borrow().len(), 1, "no DELTABLE without the table");
    }

    #[test]
    fn remove_refuses_marker_mismatch_and_markerless_tables() {
        // Different marker: refusal, no delete.
        let other = Uuid::from_bytes([0x01; 16]);
        let tx = ScriptedTx::new(vec![gettable_reply(
            Family::V4,
            b"t\0",
            Some(other.as_bytes()),
            Some(1),
        )]);
        let err = remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("nft remove"), "op named: {msg}");
        assert!(msg.contains("different session marker"), "{msg}");
        assert_eq!(tx.seen.borrow().len(), 1);

        // Markerless table: never deleted.
        let tx = ScriptedTx::new(vec![gettable_reply(Family::V4, b"t\0", None, Some(1))]);
        let err = remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .unwrap_err();
        assert!(format!("{err}").contains("no session marker"), "{err}");
        assert_eq!(tx.seen.borrow().len(), 1);
    }

    #[test]
    fn remove_maps_kernel_errors_and_raced_deletions() {
        let present = gettable_reply(Family::V4, b"t\0", Some(&[0x5a; 16]), Some(7));

        // Kernel refusal on the delete surfaces with the operation named.
        let tx = ScriptedTx::new(vec![
            present.clone(),
            err_reply(-libc::EPERM, nft_type(NFT_MSG_DELTABLE), 2),
        ]);
        let err = remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("nft remove"), "op named: {msg}");
        assert!(msg.contains("DELTABLE"), "offending message named: {msg}");

        // Raced deletion: DELTABLE answers ENOENT and the confirming
        // GETTABLE agrees the table is gone.
        let tx = ScriptedTx::new(vec![
            present,
            err_reply(-libc::ENOENT, nft_type(NFT_MSG_DELTABLE), 2),
            err_reply(-libc::ENOENT, nft_type(NFT_MSG_GETTABLE), 1),
        ]);
        remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .expect("raced deletion is a confirmed absence");

        // The deletion acked but the object survived: refusal, not success.
        let tx = ScriptedTx::new(vec![
            gettable_reply(Family::V4, b"t\0", Some(&[0x5a; 16]), Some(7)),
            ok_acks(2, 2),
            gettable_reply(Family::V4, b"t\0", Some(&[0x5a; 16]), Some(7)),
        ]);
        let err = remove_with(Family::V4, "t", &session16(), &Budget::probe(), &mut |b| {
            tx.call(b)
        })
        .unwrap_err();
        assert!(
            format!("{err}").contains("still present after DELTABLE"),
            "{err}"
        );
    }

    #[test]
    fn present_decodes_gettable_replies() {
        // Absent table: ENOENT on the GETTABLE is Ok(false).
        let tx = ScriptedTx::new(vec![err_reply(
            -libc::ENOENT,
            nft_type(NFT_MSG_GETTABLE),
            1,
        )]);
        let session = session16();
        assert!(
            !present_with(
                Family::V4,
                "t",
                &session,
                &Budget::probe(),
                Duration::ZERO,
                &mut |b| tx.call(b)
            )
            .expect("absent table")
        );
        let sent = tx.seen.borrow();
        let msgs = msg_list(&sent[0]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].type_, nft_type(NFT_MSG_GETTABLE));
        assert_eq!(msgs[0].payload[0], NFPROTO_IPV4);
        assert_eq!(find(&msg_attrs(msgs[0].payload), NFTA_TABLE_NAME), b"t\0");

        // Exact match on family, name, marker, and handle: Ok(true).
        let tx = ScriptedTx::new(vec![gettable_reply(
            Family::V4,
            b"t\0",
            Some(session.as_bytes()),
            Some(42),
        )]);
        assert!(
            present_with(
                Family::V4,
                "t",
                &session,
                &Budget::probe(),
                Duration::ZERO,
                &mut |b| tx.call(b)
            )
            .expect("present table")
        );

        // Empty reply is an error, not a silent false.
        let tx = ScriptedTx::new(vec![Vec::new()]);
        assert!(
            present_with(
                Family::V4,
                "t",
                &session,
                &Budget::probe(),
                Duration::ZERO,
                &mut |b| tx.call(b)
            )
            .is_err()
        );

        // Other errnos surface with the operation named.
        let tx = ScriptedTx::new(vec![err_reply(-libc::EPERM, nft_type(NFT_MSG_GETTABLE), 1)]);
        let err = present_with(
            Family::V4,
            "t",
            &session,
            &Budget::probe(),
            Duration::ZERO,
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("nft lookup"), "op named: {err}");

        // Wrong family byte in the reply is a refusal.
        let tx = ScriptedTx::new(vec![gettable_reply(
            Family::V6,
            b"t\0",
            Some(session.as_bytes()),
            Some(42),
        )]);
        let err = present_with(
            Family::V4,
            "t",
            &session,
            &Budget::probe(),
            Duration::ZERO,
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("family"), "{err}");

        // Wrong name in the reply is a refusal.
        let tx = ScriptedTx::new(vec![gettable_reply(
            Family::V4,
            b"other\0",
            Some(session.as_bytes()),
            Some(42),
        )]);
        let err = present_with(
            Family::V4,
            "t",
            &session,
            &Budget::probe(),
            Duration::ZERO,
            &mut |b| tx.call(b),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("returned table"), "{err}");
    }

    #[test]
    fn table_decode_requires_handle_and_marker_bytes() {
        // No handle attribute: unusable reply.
        let reply = Msg::new(nft_type(NFT_MSG_NEWTABLE), 0, 1)
            .with_nfgen(Family::V4.nfproto())
            .attr_bytes(NFTA_TABLE_NAME, b"t\0")
            .attr_bytes(NFTA_TABLE_USERDATA, &[0x5a; 16]);
        assert!(decode_table(&reply.into_raw()[16..]).is_err());

        // A handle that is not 8 bytes: unusable reply.
        let reply = Msg::new(nft_type(NFT_MSG_NEWTABLE), 0, 1)
            .with_nfgen(Family::V4.nfproto())
            .attr_bytes(NFTA_TABLE_NAME, b"t\0")
            .attr_bytes(NFTA_TABLE_HANDLE, &[1, 2, 3, 4]);
        assert!(decode_table(&reply.into_raw()[16..]).is_err());

        // Handle bytes decode big-endian, marker bytes stay opaque.
        let reply = table_reply(
            Family::V4,
            b"t\0",
            Some(&[0x5a; 16]),
            Some(0x0102_0304_0506_0708),
        );
        let info = decode_table(&reply[16..]).expect("decodes");
        assert_eq!(info.handle, 0x0102_0304_0506_0708);
        assert_eq!(info.userdata.as_deref(), Some(&[0x5a; 16][..]));
        assert_eq!(info.name, "t");
        assert_eq!(info.family, NFPROTO_IPV4);
    }

    /// Register data blobs follow the kernel's byte order: `meta mark` is
    /// stored host-order and compares native-endian, `ct packets` passes
    /// through a `byteorder` conversion and compares against big-endian
    /// bounds, and payload ports stay network-order.
    #[test]
    fn register_constants_follow_kernel_byte_order() {
        let fwmark = 0x4000_0000u32;

        // meta mark & fwmark == fwmark: mask, xor, and cmp data in host
        // order (x86_64 reference decode: mask=1073741824).
        let exprs = guard_rule_exprs(fwmark);
        let bitwise = attr_map(find(&attr_map(&exprs[1]), NFTA_EXPR_DATA));
        let mask = attr_map(find(&bitwise, NFTA_BITWISE_MASK));
        assert_eq!(
            find(&mask, NFTA_DATA_VALUE),
            &[0x00, 0x00, 0x00, 0x40],
            "mask 0x40000000 as raw native bytes"
        );
        let xor = attr_map(find(&bitwise, NFTA_BITWISE_XOR));
        assert_eq!(find(&xor, NFTA_DATA_VALUE), &0u32.to_ne_bytes());
        let mark_cmp = attr_map(find(&attr_map(&exprs[2]), NFTA_EXPR_DATA));
        let mark_cmp = attr_map(find(&mark_cmp, NFTA_CMP_DATA));
        assert_eq!(
            find(&mark_cmp, NFTA_DATA_VALUE),
            &[0x00, 0x00, 0x00, 0x40],
            "mark cmp data as raw native bytes"
        );

        // ct packets loads once, converts to network order in place, and
        // compares against big-endian bounds.
        let exprs = tcp_rule_exprs(IP_CT_DIR_ORIGINAL, 2, 200);
        assert_eq!(find(&attr_map(&exprs[0]), NFTA_EXPR_NAME), b"ct\0");
        let byteorder = attr_map(find(&attr_map(&exprs[1]), NFTA_EXPR_DATA));
        assert_eq!(find(&byteorder, NFTA_BYTEORDER_SREG).len(), 4);
        assert_eq!(u32be(find(&byteorder, NFTA_BYTEORDER_SREG)), NFT_REG32_00);
        assert_eq!(u32be(find(&byteorder, NFTA_BYTEORDER_DREG)), NFT_REG32_00);
        assert_eq!(
            u32be(find(&byteorder, NFTA_BYTEORDER_OP)),
            NFT_BYTEORDER_HTON
        );
        assert_eq!(u32be(find(&byteorder, NFTA_BYTEORDER_LEN)), 8);
        assert_eq!(u32be(find(&byteorder, NFTA_BYTEORDER_SIZE)), 8);
        let bound_low = attr_map(find(&attr_map(&exprs[2]), NFTA_EXPR_DATA));
        let bound_low = attr_map(find(&bound_low, NFTA_CMP_DATA));
        assert_eq!(
            find(&bound_low, NFTA_DATA_VALUE),
            &[0, 0, 0, 0, 0, 0, 0, 1],
            "ct bound 1 big-endian"
        );
        let bound_high = attr_map(find(&attr_map(&exprs[3]), NFTA_EXPR_DATA));
        let bound_high = attr_map(find(&bound_high, NFTA_CMP_DATA));
        assert_eq!(
            find(&bound_high, NFTA_DATA_VALUE),
            &[0, 0, 0, 0, 0, 0, 0, 12],
            "ct bound 12 big-endian"
        );

        // Payload-derived port comparisons stay network order: 443 = 01 bb.
        let dport = attr_map(find(&attr_map(&exprs[7]), NFTA_EXPR_DATA));
        let dport = attr_map(find(&dport, NFTA_CMP_DATA));
        assert_eq!(find(&dport, NFTA_DATA_VALUE), &443u16.to_be_bytes());
        let udp = udp_rule_exprs(2, 200);
        let udp_dport = attr_map(find(&attr_map(&udp[3]), NFTA_EXPR_DATA));
        let udp_dport = attr_map(find(&udp_dport, NFTA_CMP_DATA));
        assert_eq!(find(&udp_dport, NFTA_DATA_VALUE), &[0x01, 0xbb]);
    }

    #[test]
    fn unprivileged_kernel_error_is_returned_as_data() {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipping: root holds CAP_NET_ADMIN");
            return;
        }
        let batch =
            InstallPlan::new(Family::V4, 200, 0x4000_0000, "v2ray_rs_probe", &session16()).build();
        let mut socket = Netlink::open().expect("netlink socket opens");
        let stream = socket
            .transact(&batch, Duration::from_secs(5))
            .expect("a kernel NLMSG_ERROR is data, not a transport failure");
        let mut acc = collector_for(&batch);
        assert!(matches!(acc.feed(&stream), Feed::Failed(_)));
        let (failed_type, errno, _) = acc.error.expect("kernel error recorded");
        assert_eq!(failed_type, NFNL_MSG_BATCH_BEGIN);
        assert_eq!(errno, -libc::EPERM);

        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "v2ray_rs_probe",
            &session16(),
            &Budget::probe(),
            &mut |b| socket.transact(b, Duration::from_secs(5)),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Firewall(_)), "typed firewall: {err}");
        assert!(format!("{err}").contains("BATCH_BEGIN"), "{err}");
    }

    #[test]
    fn recv_timeout_never_disables_the_socket_timeout() {
        let tv = recv_timeout(Duration::from_nanos(1));
        assert_eq!((tv.tv_sec, tv.tv_usec), (0, 1));
        let tv = recv_timeout(Duration::from_micros(1500));
        assert_eq!((tv.tv_sec, tv.tv_usec), (0, 1500));
        let tv = recv_timeout(Duration::from_millis(1500));
        assert_eq!((tv.tv_sec, tv.tv_usec), (1, 500_000));
    }

    /// A reply that arrives after the total budget was consumed must not
    /// give the next exchange a fresh deadline: the pre-send check ends the
    /// operation with the typed timeout and no second send.
    #[test]
    fn late_reply_cannot_reset_the_budget() {
        let budget = Budget::with_total(Duration::from_millis(120));
        let session = session16();
        let calls = std::cell::Cell::new(0u32);
        let mut tx = |_b: &[u8]| {
            let n = calls.get() + 1;
            calls.set(n);
            if n == 1 {
                std::thread::sleep(Duration::from_millis(200));
                return Ok(gettable_reply(
                    Family::V4,
                    b"t\0",
                    Some(&[0x5a; 16]),
                    Some(7),
                ));
            }
            Ok(ok_acks(2, 2))
        };
        let err = remove_with(Family::V4, "t", &session, &budget, &mut tx).unwrap_err();
        assert!(matches!(err, Error::TimedOut(_)), "typed timeout: {err}");
        assert_eq!(
            calls.get(),
            1,
            "the late reply did not reset the budget: the next exchange refuses before sending"
        );
    }

    /// No budget, no exchange: the transport is never invoked, so no
    /// backend work can start on an already-expired operation.
    #[test]
    fn exhausted_budget_fails_before_any_send() {
        let budget = Budget::with_total(Duration::ZERO);
        let sent = std::cell::Cell::new(false);
        let mut tx = |_b: &[u8]| {
            sent.set(true);
            Ok(Vec::new())
        };
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &budget,
            &mut tx,
        )
        .unwrap_err();
        assert!(matches!(err, Error::TimedOut(_)), "typed timeout: {err}");
        assert!(!sent.get(), "no exchange started on an exhausted budget");
    }

    /// A transport reporting an expired wait surfaces as the typed
    /// timeout, never as a generic I/O error.
    #[test]
    fn transport_timeout_maps_to_typed_timedout() {
        let budget = Budget::probe();
        let mut tx = |_b: &[u8]| Err(io::Error::new(io::ErrorKind::TimedOut, "delayed transport"));
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &budget,
            &mut tx,
        )
        .unwrap_err();
        assert!(matches!(err, Error::TimedOut(_)), "typed timeout: {err}");
    }

    /// Install exchanges must leave the rollback window intact: with less
    /// than the reserve left they refuse to send at all.
    #[test]
    fn install_exchange_never_touches_the_rollback_reserve() {
        // Exactly the reserve plus a sliver: the install slice is positive.
        let budget = Budget::with_total(Duration::from_millis(3100));
        let mut tx = |_b: &[u8]| Ok(ok_acks(2, 12));
        install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &budget,
            &mut tx,
        )
        .expect("install budget respects the rollback reserve");

        // Under the reserve there is no install budget at all.
        let budget = Budget::with_total(Duration::from_millis(2000));
        let sent = std::cell::Cell::new(false);
        let mut tx = |_b: &[u8]| {
            sent.set(true);
            Ok(ok_acks(2, 12))
        };
        let err = install_with(
            Family::V4,
            200,
            0x4000_0000,
            "t",
            &session16(),
            &budget,
            &mut tx,
        )
        .unwrap_err();
        assert!(matches!(err, Error::TimedOut(_)), "typed timeout: {err}");
        assert!(!sent.get(), "reserve window is never spent on installs");
    }

    /// One budget shared across exchanges shrinks monotonically; a reply
    /// in between never restores it.
    #[test]
    fn shared_budget_shrinks_monotonically() {
        let budget = Budget::with_total(Duration::from_millis(500));
        let before = budget.remaining().expect("fresh budget has time");
        let mut tx = |_b: &[u8]| {
            std::thread::sleep(Duration::from_millis(80));
            Ok(Vec::new())
        };
        assert!(
            present_with(
                Family::V4,
                "t",
                &session16(),
                &budget,
                Duration::ZERO,
                &mut tx
            )
            .is_err()
        );
        let after = budget.remaining().expect("budget not yet exhausted");
        assert!(after < before, "remaining shrank: {before:?} -> {after:?}");
    }
}
