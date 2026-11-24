//! Firewall wiring: an owned nftables table (`v2ray_rs`, one per family)
//! funnels the first packets of TLS/QUIC flows on port 443 into our NFQUEUE,
//! plus the sysctls the techniques depend on. All netlink work lives in
//! `nft`; this module owns the journal lifecycle around it: installation is
//! all-or-nothing with intent recorded before the first mutation, removal
//! touches only the recorded families, and every kernel object is keyed by
//! the journal's session UUID marker while every mutation is bound to the
//! requesting controller (pid + start ticks) and the boot/network-namespace
//! epoch, mirroring the tun-session ownership model. Journal transactions
//! are serialized by one per-effective-UID, per-network-namespace
//! transaction lock (`state::lock_transaction`), independent of the journal
//! path. The helper never writes sysctls; it only reports unmet kernel
//! prerequisites.

use crate::error::{Error, Result};
use crate::nft::{self, Budget, Family};
use crate::state::{self, Journal, Liveness, Phase, ProcessIdentity};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use v2ray_rs_tun_session::Epoch;

pub const DEFAULT_QUEUE: u16 = 200;
/// Mirrors `engine::DEFAULT_FWMARK`: re-injected packets carry the mark so
/// they bypass the queue.
pub const DEFAULT_FWMARK: u32 = 0x4000_0000;

/// The sysctls the techniques rely on.
const SYSCTLS: [(&str, &str); 2] = [
    ("net.netfilter.nf_conntrack_acct", "1"),
    ("net.netfilter.nf_conntrack_tcp_be_liberal", "1"),
];

/// What an install against the existing journal must do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InstallAction {
    /// No journal: start the requested session from scratch.
    Fresh,
    /// The caller's own active session reinstalling with the same session
    /// id, queue, and mark, and both tables verified through the marker.
    Reuse(Journal),
    /// An incomplete session (crashed install, unfinished cleanup, or an
    /// active record whose tables are gone) owned by this caller, or a
    /// session whose controller is provably dead: clean it first.
    Clean(Journal),
}

/// Ownership decision for installs. The epoch must always match. A foreign
/// owner is refused while alive or unprovable, and its state is cleaned
/// only once its death is provable in this boot/network namespace. The
/// caller's own active session is reused only when the requested session
/// id, queue, and mark all match and both recorded tables still verify
/// through the session marker; any kernel-side mismatch is refused with the
/// journal preserved, never adopted. Everything else owned by the caller is
/// an incomplete session that gets cleaned before the fresh install.
fn decide_install(
    existing: Option<&Journal>,
    epoch: &Epoch,
    owner: &ProcessIdentity,
    queue: u16,
    fwmark: u32,
    session: Uuid,
    mut present: impl FnMut(Family, Uuid) -> Result<bool>,
) -> Result<InstallAction> {
    let Some(j) = existing else {
        return Ok(InstallAction::Fresh);
    };
    if j.epoch != *epoch {
        return Err(Error::Firewall(
            "the anti-DPI journal belongs to another boot or network namespace; use --sweep".into(),
        ));
    }
    // A numeric PID is meaningful only in the PID namespace that assigned
    // it: no ownership or liveness decision without matching evidence.
    state::check_pidns(j).map_err(|e| {
        Error::Firewall(format!(
            "the anti-DPI journal's PID namespace does not match this process ({e}); refusing to take it over"
        ))
    })?;
    if j.owner != *owner {
        return match state::owner_liveness(&j.owner) {
            Liveness::Dead => Ok(InstallAction::Clean(j.clone())),
            // A dedicated error, not the generic firewall conflict: it is
            // raised only here, under the transaction lock and before this
            // request acquired anything, so `install-rules` can report a
            // verified refusal on its own exit status.
            Liveness::Alive => Err(Error::ForeignLiveOwner(format!(
                "the anti-DPI rules are owned by a live process (pid {}); refusing to modify them",
                j.owner.pid
            ))),
            Liveness::Unknown => Err(Error::Firewall(format!(
                "cannot determine whether the anti-DPI owner (pid {}) is still alive; refusing to modify its state",
                j.owner.pid
            ))),
        };
    }
    if j.phase == Phase::Active {
        if j.session_id != session {
            return Err(Error::Firewall(format!(
                "the anti-DPI journal records a different active session ({}); remove the rules first",
                j.session_id
            )));
        }
        if j.queue != queue || j.fwmark != fwmark {
            return Err(Error::Firewall(format!(
                "the active anti-DPI session holds queue {} with mark {:#010x}; refusing to change them without removal",
                j.queue, j.fwmark
            )));
        }
        let mut absent = false;
        for (name, family) in [("ipv4", Family::V4), ("ipv6", Family::V6)] {
            match present(family, j.session_id) {
                Ok(true) => {}
                Ok(false) => absent = true,
                Err(e) => {
                    return Err(Error::Firewall(format!(
                        "cannot verify the recorded {name} table for session {}: {e}",
                        j.session_id
                    )));
                }
            }
        }
        if absent {
            // The active session's tables are gone: it is incomplete and
            // this caller may clean it.
            return Ok(InstallAction::Clean(j.clone()));
        }
        return Ok(InstallAction::Reuse(j.clone()));
    }
    // Applying or Cleaning: an incomplete session owned by this caller.
    Ok(InstallAction::Clean(j.clone()))
}

/// Ok(false): nothing recorded, which is a successful no-op removal.
/// Refusals for a foreign epoch, a foreign owner, or a mismatched session
/// id happen before any mutation.
fn decide_removal(
    existing: Option<&Journal>,
    epoch: &Epoch,
    owner: &ProcessIdentity,
    session: Option<Uuid>,
) -> Result<bool> {
    let Some(j) = existing else {
        return Ok(false);
    };
    if j.epoch != *epoch {
        return Err(Error::Firewall(
            "the anti-DPI journal belongs to another boot or network namespace; use --sweep".into(),
        ));
    }
    // A numeric PID is meaningful only in the PID namespace that assigned
    // it: no ownership decision without matching evidence.
    state::check_pidns(j).map_err(|e| {
        Error::Firewall(format!(
            "the anti-DPI journal's PID namespace does not match this process ({e}); refusing automatic removal"
        ))
    })?;
    if j.owner != *owner {
        return Err(Error::Firewall(format!(
            "the anti-DPI rules are owned by pid {}; refusing removal",
            j.owner.pid
        )));
    }
    if let Some(s) = session
        && s != j.session_id
    {
        return Err(Error::Firewall(format!(
            "session {s} does not match the recorded anti-DPI session {}; refusing removal",
            j.session_id
        )));
    }
    Ok(true)
}

/// What a sweep against the existing journal must do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SweepDecision {
    /// No journal and no unowned tables under the owned names.
    Nothing,
    /// The journal belongs to a previous boot and both owned table names
    /// are confirmed absent in this namespace: retire only the stale
    /// record, touching no kernel object.
    RetireStale,
    /// A provably dead owner in this boot and network namespace: drain the
    /// recorded families.
    Drain(Journal),
}

/// Sweep gate: a journal from a previous boot is retired only when both
/// owned table names are absent in this namespace (no PID-namespace check
/// applies to a prior boot), refused when a same-named table exists. For a
/// same-boot journal, a legacy record without PID-namespace evidence stays
/// preserved for operator inspection and a record from another network
/// namespace refuses, naming that namespace. A live or unprovable owner is
/// refused, and tables without a journal are reported, never deleted.
fn decide_sweep(
    existing: Option<&Journal>,
    epoch: &Epoch,
    present_family: &impl Fn(Family) -> Result<bool>,
) -> Result<SweepDecision> {
    let Some(j) = existing else {
        let mut found: Vec<&str> = Vec::new();
        for (name, family) in [("ip", Family::V4), ("ip6", Family::V6)] {
            if present_family(family)
                .map_err(|e| Error::Firewall(format!("cannot probe nftables: {e}")))?
            {
                found.push(name);
            }
        }
        if !found.is_empty() {
            return Err(Error::Firewall(format!(
                "v2ray_rs table(s) ({}) exist without a journal; refusing to delete unowned state",
                found.join(", ")
            )));
        }
        return Ok(SweepDecision::Nothing);
    };
    if j.epoch.boot_id != epoch.boot_id {
        // Previous boot: the recorded PIDs cannot exist here, so the
        // PID-namespace check is meaningless and skipped. The stale record
        // is retired only when both owned table names are confirmed absent
        // in this namespace; any same-named table refuses the sweep.
        let mut present = Vec::new();
        for (name, family) in [("ip", Family::V4), ("ip6", Family::V6)] {
            if present_family(family)
                .map_err(|e| Error::Firewall(format!("cannot probe nftables: {e}")))?
            {
                present.push(name);
            }
        }
        if !present.is_empty() {
            return Err(Error::Firewall(format!(
                "the anti-DPI journal belongs to a previous boot, but table(s) ({}) exist under the owned names in this namespace; sweep refused",
                present.join(", ")
            )));
        }
        return Ok(SweepDecision::RetireStale);
    }
    // Same boot only: a legacy record without PID-namespace evidence stays
    // preserved for operator inspection, and a record from another network
    // namespace refuses, naming that namespace.
    state::check_pidns(j).map_err(|e| {
        Error::Firewall(format!(
            "the anti-DPI journal cannot be tied to this PID namespace ({e}); sweep refused and the record preserved"
        ))
    })?;
    if j.epoch != *epoch {
        return Err(Error::Firewall(format!(
            "the anti-DPI journal belongs to another network namespace (inode {}) in this boot; run the sweep in that namespace",
            j.epoch.namespace_inode
        )));
    }
    match state::owner_liveness(&j.owner) {
        Liveness::Dead => {
            // Death alone is not enough: every recorded family must still
            // be provably present or absent before anything is drained.
            for (slot, _name, family) in FAMILY_SLOTS {
                if family_flag(j, slot) {
                    present_family(family)
                        .map_err(|e| Error::Firewall(format!("cannot probe nftables: {e}")))?;
                }
            }
            Ok(SweepDecision::Drain(j.clone()))
        }
        Liveness::Alive => Err(Error::Firewall(format!(
            "the anti-DPI owner (pid {}) is still alive; sweep refused",
            j.owner.pid
        ))),
        Liveness::Unknown => Err(Error::Firewall(format!(
            "cannot determine whether the anti-DPI owner (pid {}) is alive; sweep refused",
            j.owner.pid
        ))),
    }
}

/// The two family obligations in drain order with their journal slots.
const FAMILY_SLOTS: [(usize, &str, Family); 2] = [(0, "ipv4", Family::V4), (1, "ipv6", Family::V6)];

fn family_flag(journal: &Journal, slot: usize) -> bool {
    if slot == 0 {
        journal.ipv4
    } else {
        journal.ipv6
    }
}

fn set_family_flag(journal: &mut Journal, slot: usize, value: bool) {
    if slot == 0 {
        journal.ipv4 = value;
    } else {
        journal.ipv6 = value;
    }
}

/// Removes every recorded family through the session marker, clearing each
/// journal obligation only after its removal confirms absence and deleting
/// the journal only once nothing is recorded anymore. Any removal failure
/// or failed journal write keeps the remaining obligations on disk; the
/// phase must already be persisted as `cleaning` before this runs.
fn drain_journal(
    path: &Path,
    journal: &mut Journal,
    mut remove_family: impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<()> {
    let session = journal.session_id;
    let mut failures: Vec<String> = Vec::new();
    for (slot, name, family) in FAMILY_SLOTS {
        if !family_flag(journal, slot) {
            continue;
        }
        match remove_family(family, session) {
            Ok(()) => {
                set_family_flag(journal, slot, false);
                if let Err(w) = write_journal_at(path, journal) {
                    failures.push(format!("{name}: journal update failed: {w}"));
                }
            }
            Err(re) => failures.push(format!("{name}: {re}")),
        }
    }
    if failures.is_empty() {
        journal_removal(path)
    } else {
        Err(Error::Firewall(format!(
            "rollback failed ({})",
            failures.join("; ")
        )))
    }
}

/// Cleans an incomplete session: `cleaning` is persisted before the first
/// removal and every recorded family is removed through its session marker
/// before the journal is deleted. Any failure keeps the journal on disk so
/// a later install, removal, or sweep can resume.
fn clean_session(
    path: &Path,
    journal: &Journal,
    remove_family: &mut impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<()> {
    let mut journal = journal.clone();
    journal.phase = Phase::Cleaning;
    write_journal_at(path, &journal)?;
    drain_journal(path, &mut journal, remove_family).map_err(|e| {
        Error::Firewall(format!(
            "cannot clean the incomplete anti-DPI session {}: {e}",
            journal.session_id
        ))
    })
}

fn read_journal_at(path: &Path) -> Result<Option<Journal>> {
    match state::read_journal(path) {
        Ok(j) => Ok(Some(j)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Firewall(format!(
            "cannot read the anti-DPI journal: {e}"
        ))),
    }
}

fn write_journal_at(path: &Path, journal: &Journal) -> Result<()> {
    state::write_journal(path, journal)
        .map_err(|e| Error::Firewall(format!("cannot write the anti-DPI journal: {e}")))
}

fn journal_removal(path: &Path) -> Result<()> {
    state::remove_journal(path)
        .map_err(|e| Error::Firewall(format!("cannot clear the anti-DPI journal: {e}")))
}

/// Fresh intent requires both owned table names absent in this namespace:
/// an existing same-named table is never adopted, whatever marker it carries.
fn require_names_absent(name_present: &mut impl FnMut(Family) -> Result<bool>) -> Result<()> {
    for (_slot, name, family) in FAMILY_SLOTS {
        if name_present(family)
            .map_err(|e| Error::Firewall(format!("cannot probe the {name} table: {e}")))?
        {
            return Err(Error::Firewall(format!(
                "the {name} table already exists; refusing to install a fresh session over it"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn install_at(
    path: &Path,
    queue: u16,
    fwmark: u32,
    owner: ProcessIdentity,
    session: Uuid,
    mut present: impl FnMut(Family, Uuid) -> Result<bool>,
    mut name_present: impl FnMut(Family) -> Result<bool>,
    mut install_family: impl FnMut(Family, u16, u32, Uuid) -> Result<()>,
    mut remove_family: impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<()> {
    // One exclusive transaction lock covers the whole read-decide-mutate
    // transaction; it is per-euid and per-network-namespace, shared by every
    // journal path.
    let _lock = state::lock_transaction()
        .map_err(|e| Error::Firewall(format!("cannot take the anti-DPI transaction lock: {e}")))?;
    let epoch = Epoch::current()
        .map_err(|e| Error::Firewall(format!("cannot read the boot/namespace epoch: {e}")))?;
    let existing = read_journal_at(path)?;
    match decide_install(
        existing.as_ref(),
        &epoch,
        &owner,
        queue,
        fwmark,
        session,
        &mut present,
    )? {
        InstallAction::Fresh => {
            require_names_absent(&mut name_present)?;
            let journal = Journal::new(queue, fwmark, owner, session)
                .map_err(|e| Error::Firewall(format!("cannot start the anti-DPI journal: {e}")))?;
            install_families(
                path,
                journal,
                queue,
                fwmark,
                session,
                &mut install_family,
                &mut remove_family,
            )
        }
        InstallAction::Reuse(journal) => {
            // Verified reuse: zero kernel mutations and nothing reinstall.
            // A complete active record stays byte-for-byte unchanged; only
            // a lost family flag is repaired with the one complete write.
            if journal.ipv4 && journal.ipv6 {
                return Ok(());
            }
            let mut journal = journal;
            journal.ipv4 = true;
            journal.ipv6 = true;
            write_journal_at(path, &journal)
        }
        InstallAction::Clean(old) => {
            clean_session(path, &old, &mut remove_family)?;
            require_names_absent(&mut name_present)?;
            let journal = Journal::new(queue, fwmark, owner, session)
                .map_err(|e| Error::Firewall(format!("cannot start the anti-DPI journal: {e}")))?;
            install_families(
                path,
                journal,
                queue,
                fwmark,
                session,
                &mut install_family,
                &mut remove_family,
            )
        }
    }
}

/// Runs the two family installs with optimistic intent: the journal says
/// `applying` before the first mutation, each family flag is persisted
/// BEFORE that family's first possible mutation, and `active` lands only
/// after both families succeeded. Any failure persists `cleaning` before
/// the rollback, clears an obligation only after a confirmed removal, and
/// keeps the journal on disk when rollback or journal writes fail.
fn install_families(
    path: &Path,
    mut journal: Journal,
    queue: u16,
    fwmark: u32,
    session: Uuid,
    install_family: &mut impl FnMut(Family, u16, u32, Uuid) -> Result<()>,
    remove_family: &mut impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<()> {
    write_journal_at(path, &journal)?;
    for (slot, _name, family) in FAMILY_SLOTS {
        set_family_flag(&mut journal, slot, true);
        write_journal_at(path, &journal)?;
        if let Err(e) = install_family(family, queue, fwmark, session) {
            if matches!(e, Error::Conflict(_)) {
                // A confirmed EEXIST: this family's table was never
                // acquired, so exactly its optimistic obligation is
                // dropped and persisted before the rollback drains the
                // other family. Ambiguous replies keep every obligation.
                set_family_flag(&mut journal, slot, false);
                write_journal_at(path, &journal)?;
            }
            return Err(rollback(path, &mut journal, remove_family, e));
        }
    }
    journal.phase = Phase::Active;
    write_journal_at(path, &journal)?;
    Ok(())
}

/// Rolls a failed install back: `cleaning` is persisted before the first
/// removal, every recorded family is removed through the session marker,
/// and the journal survives ambiguous acknowledgements and failed journal
/// writes. The returned error names the initiating error and every cleanup
/// error.
fn rollback(
    path: &Path,
    journal: &mut Journal,
    remove_family: &mut impl FnMut(Family, Uuid) -> Result<()>,
    initiating: Error,
) -> Error {
    journal.phase = Phase::Cleaning;
    if let Err(w) = write_journal_at(path, journal) {
        return Error::Firewall(format!(
            "{initiating}; the journal could not be persisted as cleaning: {w}"
        ));
    }
    match drain_journal(path, journal, remove_family) {
        Ok(()) => initiating,
        Err(drain) => Error::Firewall(format!("{initiating}; {drain}")),
    }
}

fn remove_at(
    path: &Path,
    owner: ProcessIdentity,
    session: Option<Uuid>,
    mut remove_family: impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<bool> {
    // One exclusive transaction lock covers the whole read-decide-mutate
    // transaction; it is per-euid and per-network-namespace, shared by every
    // journal path.
    let _lock = state::lock_transaction()
        .map_err(|e| Error::Firewall(format!("cannot take the anti-DPI transaction lock: {e}")))?;
    let epoch = Epoch::current()
        .map_err(|e| Error::Firewall(format!("cannot read the boot/namespace epoch: {e}")))?;
    let existing = read_journal_at(path)?;
    if !decide_removal(existing.as_ref(), &epoch, &owner, session)? {
        return Ok(false);
    }
    let mut journal = existing.expect("decide_removal returned proceed");
    // Intent before the first removal: a crash during cleanup leaves phase
    // `cleaning`, which the owner or a later sweep can resume. Without an
    // explicit session id the owned journal's session id is adopted.
    journal.phase = Phase::Cleaning;
    write_journal_at(path, &journal)?;
    drain_journal(path, &mut journal, &mut remove_family).map_err(|e| {
        Error::Firewall(format!(
            "recorded family removal failed; the journal is kept for retry: {e}"
        ))
    })?;
    Ok(true)
}

fn sweep_at(
    path: &Path,
    present_family: impl Fn(Family) -> Result<bool>,
    mut remove_family: impl FnMut(Family, Uuid) -> Result<()>,
) -> Result<bool> {
    // One exclusive transaction lock covers the whole read-decide-mutate
    // transaction; it is per-euid and per-network-namespace, shared by every
    // journal path.
    let _lock = state::lock_transaction()
        .map_err(|e| Error::Firewall(format!("cannot take the anti-DPI transaction lock: {e}")))?;
    let epoch = Epoch::current()
        .map_err(|e| Error::Firewall(format!("cannot read the boot/namespace epoch: {e}")))?;
    let existing = read_journal_at(path)?;
    match decide_sweep(existing.as_ref(), &epoch, &present_family)? {
        SweepDecision::Nothing => Ok(false),
        SweepDecision::RetireStale => {
            journal_removal(path)?;
            Ok(true)
        }
        SweepDecision::Drain(mut journal) => {
            journal.phase = Phase::Cleaning;
            write_journal_at(path, &journal)?;
            drain_journal(path, &mut journal, &mut remove_family).map_err(|e| {
                Error::Firewall(format!(
                    "recorded family removal failed; the journal is kept for retry: {e}"
                ))
            })?;
            Ok(true)
        }
    }
}

/// Readiness gate for `run`: the journal must record an `active` install
/// for this queue and epoch, every recorded family must still verify
/// through its session marker, and a managed run (`session` given) must
/// also match the session id and the expected controller. With
/// `require_journal = false` an absent journal is accepted (rules managed
/// externally); an existing journal is still fully checked. Read-only: no
/// journal lock is taken.
fn verify_active_at(
    path: &Path,
    queue: u16,
    session: Option<Uuid>,
    controller: Option<&ProcessIdentity>,
    require_journal: bool,
    present_family: impl Fn(Family, Uuid) -> Result<bool>,
) -> Result<()> {
    let Some(j) = read_journal_at(path)? else {
        if require_journal {
            return Err(Error::Firewall(
                "no anti-DPI firewall journal; install the rules first".into(),
            ));
        }
        return Ok(());
    };
    if j.phase != Phase::Active {
        return Err(Error::Firewall(format!(
            "the anti-DPI journal phase is {:?}, not active",
            j.phase
        )));
    }
    // An active record must carry both family obligations: a missing flag
    // is an incomplete install no run may start against.
    if !j.ipv4 {
        return Err(Error::Firewall(
            "the active anti-DPI journal does not record the ipv4 table".into(),
        ));
    }
    if !j.ipv6 {
        return Err(Error::Firewall(
            "the active anti-DPI journal does not record the ipv6 table".into(),
        ));
    }
    state::check_pidns(&j).map_err(|e| {
        Error::Firewall(format!(
            "the anti-DPI journal cannot be verified in this PID namespace: {e}"
        ))
    })?;
    let epoch = Epoch::current()
        .map_err(|e| Error::Firewall(format!("cannot read the boot/namespace epoch: {e}")))?;
    if j.epoch != epoch {
        return Err(Error::Firewall(
            "the anti-DPI journal belongs to another boot or network namespace".into(),
        ));
    }
    if j.queue != queue {
        return Err(Error::Firewall(format!(
            "the anti-DPI journal holds queue {}, not {queue}",
            j.queue
        )));
    }
    if let Some(s) = session
        && s != j.session_id
    {
        return Err(Error::Firewall(format!(
            "the anti-DPI journal records session {}, not {s}",
            j.session_id
        )));
    }
    if let Some(expected) = controller
        && j.owner != *expected
    {
        return Err(Error::Firewall(format!(
            "the anti-DPI journal is owned by pid {}, not this controller",
            j.owner.pid
        )));
    }
    for (name, family, recorded) in [("ipv4", Family::V4, j.ipv4), ("ipv6", Family::V6, j.ipv6)] {
        if recorded {
            let present = present_family(family, j.session_id)
                .map_err(|e| Error::Firewall(format!("cannot probe nftables: {e}")))?;
            if !present {
                return Err(Error::Firewall(format!(
                    "the recorded {name} table for session {} is missing from the kernel",
                    j.session_id
                )));
            }
        }
    }
    Ok(())
}

/// Locates the anti-DPI journal, naming the environment failure for the
/// CLI user.
fn journal_path() -> Result<PathBuf> {
    state::state_path()
        .map_err(|e| Error::Firewall(format!("cannot locate the anti-DPI journal: {e}")))
}

/// Installs both families for this controller and session, all-or-nothing:
/// the journal records `applying` and each family flag before its first
/// possible mutation, and `active` only when both are in. Any failure
/// persists `cleaning`, removes what the session recorded, and clears the
/// journal only after confirmed absence.
pub fn install(queue: u16, fwmark: u32, owner: ProcessIdentity, session: Uuid) -> Result<()> {
    // One monotonic deadline for the whole operation: fresh probes, both
    // family installs, and the rollback drain share it, with the rollback
    // reserve held back from the install exchanges.
    let budget = Budget::install();
    let path = journal_path()?;
    install_at(
        &path,
        queue,
        fwmark,
        owner,
        session,
        |family, s| nft::present(family, s, &budget, nft::ROLLBACK_RESERVE),
        |family| nft::name_present(family, &budget, nft::ROLLBACK_RESERVE),
        |family, q, m, s| nft::install(family, q, m, s, &budget),
        |family, s| nft::remove(family, s, &budget),
    )
}

/// Removes exactly the recorded families for the owning controller. A
/// supplied session id must match the journal's; `None` adopts the owned
/// journal's session id. Ok(false): nothing recorded ("no state recorded"
/// is a success). Any failure keeps the journal in phase `cleaning` for a
/// retry.
pub fn remove(owner: ProcessIdentity, session: Option<Uuid>) -> Result<bool> {
    let budget = Budget::removal();
    let path = journal_path()?;
    remove_at(&path, owner, session, |family, s| {
        nft::remove(family, s, &budget)
    })
}

/// Removes recorded families when the recorded owner is provably dead in
/// this boot/network namespace; refuses a live or unprovable owner and a
/// foreign epoch, and reports unowned tables without deleting them.
/// Ok(false): nothing to do.
pub fn sweep() -> Result<bool> {
    // Fail closed: only a confirmed absence counts as absent, and a
    // transport or kernel error refuses the sweep instead of deleting
    // anything.
    let budget = Budget::removal();
    let present = |family| nft::name_present(family, &budget, std::time::Duration::ZERO);
    let path = journal_path()?;
    sweep_at(&path, present, |family, s| nft::remove(family, s, &budget))
}

/// Holds the anti-DPI transaction lock between readiness verification and
/// publication: while a run owns it, no other transaction can install,
/// remove, or sweep the owned tables. Dropping the guard closes the held
/// lock file and releases the lock.
pub struct RunGuard(std::fs::File);

impl Drop for RunGuard {
    fn drop(&mut self) {
        let _ = &self.0;
    }
}

/// Readiness gate for `run` serialized through the transaction lock: the
/// full active verification (phase, both family flags, PID namespace,
/// epoch, queue, session, controller, kernel presence) runs while the
/// per-euid, per-network-namespace lock is held, and the returned guard
/// carries the lock until readiness publication. The caller releases it
/// immediately after writing the READY sentinel (see
/// `Engine::publish_ready`), before the engine loop. Teardown
/// serialization does not depend on this guard: removal and sweep take
/// their own transaction lock.
pub fn verify_active_for_run(
    queue: u16,
    session: Option<Uuid>,
    controller: Option<&ProcessIdentity>,
    require_journal: bool,
) -> Result<RunGuard> {
    let lock = state::lock_transaction()
        .map_err(|e| Error::Firewall(format!("cannot take the anti-DPI transaction lock: {e}")))?;
    let path = journal_path()?;
    verify_active_at(
        &path,
        queue,
        session,
        controller,
        require_journal,
        |family, s| nft::present(family, s, &Budget::probe(), std::time::Duration::ZERO),
    )?;
    Ok(RunGuard(lock))
}

/// The owned table name the implementation installs, one per family. The
/// privileged tests assert against the same name.
pub fn table_name() -> String {
    crate::nft::table_name()
}

/// Kernel prerequisites that are currently unmet. Observation only: nothing
/// is written, and the helper never writes sysctls.
pub fn unmet_sysctls() -> Vec<(&'static str, &'static str)> {
    SYSCTLS
        .iter()
        .copied()
        .filter(|(key, want)| read_sysctl(key).as_deref() != Some(*want))
        .collect()
}

/// `/proc/sys` tail for a dotted sysctl key: only `[A-Za-z0-9_-]`
/// components, no empty components.
fn sysctl_tail(key: &str) -> Option<String> {
    let tail = key.replace('.', "/");
    if tail.is_empty()
        || !tail.split('/').all(|c| {
            !c.is_empty()
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return None;
    }
    Some(tail)
}

pub fn read_sysctl(key: &str) -> Option<String> {
    let tail = sysctl_tail(key)?;
    std::fs::read_to_string(format!("/proc/sys/{tail}"))
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Above the kernel's pid limit (2^22) and inside the signed `pid_t`
    /// range: this owner can never exist, and the kill probe addresses a
    /// process, never a process group.
    const DEAD: ProcessIdentity = ProcessIdentity {
        pid: 2_000_000_000,
        start_time_ticks: 1,
    };

    fn owner_now() -> ProcessIdentity {
        ProcessIdentity::current().unwrap()
    }

    fn owner(pid: u32, ticks: u64) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            start_time_ticks: ticks,
        }
    }

    fn journal(
        epoch: &Epoch,
        owner: ProcessIdentity,
        phase: Phase,
        ipv4: bool,
        ipv6: bool,
    ) -> Journal {
        Journal {
            version: state::JOURNAL_VERSION,
            session_id: Uuid::new_v4(),
            epoch: epoch.clone(),
            pidns: Some(state::current_pidns().unwrap()),
            owner,
            phase,
            queue: 200,
            fwmark: DEFAULT_FWMARK,
            ipv4,
            ipv6,
        }
    }

    fn seed(path: &Path, j: &Journal) {
        state::write_journal(path, j).unwrap();
    }

    /// Serializes the per-euid transaction lock across the parallel test
    /// threads: every `_at` function here takes `state::lock_transaction()`.
    fn tx_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static TX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        TX_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn name_absent(_family: Family) -> Result<bool> {
        Ok(false)
    }

    fn forbidden_install(_f: Family, _q: u16, _m: u32, _s: Uuid) -> Result<()> {
        panic!("no install may run here");
    }

    fn on_disk(path: &Path) -> Option<Journal> {
        state::read_journal(path).ok()
    }

    fn present_ok(_family: Family, _session: Uuid) -> Result<bool> {
        Ok(true)
    }

    /// Table exists but not under this session's marker.
    fn mismatch_v4(family: Family, _session: Uuid) -> Result<bool> {
        match family {
            Family::V4 => Err(Error::Firewall("marker mismatch".into())),
            Family::V6 => Ok(true),
        }
    }

    fn install_both(_family: Family, _queue: u16, _fwmark: u32, _session: Uuid) -> Result<()> {
        Ok(())
    }

    fn fail_v4(family: Family, _queue: u16, _fwmark: u32, _session: Uuid) -> Result<()> {
        match family {
            Family::V4 => Err(Error::Firewall("v4 install failed".into())),
            Family::V6 => Ok(()),
        }
    }

    fn fail_v6(family: Family, _queue: u16, _fwmark: u32, _session: Uuid) -> Result<()> {
        match family {
            Family::V4 => Ok(()),
            Family::V6 => Err(Error::Firewall("v6 install failed".into())),
        }
    }

    fn never_remove(_family: Family, _session: Uuid) -> Result<()> {
        panic!("unexpected removal");
    }

    fn recording_remove<'a>(
        sink: &'a mut Vec<&'static str>,
    ) -> impl FnMut(Family, Uuid) -> Result<()> + 'a {
        move |family, _session| {
            sink.push(if matches!(family, Family::V4) {
                "v4"
            } else {
                "v6"
            });
            Ok(())
        }
    }

    #[test]
    fn fresh_install_records_families_and_activates() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            session,
            present_ok,
            name_absent,
            install_both,
            never_remove,
        )
        .unwrap();
        let j = on_disk(&path).unwrap();
        assert_eq!(j.phase, Phase::Active);
        assert!(j.ipv4 && j.ipv6);
        assert_eq!(j.owner, DEAD);
        assert_eq!(j.queue, 200);
        assert_eq!(j.fwmark, DEFAULT_FWMARK);
        assert_eq!(j.session_id, session);
    }

    #[test]
    fn v6_failure_rolls_back_both_recorded_families_and_clears_the_journal() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            fail_v6,
            remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("v6"), "{err}");
        // The optimistic ipv6 flag records the obligation even though the
        // v6 install never completed; the removal treats absence as clean.
        assert_eq!(removed, vec!["v4", "v6"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn failed_rollback_keeps_the_journal_and_names_both_errors() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let remove = |family: Family, _session: Uuid| -> Result<()> {
            match family {
                Family::V4 => Err(Error::Firewall("v4 rollback failed".into())),
                Family::V6 => Ok(()),
            }
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            fail_v6,
            remove,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("v6 install failed"), "{msg}");
        assert!(msg.contains("v4 rollback failed"), "{msg}");
        let j = on_disk(&path).unwrap();
        assert_eq!(j.phase, Phase::Cleaning);
        // v4 could not be removed, so its obligation is kept; v6 was
        // removed and its flag cleared.
        assert!(j.ipv4 && !j.ipv6);
    }

    #[test]
    fn v4_failure_installs_nothing_and_clears_the_optimistic_flag() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            fail_v4,
            remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("v4"), "{err}");
        assert_eq!(removed, vec!["v4"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn optimistic_family_flag_survives_an_injected_crash_point() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let remove = |family: Family, _session: Uuid| -> Result<()> {
            match family {
                // The crash point: the rolled-back family cannot be removed.
                Family::V4 => Err(Error::Firewall("v4 removal refused".into())),
                Family::V6 => Ok(()),
            }
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            session,
            present_ok,
            name_absent,
            fail_v4,
            remove,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("v4 install failed"), "{msg}");
        assert!(msg.contains("v4 removal refused"), "{msg}");
        let j = on_disk(&path).unwrap();
        // The ipv4 flag was persisted BEFORE the failing v4 mutation, so
        // the crash window leaves the obligation recoverable on disk.
        assert_eq!(j.phase, Phase::Cleaning);
        assert!(j.ipv4);
        assert!(!j.ipv6);
        assert_eq!(j.session_id, session);
    }

    #[test]
    fn same_owner_reinstall_keeps_the_session_without_mutations() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let mut j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            false,
        );
        j.session_id = session;
        seed(&path, &j);
        install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            owner_now(),
            session,
            present_ok,
            name_absent,
            forbidden_install,
            never_remove,
        )
        .unwrap();
        let j2 = on_disk(&path).unwrap();
        assert_eq!(j2.session_id, session);
        assert_eq!(j2.phase, Phase::Active);
        assert_eq!(j2.queue, 200);
        assert!(j2.ipv4 && j2.ipv6, "the lost flag is repaired by one write");
        // The repaired session still drains both recorded families.
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        assert!(remove_at(&path, owner_now(), None, remove).unwrap());
        assert_eq!(removed, vec!["v4", "v6"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn active_session_refuses_a_changed_queue_or_mark() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        let mut j = j;
        j.session_id = session;
        seed(&path, &j);
        let mut installs = 0usize;
        let mut install_count = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            installs += 1;
            Ok(())
        };
        // Changed queue.
        let err = install_at(
            &path,
            201,
            DEFAULT_FWMARK,
            owner_now(),
            session,
            present_ok,
            name_absent,
            &mut install_count,
            never_remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("queue"), "{err}");
        // Changed mark.
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK + 1,
            owner_now(),
            session,
            present_ok,
            name_absent,
            &mut install_count,
            never_remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mark"), "{err}");
        assert_eq!(installs, 0);
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn active_session_refuses_a_different_session_id() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let mut j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        j.session_id = session;
        seed(&path, &j);
        let mut installs = 0usize;
        let install_count = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            installs += 1;
            Ok(())
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            owner_now(),
            Uuid::new_v4(),
            present_ok,
            name_absent,
            install_count,
            never_remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("session"), "{err}");
        assert_eq!(installs, 0);
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn marker_mismatch_preserves_the_journal() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let mut j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        j.session_id = session;
        seed(&path, &j);
        let mut installs = 0usize;
        let install_count = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            installs += 1;
            Ok(())
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            owner_now(),
            session,
            mismatch_v4,
            name_absent,
            install_count,
            never_remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("marker mismatch"), "{err}");
        assert_eq!(installs, 0);
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn foreign_live_owner_is_refused_without_changes() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        seed(&path, &j);
        let mut removals = 0usize;
        let count = |_f: Family, _s: Uuid| -> Result<()> {
            removals += 1;
            Ok(())
        };
        let mut installs = 0usize;
        let install_count = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            installs += 1;
            Ok(())
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            install_count,
            count,
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::ForeignLiveOwner(_)),
            "a live foreign owner needs the dedicated refusal, got {err:?}"
        );
        assert!(err.to_string().contains("live process"), "{err}");
        assert_eq!((removals, installs), (0, 0));
        // The refusal is raised before acquisition mutates anything: the
        // journal keeps its owner, session, phase, and both recorded
        // families.
        let unchanged = on_disk(&path).expect("the journal must survive the refusal");
        assert_eq!(unchanged, j);
        assert_eq!(unchanged.phase, Phase::Active);
    }

    #[test]
    fn foreign_epoch_is_refused_without_changes() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut foreign_epoch = Epoch::current().unwrap();
        foreign_epoch.namespace_inode += 1;
        let j = journal(&foreign_epoch, DEAD, Phase::Active, true, false);
        seed(&path, &j);
        let mut removals = 0usize;
        let count = |_f: Family, _s: Uuid| -> Result<()> {
            removals += 1;
            Ok(())
        };
        let mut installs = 0usize;
        let install_count = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            installs += 1;
            Ok(())
        };
        assert!(
            install_at(
                &path,
                200,
                DEFAULT_FWMARK,
                DEAD,
                Uuid::new_v4(),
                present_ok,
                name_absent,
                install_count,
                count
            )
            .is_err()
        );
        assert_eq!((removals, installs), (0, 0));
        assert_eq!(on_disk(&path), Some(j.clone()));
        assert!(remove_at(&path, DEAD, None, never_remove).is_err());
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn dead_owner_state_is_recovered_then_reinstalled() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let old = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, false);
        let old_session = old.session_id;
        seed(&path, &old);
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            owner_now(),
            Uuid::new_v4(),
            present_ok,
            name_absent,
            install_both,
            remove,
        )
        .unwrap();
        assert_eq!(removed, vec!["v4"]);
        let j = on_disk(&path).unwrap();
        assert_eq!(j.owner, owner_now());
        assert_eq!(j.phase, Phase::Active);
        assert_ne!(j.session_id, old_session);
    }

    #[test]
    fn removal_only_takes_recorded_families() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, false);
        seed(&path, &j);
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        assert!(remove_at(&path, DEAD, None, remove).unwrap());
        assert_eq!(removed, vec!["v4"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn removal_failure_keeps_the_journal_in_cleaning_phase() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, true);
        seed(&path, &j);
        let mut removed = Vec::new();
        let mut remove = |family: Family, _session: Uuid| -> Result<()> {
            match family {
                Family::V4 => {
                    removed.push("v4");
                    Ok(())
                }
                Family::V6 => Err(Error::Firewall("v6 removal failed".into())),
            }
        };
        let err = remove_at(&path, DEAD, None, &mut remove).unwrap_err();
        assert!(err.to_string().contains("v6"), "{err}");
        assert_eq!(removed, vec!["v4"]);
        let j2 = on_disk(&path).unwrap();
        assert_eq!(j2.phase, Phase::Cleaning);
        // v4 was removed and its obligation cleared; v6 failed and stays
        // recorded for a retry.
        assert!(!j2.ipv4 && j2.ipv6);
    }

    #[test]
    fn removal_without_journal_is_a_successful_no_op() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut calls = 0usize;
        let count = |_f: Family, _s: Uuid| -> Result<()> {
            calls += 1;
            Ok(())
        };
        assert!(!remove_at(&path, DEAD, None, count).unwrap());
        assert_eq!(calls, 0);
    }

    #[test]
    fn removal_refuses_a_foreign_owner() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        seed(&path, &j);
        assert!(remove_at(&path, DEAD, None, never_remove).is_err());
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn removal_validates_a_supplied_session_id() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, false);
        let session = j.session_id;
        seed(&path, &j);
        // A mismatched session id is refused with the journal preserved.
        assert!(remove_at(&path, DEAD, Some(Uuid::new_v4()), never_remove).is_err());
        assert_eq!(on_disk(&path), Some(j.clone()));
        // The matching id removes; an explicit id is therefore honored.
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        assert!(remove_at(&path, DEAD, Some(session), remove).unwrap());
        assert_eq!(removed, vec!["v4"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn lock_contention_fails_without_mutation() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let mut j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, false);
        j.session_id = session;
        seed(&path, &j);
        {
            let guard = state::lock_transaction().unwrap();
            let err = install_at(
                &path,
                200,
                DEFAULT_FWMARK,
                owner_now(),
                session,
                present_ok,
                name_absent,
                install_both,
                never_remove,
            )
            .unwrap_err();
            assert!(err.to_string().contains("lock"), "{err}");
            // The same contention refusal applies to removal and sweep.
            assert!(remove_at(&path, DEAD, None, never_remove).is_err());
            assert!(sweep_at(&path, |_f: Family| Ok(false), never_remove).is_err());
            assert_eq!(on_disk(&path), Some(j.clone()));
            drop(guard);
        }
        // With the lock released the same install proceeds.
        install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            session,
            present_ok,
            name_absent,
            install_both,
            never_remove,
        )
        .unwrap();
        let j2 = on_disk(&path).unwrap();
        assert_eq!(j2.phase, Phase::Active);
        assert_eq!(j2.session_id, session);
        assert!(j2.ipv4 && j2.ipv6);
    }

    #[test]
    fn sweep_decisions_follow_ownership() {
        let cur = Epoch::current().unwrap();
        let live = journal(&cur, owner_now(), Phase::Active, true, true);
        assert!(decide_sweep(Some(&live), &cur, &|_f: Family| Ok(true)).is_err());
        let dead = journal(&cur, DEAD, Phase::Active, true, false);
        assert_eq!(
            decide_sweep(Some(&dead), &cur, &|_f: Family| Ok(false)).unwrap(),
            SweepDecision::Drain(dead)
        );
        // An owner whose death cannot be established is refused.
        let unknown = journal(&cur, owner(0, 1), Phase::Active, true, true);
        assert!(decide_sweep(Some(&unknown), &cur, &|_f: Family| Ok(false)).is_err());
        assert!(decide_sweep(None, &cur, &|_f: Family| Ok(true)).is_err());
        assert_eq!(
            decide_sweep(None, &cur, &|_f: Family| Ok(false)).unwrap(),
            SweepDecision::Nothing
        );
        // Prior boot with both names absent: retire the stale record.
        let mut prior = cur.clone();
        prior.boot_id = Uuid::new_v4();
        let prior_j = journal(&prior, DEAD, Phase::Active, true, true);
        assert_eq!(
            decide_sweep(Some(&prior_j), &cur, &|_f: Family| Ok(false)).unwrap(),
            SweepDecision::RetireStale
        );
        assert!(decide_sweep(Some(&prior_j), &cur, &|_f: Family| Ok(true)).is_err());
    }

    #[test]
    fn sweep_refuses_unowned_tables_and_sweeps_dead_owner_state() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        // Tables present with no journal: refuse, never delete.
        assert!(sweep_at(&path, |_f: Family| Ok(true), never_remove).is_err());
        assert!(on_disk(&path).is_none());
        // Dead owner, phase `applying` (crashed install): sweep completes it.
        let j = journal(
            &Epoch::current().unwrap(),
            DEAD,
            Phase::Applying,
            true,
            true,
        );
        seed(&path, &j);
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        assert!(sweep_at(&path, |_f: Family| Ok(false), remove).unwrap());
        assert_eq!(removed, vec!["v4", "v6"]);
        assert!(on_disk(&path).is_none());
    }

    /// Acceptance for the second-journal-path defect: two distinct journal
    /// paths for the same kernel objects serialize on the one per-euid,
    /// per-network-namespace transaction lock — the losing path writes no
    /// journal and the winning record stays the only owner, with equal and
    /// with different session UUIDs.
    #[test]
    fn second_journal_path_serializes_on_the_shared_transaction_lock() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path_a = dir.path().join("a.json");
        let path_b = dir.path().join("b.json");
        let session = Uuid::new_v4();
        install_at(
            &path_a,
            200,
            DEFAULT_FWMARK,
            DEAD,
            session,
            present_ok,
            name_absent,
            install_both,
            never_remove,
        )
        .unwrap();
        // While the winner's transaction is live, a second journal path
        // fails on the shared lock before any decision or mutation — with
        // the same session id and with a different one.
        let held = state::lock_transaction().unwrap();
        for attempted in [session, Uuid::new_v4()] {
            let err = install_at(
                &path_b,
                200,
                DEFAULT_FWMARK,
                DEAD,
                attempted,
                present_ok,
                name_absent,
                forbidden_install,
                never_remove,
            )
            .unwrap_err();
            assert!(err.to_string().contains("lock"), "{err}");
        }
        assert!(
            on_disk(&path_b).is_none(),
            "the losing path writes no journal"
        );
        drop(held);
        // The winning record is the only owner and is unchanged.
        let j = on_disk(&path_a).unwrap();
        assert_eq!(j.phase, Phase::Active);
        assert_eq!(j.session_id, session);
        assert!(j.ipv4 && j.ipv6);
        // Once unlocked, a different-UUID install on the second path still
        // refuses on the occupied kernel names: no second journal, no
        // mutation.
        let present_v4 = |family: Family| Ok(matches!(family, Family::V4));
        let err = install_at(
            &path_b,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            present_v4,
            forbidden_install,
            never_remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(on_disk(&path_b).is_none());
    }

    /// A failing name probe must fail the sweep closed for a prior-boot and
    /// a same-boot journal alike: nothing is removed and the record survives
    /// for operator inspection.
    #[test]
    fn sweep_probe_failure_refuses_and_preserves_the_record() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let failing =
            |_family: Family| -> Result<bool> { Err(Error::Firewall("name probe failed".into())) };
        let cur = Epoch::current().unwrap();
        // Prior boot: would retire the stale record, but only with both
        // names confirmed absent.
        let mut prior = cur.clone();
        prior.boot_id = Uuid::new_v4();
        let prior_j = journal(&prior, DEAD, Phase::Active, true, true);
        seed(&path, &prior_j);
        let err = sweep_at(&path, failing, never_remove).unwrap_err();
        assert!(err.to_string().contains("probe failed"), "{err}");
        assert_eq!(on_disk(&path), Some(prior_j));
        // Same boot, dead owner: would drain the recorded families, but the
        // uncertain probe refuses first.
        let same_boot = journal(&cur, DEAD, Phase::Active, true, true);
        seed(&path, &same_boot);
        let err = sweep_at(&path, failing, never_remove).unwrap_err();
        assert!(err.to_string().contains("probe failed"), "{err}");
        assert_eq!(on_disk(&path), Some(same_boot));
    }

    #[test]
    fn sweep_refuses_a_foreign_epoch() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut foreign_epoch = Epoch::current().unwrap();
        foreign_epoch.namespace_inode += 1;
        let j = journal(&foreign_epoch, DEAD, Phase::Active, true, true);
        seed(&path, &j);
        let err = sweep_at(&path, |_f: Family| Ok(false), never_remove).unwrap_err();
        assert!(err.to_string().contains("namespace"), "{err}");
        assert_eq!(on_disk(&path), Some(j));
    }

    /// Cleaning a dead owner's state and reinstalling must not adopt a
    /// table that reappeared under an owned name after the old journal was
    /// drained: the fresh install refuses and leaves nothing behind.
    #[test]
    fn clean_then_install_refuses_a_table_that_reappeared() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        seed(
            &path,
            &journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, false),
        );
        let name_there = |family: Family| Ok(matches!(family, Family::V4));
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        // A different dead owner reaches the Clean path; the same owner with a
        // different session short-circuits at decide_install instead.
        let other_dead = owner(2_000_000_001, 1);
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            other_dead,
            Uuid::new_v4(),
            present_ok,
            name_there,
            forbidden_install,
            remove,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("ipv4 table already exists"),
            "{err}"
        );
        assert_eq!(removed, vec!["v4"]);
        assert!(on_disk(&path).is_none(), "the drained journal stays gone");
    }

    #[test]
    fn verify_active_checks_phase_epoch_queue_session_controller_and_presence() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        assert!(verify_active_at(&path, 200, None, None, true, present_ok).is_err());
        assert!(verify_active_at(&path, 200, None, None, false, present_ok).is_ok());

        let session = Uuid::new_v4();
        let mut j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, true);
        j.session_id = session;
        seed(&path, &j);
        assert!(verify_active_at(&path, 200, None, None, true, present_ok).is_ok());
        assert!(verify_active_at(&path, 201, None, None, true, present_ok).is_err());
        // Both family flags must be recorded: a missing flag refuses.
        let mut half = j.clone();
        half.ipv6 = false;
        seed(&path, &half);
        let err = verify_active_at(&path, 200, None, None, true, present_ok).unwrap_err();
        assert!(err.to_string().contains("ipv6"), "{err}");
        seed(&path, &j);
        let only_v4_present = |family: Family, _s: Uuid| Ok(matches!(family, Family::V4));
        assert!(verify_active_at(&path, 200, None, None, true, only_v4_present).is_err());
        let all_missing = |_f: Family, _s: Uuid| Ok(false);
        assert!(verify_active_at(&path, 200, None, None, true, all_missing).is_err());
        // A managed run must reference the recorded session.
        assert!(
            verify_active_at(&path, 200, Some(Uuid::new_v4()), None, true, present_ok).is_err()
        );
        assert!(verify_active_at(&path, 200, Some(session), None, true, present_ok).is_ok());
        // A managed run must be owned by the expected controller.
        assert!(verify_active_at(&path, 200, Some(session), Some(&DEAD), true, present_ok).is_ok());
        assert!(
            verify_active_at(
                &path,
                200,
                Some(session),
                Some(&owner_now()),
                true,
                present_ok
            )
            .is_err()
        );
        // A marker mismatch is a refusal, never an adoption.
        assert!(verify_active_at(&path, 200, Some(session), None, true, mismatch_v4).is_err());

        let mut cleaning = j.clone();
        cleaning.phase = Phase::Cleaning;
        seed(&path, &cleaning);
        assert!(verify_active_at(&path, 200, None, None, true, present_ok).is_err());
    }

    #[test]
    fn sysctl_keys_accept_only_safe_paths() {
        assert_eq!(
            sysctl_tail("net.netfilter.nf_conntrack_acct").as_deref(),
            Some("net/netfilter/nf_conntrack_acct")
        );
        assert_eq!(
            sysctl_tail("net-net_filter/x_9-y").as_deref(),
            Some("net-net_filter/x_9-y")
        );
        assert!(sysctl_tail("").is_none());
        assert!(sysctl_tail("../x").is_none());
        assert!(sysctl_tail("net..x").is_none());
        assert!(sysctl_tail("net.").is_none());
        assert!(sysctl_tail(".net").is_none());
        assert!(sysctl_tail("net//x").is_none());
        assert!(sysctl_tail("net/x").is_some());
        assert_eq!(sysctl_tail("nf.conntrack").as_deref(), Some("nf/conntrack"));
        assert!(sysctl_tail("net x").is_none());
    }

    #[test]
    fn verified_reuse_with_both_flags_writes_nothing_and_drains_both() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let session = Uuid::new_v4();
        let mut j = journal(
            &Epoch::current().unwrap(),
            owner_now(),
            Phase::Active,
            true,
            true,
        );
        j.session_id = session;
        seed(&path, &j);
        let before = std::fs::read(&path).unwrap();
        install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            owner_now(),
            session,
            present_ok,
            name_absent,
            forbidden_install,
            never_remove,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a complete active record stays byte-for-byte unchanged"
        );
        // The later removal drains both recorded families.
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        assert!(remove_at(&path, owner_now(), None, remove).unwrap());
        assert_eq!(removed, vec!["v4", "v6"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn fresh_install_refuses_when_an_owned_table_name_is_present() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let name_there = |family: Family| Ok(matches!(family, Family::V4));
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_there,
            forbidden_install,
            never_remove,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("ipv4 table already exists"),
            "{err}"
        );
        assert!(on_disk(&path).is_none(), "no journal before any mutation");
    }

    #[test]
    fn create_conflict_drops_only_that_family_and_rolls_back_the_other() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        let install = |family: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            match family {
                Family::V4 => Ok(()),
                Family::V6 => Err(Error::Conflict("ipv6 table exists".into())),
            }
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            install,
            remove,
        )
        .unwrap_err();
        assert!(err.to_string().contains("ipv6 table exists"), "{err}");
        // The conflicted family was never owned, so its obligation is
        // dropped without a removal; the confirmed v4 creation is rolled
        // back and the journal clears.
        assert_eq!(removed, vec!["v4"]);
        assert!(on_disk(&path).is_none());
    }

    #[test]
    fn ambiguous_install_reply_preserves_the_journal_and_flags() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let install = |_f: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            Err(Error::Firewall("install reply timed out".into()))
        };
        let remove = |_f: Family, _s: Uuid| -> Result<()> {
            Err(Error::Firewall("removal reply lost".into()))
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            install,
            remove,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("install reply timed out"), "{msg}");
        assert!(msg.contains("removal reply lost"), "{msg}");
        let j = on_disk(&path).unwrap();
        assert_eq!(j.phase, Phase::Cleaning);
        assert!(j.ipv4, "an ambiguous reply keeps the obligation recorded");
        assert!(!j.ipv6);
    }

    #[test]
    fn legacy_journal_without_pidns_refuses_every_ownership_decision() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, true);
        j.pidns = None;
        seed(&path, &j);
        assert!(
            install_at(
                &path,
                200,
                DEFAULT_FWMARK,
                owner_now(),
                Uuid::new_v4(),
                present_ok,
                name_absent,
                forbidden_install,
                never_remove,
            )
            .is_err()
        );
        assert!(remove_at(&path, DEAD, None, never_remove).is_err());
        assert!(sweep_at(&path, |_f: Family| Ok(false), never_remove).is_err());
        assert!(verify_active_at(&path, 200, None, None, true, present_ok).is_err());
        assert_eq!(on_disk(&path), Some(j), "the legacy record is preserved");
    }

    #[test]
    fn foreign_pidns_journal_refuses_takeover_removal_and_sweep() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut j = journal(&Epoch::current().unwrap(), DEAD, Phase::Active, true, true);
        j.pidns = Some(state::PidNamespace {
            device: 999,
            inode: 999,
        });
        seed(&path, &j);
        assert!(
            install_at(
                &path,
                200,
                DEFAULT_FWMARK,
                owner_now(),
                Uuid::new_v4(),
                present_ok,
                name_absent,
                forbidden_install,
                never_remove,
            )
            .is_err()
        );
        assert!(remove_at(&path, DEAD, None, never_remove).is_err());
        assert!(sweep_at(&path, |_f: Family| Ok(false), never_remove).is_err());
        assert!(verify_active_at(&path, 200, None, None, true, present_ok).is_err());
        assert_eq!(on_disk(&path), Some(j));
    }

    #[test]
    fn sweep_table_covers_boot_namespace_and_legacy_cases() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let cur = Epoch::current().unwrap();
        let absent = |_f: Family| Ok(false);
        let present_v4 = |f: Family| Ok(matches!(f, Family::V4));
        let mut prior = cur.clone();
        prior.boot_id = Uuid::new_v4();
        let prior_j = journal(&prior, DEAD, Phase::Active, true, true);
        // Prior boot, both names absent: retire only the stale journal.
        seed(&path, &prior_j);
        assert!(sweep_at(&path, absent, never_remove).unwrap());
        assert!(on_disk(&path).is_none(), "no kernel object is touched");
        // Prior boot, a same-named table exists in this namespace: refuse.
        seed(&path, &prior_j);
        let err = sweep_at(&path, present_v4, never_remove).unwrap_err();
        assert!(err.to_string().contains("previous boot"), "{err}");
        assert_eq!(on_disk(&path), Some(prior_j));
        // Same boot, another network namespace: refuse, naming it.
        let mut foreign_ns = cur.clone();
        foreign_ns.namespace_inode += 1;
        let ns_j = journal(&foreign_ns, DEAD, Phase::Active, true, true);
        seed(&path, &ns_j);
        let err = sweep_at(&path, absent, never_remove).unwrap_err();
        assert!(err.to_string().contains("network namespace"), "{err}");
        assert_eq!(on_disk(&path), Some(ns_j));
        // A legacy record without PID-namespace evidence stays preserved.
        let mut legacy = journal(&cur, DEAD, Phase::Active, true, true);
        legacy.pidns = None;
        seed(&path, &legacy);
        assert!(sweep_at(&path, absent, never_remove).is_err());
        assert_eq!(on_disk(&path), Some(legacy));
    }

    /// Install family returning the typed timeout: the error surfaces
    /// unchanged, the rollback drains only the two owned families, and
    /// nothing else is touched.
    #[test]
    fn install_timeout_propagates_and_rolls_back_only_owned_families() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let mut removed = Vec::new();
        let remove = recording_remove(&mut removed);
        let timeout_v4 = |family: Family, _q: u16, _m: u32, _s: Uuid| -> Result<()> {
            match family {
                Family::V4 => Err(Error::TimedOut("delayed transport reply".into())),
                Family::V6 => Ok(()),
            }
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            timeout_v4,
            remove,
        )
        .unwrap_err();
        assert!(matches!(err, Error::TimedOut(_)), "typed timeout: {err}");
        assert_eq!(
            removed,
            vec!["v4"],
            "rollback touches exactly the recorded owned families; the foreign-named tables are never probed or touched"
        );
        assert!(on_disk(&path).is_none());
    }

    /// A timeout during the rollback itself leaves the kernel state
    /// uncertain, so the journal is retained for a later retry.
    #[test]
    fn timeout_during_rollback_keeps_the_journal() {
        let _guard = tx_test_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let never_confirms = |_family: Family, _session: Uuid| -> Result<()> {
            Err(Error::TimedOut("rollback exchange also timed out".into()))
        };
        let err = install_at(
            &path,
            200,
            DEFAULT_FWMARK,
            DEAD,
            Uuid::new_v4(),
            present_ok,
            name_absent,
            |family, _q, _m, _s| {
                if matches!(family, Family::V4) {
                    Err(Error::TimedOut("install timed out".into()))
                } else {
                    Ok(())
                }
            },
            never_confirms,
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        let j = on_disk(&path).expect("journal retained");
        assert_eq!(j.phase, Phase::Cleaning);
    }
}
