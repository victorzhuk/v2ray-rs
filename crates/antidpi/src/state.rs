//! Anti-DPI ownership journal: one JSON record per user tying the nftables
//! tables to the controller process that requested them and to the
//! boot/network-namespace epoch that observes them. The file is bounded,
//! owner-only (0600 in a 0700 directory) and replaced atomically, mirroring
//! the tun-session journal. The record also carries the PID namespace that
//! assigned the recorded PIDs: ownership and liveness decisions refuse a
//! journal written under a different — or unprovable — one. Journal
//! transactions serialize on one per-effective-UID, per-network-namespace
//! lock independent of the journal path, so two journal paths for the same
//! kernel objects cannot bypass each other.

use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

pub use v2ray_rs_tun_session::{Epoch, ProcessIdentity};

pub const JOURNAL_VERSION: u32 = 1;
const JOURNAL_NAME: &str = "antidpi.v1.json";
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
/// `libc::kill` takes a signed `pid_t`: a stored pid above this cannot be
/// probed or addressed and the journal carrying it is refused.
pub const MAX_OWNER_PID: u32 = i32::MAX as u32;

fn owner_pid_valid(pid: u32) -> bool {
    (1..=MAX_OWNER_PID).contains(&pid)
}

/// `phase` moves `applying` -> `active` on a successful install and
/// `cleaning` before the first removal, so every crash window leaves
/// recoverable intent behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Applying,
    Active,
    Cleaning,
}

/// The PID namespace that assigned the recorded PIDs (`/proc/self/ns/pid`
/// device and inode). A numeric PID is meaningful only in the namespace
/// that assigned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidNamespace {
    pub device: u64,
    pub inode: u64,
}

/// This process's PID namespace.
pub fn current_pidns() -> io::Result<PidNamespace> {
    let ns = std::fs::metadata("/proc/self/ns/pid")?;
    Ok(PidNamespace {
        device: ns.dev(),
        inode: ns.ino(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub version: u32,
    pub session_id: uuid::Uuid,
    pub epoch: Epoch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pidns: Option<PidNamespace>,
    pub owner: ProcessIdentity,
    pub phase: Phase,
    pub queue: u16,
    pub fwmark: u32,
    pub ipv4: bool,
    pub ipv6: bool,
}

impl Journal {
    /// Fresh `applying` record for this session. Written before the first
    /// kernel mutation.
    pub fn new(
        queue: u16,
        fwmark: u32,
        owner: ProcessIdentity,
        session_id: uuid::Uuid,
    ) -> io::Result<Self> {
        Ok(Self {
            version: JOURNAL_VERSION,
            session_id,
            epoch: Epoch::current()?,
            pidns: Some(current_pidns()?),
            owner,
            phase: Phase::Applying,
            queue,
            fwmark,
            ipv4: false,
            ipv6: false,
        })
    }
}

/// Refuses journals that lack matching PID-namespace evidence: a PIDs-only
/// identity cannot authorize deletion or takeover across that boundary.
pub fn check_pidns(journal: &Journal) -> io::Result<()> {
    let current = current_pidns()?;
    match journal.pidns {
        Some(p) if p == current => Ok(()),
        Some(p) => Err(io::Error::other(format!(
            "the journal was written under PID namespace {}:{} (this process: {}:{})",
            p.device, p.inode, current.device, current.inode
        ))),
        None => Err(io::Error::other(
            "the journal carries no PID-namespace evidence",
        )),
    }
}

/// Journal path: `V2RAYRS_ANTIDPI_STATE` overrides; otherwise
/// `$XDG_STATE_HOME/v2ray-rs/antidpi.v1.json` with the `~/.local/state`
/// fallback. Both bases must be absolute: a relative or missing base is an
/// error, never a silent fallback.
pub fn state_path() -> io::Result<PathBuf> {
    state_path_from(
        std::env::var("V2RAYRS_ANTIDPI_STATE").ok().as_deref(),
        std::env::var("XDG_STATE_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Pure core of `state_path` for the variables it reads.
fn state_path_from(
    override_path: Option<&str>,
    xdg: Option<&str>,
    home: Option<&str>,
) -> io::Result<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let base = if let Some(x) = xdg.filter(|x| !x.is_empty()) {
        let path = Path::new(x);
        if !path.is_absolute() {
            return Err(io::Error::other(
                "anti-DPI XDG_STATE_HOME must be an absolute path",
            ));
        }
        PathBuf::from(path)
    } else {
        let Some(h) = home.filter(|h| !h.is_empty()) else {
            return Err(io::Error::other(
                "anti-DPI journal location is unknown: set HOME or V2RAYRS_ANTIDPI_STATE",
            ));
        };
        if !Path::new(h).is_absolute() {
            return Err(io::Error::other("anti-DPI HOME must be an absolute path"));
        }
        Path::new(h).join(".local/state")
    };
    Ok(base.join("v2ray-rs").join(JOURNAL_NAME))
}

/// Reads a journal: bounded (64 KiB), regular file, owner-only, no symlinks,
/// known schema version. Any other failure is an error so foreign or corrupt
/// state is never silently overwritten.
pub fn read_journal(path: &Path) -> io::Result<Journal> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // geteuid has no preconditions and leaves the caller's credentials unchanged.
    let caller_uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != caller_uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(io::Error::other(
            "anti-DPI journal must be an owner-only regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(io::Error::other("anti-DPI journal exceeds size limit"));
    }
    let journal: Journal = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if journal.version != JOURNAL_VERSION {
        return Err(io::Error::other("unsupported anti-DPI journal version"));
    }
    if !owner_pid_valid(journal.owner.pid) {
        return Err(io::Error::other(
            "anti-DPI journal owner pid is out of range",
        ));
    }
    Ok(journal)
}

/// Atomically replaces the journal through the pinned parent directory
/// handle: the temporary file is created with `openat(dir)`, fsynced, and
/// moved into place with `renameat(dir, dir)`. After the parent check no
/// step goes through the path again; any failure unlinks the temporary
/// file. Missing components are created with mode 0700, an unsuitable
/// existing parent is refused without modifying it.
pub fn write_journal(path: &Path, journal: &Journal) -> io::Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("anti-DPI journal path must end in a file name"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| io::Error::other("anti-DPI journal path has no parent"))?;
    let dir = open_journal_parent(parent)?;
    let bytes = serde_json::to_vec_pretty(journal).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(io::Error::other("anti-DPI journal exceeds size limit"));
    }
    let name_c = CString::new(name)
        .map_err(|_| io::Error::other("anti-DPI journal path must end in a file name"))?;
    let temp = format!(".{name}.{}", uuid::Uuid::new_v4().simple());
    let temp_c = CString::new(temp.as_str())
        .map_err(|_| io::Error::other("anti-DPI journal path must end in a file name"))?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            temp_c.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            FILE_MODE,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { <File as FromRawFd>::from_raw_fd(fd) };
    let mut result = (|| -> io::Result<()> {
        file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_ok() {
        let renamed = unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                temp_c.as_ptr(),
                dir.as_raw_fd(),
                name_c.as_ptr(),
            )
        };
        if renamed != 0 {
            result = Err(io::Error::last_os_error());
        } else {
            drop(file);
            result = dir.sync_all();
        }
    }
    if result.is_err() {
        unsafe { libc::unlinkat(dir.as_raw_fd(), temp_c.as_ptr(), 0) };
    }
    result
}

/// Opens the journal's parent directory after pinning what the checks saw:
/// the handle is opened with `O_DIRECTORY|O_NOFOLLOW` and must reference
/// the device/inode the metadata described, so a swap between check and
/// open is caught before anything is written into it.
fn open_journal_parent(parent: &Path) -> io::Result<File> {
    // geteuid has no preconditions and leaves the caller's credentials unchanged.
    let caller_uid = unsafe { libc::geteuid() };
    let checked = match std::fs::symlink_metadata(parent) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            create_private_dirs(parent, caller_uid)?;
            std::fs::symlink_metadata(parent)?
        }
        Err(e) => return Err(e),
        Ok(md) => {
            verify_private_dir(parent, &md, caller_uid)?;
            md
        }
    };
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(parent)?;
    let opened = dir.metadata()?;
    if (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
        return Err(io::Error::other(
            "anti-DPI journal parent changed while it was being opened",
        ));
    }
    Ok(dir)
}

/// Creates each missing component with mode 0700 (verified after
/// creation). Existing ancestors above the immediate parent — a normal
/// `0755` `~/.local/state` — are passed through unverified and unmodified;
/// only the immediate journal parent is required to be private, which the
/// caller (`open_journal_parent`) verifies.
fn create_private_dirs(parent: &Path, caller_uid: u32) -> io::Result<()> {
    let mut missing: Vec<&Path> = Vec::new();
    let mut cursor = parent;
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor);
                match cursor.parent() {
                    Some(p) if p != cursor => cursor = p,
                    _ => {
                        return Err(io::Error::other(
                            "anti-DPI journal parent has no existing ancestor",
                        ));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
    for component in missing.iter().rev() {
        std::fs::DirBuilder::new()
            .mode(DIR_MODE)
            .create(component)?;
        let md = std::fs::symlink_metadata(component)?;
        verify_private_dir(component, &md, caller_uid)?;
    }
    Ok(())
}

/// Real directory (never a symlink), owned by the caller, owner-only
/// permissions. A refusal never modifies the directory.
fn verify_private_dir(path: &Path, md: &std::fs::Metadata, caller_uid: u32) -> io::Result<()> {
    if !md.is_dir() {
        return Err(io::Error::other(format!(
            "anti-DPI journal parent {path:?} must be a real directory, not a symlink or other file"
        )));
    }
    if md.uid() != caller_uid {
        return Err(io::Error::other(format!(
            "anti-DPI journal parent {path:?} must be owned by the caller"
        )));
    }
    if md.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "anti-DPI journal parent {path:?} must have owner-only permissions (mode {:o})",
            md.mode() & 0o777
        )));
    }
    Ok(())
}

/// Removes the journal; an absent file is already the desired state.
pub fn remove_journal(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The anti-DPI transaction lock: one per-effective-UID, per-network-
/// namespace lock independent of the journal path, so two journal paths
/// for the same kernel objects serialize on the same lock. The runtime
/// directory is `/run/user/<euid>` (for euid 0: `/run/v2ray-rs-antidpi`,
/// created when absent); an unusable runtime directory fails closed.
pub fn lock_transaction() -> io::Result<File> {
    let euid = unsafe { libc::geteuid() };
    let dir = runtime_dir(euid)?;
    let netns = std::fs::metadata("/proc/self/ns/net")?.ino();
    lock_transaction_in(&dir, netns)
}

/// Locates and validates the euid's transaction-lock directory: a real,
/// caller-owned, 0700 directory — verified, never modified. Only root's
/// directory is created when absent.
fn runtime_dir(euid: u32) -> io::Result<PathBuf> {
    let dir = if euid == 0 {
        PathBuf::from("/run/v2ray-rs-antidpi")
    } else {
        PathBuf::from(format!("/run/user/{euid}"))
    };
    let md = match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound && euid == 0 => {
            std::fs::DirBuilder::new().mode(DIR_MODE).create(&dir)?;
            std::fs::symlink_metadata(&dir)?
        }
        Err(e) => return Err(e),
        Ok(md) => md,
    };
    validate_runtime_dir(&dir, &md, euid)?;
    Ok(dir)
}

fn validate_runtime_dir(dir: &Path, md: &std::fs::Metadata, euid: u32) -> io::Result<()> {
    if !md.is_dir() {
        return Err(io::Error::other(format!(
            "anti-DPI runtime directory {dir:?} must be a real directory"
        )));
    }
    if md.uid() != euid {
        return Err(io::Error::other(format!(
            "anti-DPI runtime directory {dir:?} must be owned by euid {euid}"
        )));
    }
    if md.mode() & 0o777 != DIR_MODE {
        return Err(io::Error::other(format!(
            "anti-DPI runtime directory {dir:?} must have mode 0700"
        )));
    }
    Ok(())
}

/// Takes the named transaction lock inside `dir`: mode 0600, `O_NOFOLLOW`,
/// a regular file with a single link, locked with one non-blocking
/// exclusive `flock`. The lock file is never unlinked; a busy lock fails
/// without touching the journal.
pub(crate) fn lock_transaction_in(dir: &Path, netns_inode: u64) -> io::Result<File> {
    let lock_path = dir.join(format!("v2ray-rs-antidpi-{netns_inode}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)?;
    let metadata = file.metadata()?;
    // Type, owner, and link count must hold before anything is modified: a
    // foreign or hardlinked lock is refused, not repaired.
    let euid = unsafe { libc::geteuid() };
    if !metadata.is_file() || metadata.uid() != euid || metadata.nlink() != 1 {
        return Err(io::Error::other(
            "the anti-DPI transaction lock must be a regular, singly linked, caller-owned file",
        ));
    }
    // Repair a mode loosened by an unusual umask, then require the repaired
    // mode to be owner-only.
    file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))?;
    if file.metadata()?.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "the anti-DPI transaction lock must be owner-only",
        ));
    }
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

/// Reads field 22 of `/proc/<pid>/stat`: the start time in clock ticks,
/// which pins an identity against pid reuse.
pub fn start_time_ticks(pid: u32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::other("invalid process stat"))?;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::other("process start time is missing"))?
        .parse()
        .map_err(io::Error::other)
}

/// The controller that spawned this helper: parent pid plus its start ticks.
/// A lost parent (ppid 0/1) is refused: nobody could own or later clean up
/// the recorded state.
pub fn parent_identity() -> io::Result<ProcessIdentity> {
    let pid = unsafe { libc::getppid() };
    if pid <= 1 {
        return Err(io::Error::other(
            "no owning parent process for the anti-DPI journal",
        ));
    }
    Ok(ProcessIdentity {
        pid: pid as u32,
        start_time_ticks: start_time_ticks(pid as u32)?,
    })
}

/// Parses `--owner PID:START`; both fields are required decimal, pid 0/1 is
/// refused and the pid must fit the kernel's signed `pid_t` range.
pub fn parse_owner(s: &str) -> io::Result<ProcessIdentity> {
    let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg.to_string());
    let Some((pid_s, ticks_s)) = s.split_once(':') else {
        return Err(invalid("owner must be PID:START"));
    };
    let pid: u32 = pid_s
        .parse()
        .map_err(|_| invalid("owner pid must be a decimal number"))?;
    let start_time_ticks: u64 = ticks_s
        .parse()
        .map_err(|_| invalid("owner start time must be a decimal number"))?;
    if pid <= 1 {
        return Err(invalid("pid 0 and pid 1 cannot own anti-DPI state"));
    }
    if pid > MAX_OWNER_PID {
        return Err(invalid("owner pid must be at most 2147483647"));
    }
    Ok(ProcessIdentity {
        pid,
        start_time_ticks,
    })
}

/// `--owner` when given, otherwise the spawning controller. An asserted
/// owner must be this process's actual parent: resolving a path or
/// asserting a PID is not authorization, so anything else is refused.
pub fn resolve_owner(explicit: Option<&str>) -> io::Result<ProcessIdentity> {
    match explicit {
        Some(s) => {
            let claimed = parse_owner(s)?;
            let parent_pid = unsafe { libc::getppid() };
            if parent_pid <= 1 || claimed.pid != parent_pid as u32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the asserted owner is not this helper's parent process",
                ));
            }
            if claimed.start_time_ticks != start_time_ticks(parent_pid as u32)? {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the asserted owner's start time does not match this helper's parent",
                ));
            }
            Ok(claimed)
        }
        None => parent_identity(),
    }
}

/// Owner liveness: `Alive` pins the pid via matching start ticks, `Dead`
/// needs a different tick count or a `/proc` entry confirmed missing by an
/// `ESRCH` kill probe, and everything else (an unprovable pid, permissions,
/// read errors, ambiguous probes) stays `Unknown`, which every caller must
/// refuse on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Dead,
    Unknown,
}

/// Derives liveness from start ticks, protecting against pid reuse. A pid
/// that `libc::kill` cannot address (0, or one above `i32::MAX`) is never
/// probed and stays `Unknown`.
pub fn owner_liveness(owner: &ProcessIdentity) -> Liveness {
    if !owner_pid_valid(owner.pid) {
        return Liveness::Unknown;
    }
    match start_time_ticks(owner.pid) {
        Ok(ticks) if ticks == owner.start_time_ticks => Liveness::Alive,
        Ok(_) => Liveness::Dead,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // `kill(pid, 0)` only performs the existence check: ESRCH
            // confirms the process is gone, any other outcome is ambiguous.
            let probed = unsafe { libc::kill(owner.pid as libc::pid_t, 0) };
            if probed == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                Liveness::Dead
            } else {
                Liveness::Unknown
            }
        }
        Err(_) => Liveness::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(pid: u32, ticks: u64) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            start_time_ticks: ticks,
        }
    }

    fn sample_journal(owner: ProcessIdentity) -> Journal {
        Journal {
            version: JOURNAL_VERSION,
            session_id: uuid::Uuid::new_v4(),
            epoch: Epoch::current().unwrap(),
            pidns: Some(current_pidns().unwrap()),
            owner,
            phase: Phase::Applying,
            queue: 200,
            fwmark: 0x4000_0000,
            ipv4: true,
            ipv6: false,
        }
    }

    #[test]
    fn journal_round_trips_with_private_modes() {
        let dir = tempfile::tempdir().unwrap();
        // Write one level deep so the 0700 directory is one we created.
        let path = dir.path().join("state/v2ray-rs/antidpi.v1.json");
        let journal = sample_journal(owner(2_000_000_000, 1));
        write_journal(&path, &journal).unwrap();
        assert_eq!(read_journal(&path).unwrap(), journal);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn world_readable_journal_is_refused_without_modification() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        let journal = sample_journal(owner(2_000_000_000, 1));
        write_journal(&path, &journal).unwrap();
        let original = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_journal(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn owner_parse_validates_both_fields() {
        assert_eq!(parse_owner("123:456").unwrap(), owner(123, 456));
        assert!(parse_owner("").is_err());
        assert!(parse_owner("123").is_err());
        assert!(parse_owner("123:").is_err());
        assert!(parse_owner(":456").is_err());
        assert!(parse_owner("0:456").is_err());
        assert!(parse_owner("1:456").is_err());
        assert!(parse_owner("x:456").is_err());
        assert!(parse_owner("123:x").is_err());
        assert!(parse_owner("123:-1").is_err());
        assert!(parse_owner("4294967296:1").is_err());
        assert!(parse_owner("2147483648:456").is_err());
        assert_eq!(
            parse_owner("2147483647:456").unwrap(),
            owner(2147483647, 456)
        );
        assert!(parse_owner("1:99999999999999999999999").is_err());
    }

    #[test]
    fn owner_liveness_matches_start_ticks_and_probes() {
        let me = ProcessIdentity::current().unwrap();
        assert_eq!(owner_liveness(&me), Liveness::Alive);
        // Above the kernel's pid limit (2^22) and inside the signed
        // `pid_t` range: this owner can never exist, and the kill probe
        // addresses a process, never a process group.
        assert_eq!(owner_liveness(&owner(2_000_000_000, 1)), Liveness::Dead);
        // Different start ticks mean the pid was reused by someone else.
        assert_eq!(
            owner_liveness(&owner(me.pid, me.start_time_ticks.wrapping_add(1))),
            Liveness::Dead
        );
        // Pid 0 has no /proc entry and `kill(0, 0)` probes the process
        // group instead, so death cannot be established.
        assert_eq!(owner_liveness(&owner(0, 1)), Liveness::Unknown);
        // A pid `libc::kill` cannot address is never probed.
        assert_eq!(
            owner_liveness(&owner(i32::MAX as u32 + 1, 1)),
            Liveness::Unknown
        );
        assert_eq!(owner_liveness(&owner(u32::MAX, 1)), Liveness::Unknown);
    }

    #[test]
    fn a_world_readable_ancestor_does_not_block_journal_creation() {
        let dir = tempfile::tempdir().unwrap();
        // A normal 0755 ancestor exists; only the missing journal directory
        // below it is created, private, and nothing above is touched.
        let ancestor = dir.path().join("state");
        std::fs::create_dir(&ancestor).unwrap();
        std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = ancestor.join("v2ray-rs/antidpi.v1.json");
        let journal = sample_journal(owner(2_000_000_000, 1));
        write_journal(&path, &journal).unwrap();
        assert_eq!(read_journal(&path).unwrap(), journal);
        let child = std::fs::symlink_metadata(ancestor.join("v2ray-rs")).unwrap();
        assert!(child.is_dir());
        assert_eq!(child.permissions().mode() & 0o777, 0o700);
        let md = std::fs::symlink_metadata(&ancestor).unwrap();
        assert_eq!(md.permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn an_existing_permissive_or_symlinked_parent_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(owner(2_000_000_000, 1));
        // An existing 0755 journal parent is refused without modification.
        let permissive = dir.path().join("journal.json");
        std::fs::create_dir(&permissive).unwrap();
        std::fs::set_permissions(&permissive, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(write_journal(&permissive, &journal).is_err());
        let md = std::fs::symlink_metadata(&permissive).unwrap();
        assert_eq!(md.permissions().mode() & 0o777, 0o755);
        // A symlinked parent is never followed: nothing is written through
        // it and the target stays empty.
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(write_journal(&link.join("j.json"), &journal).is_err());
        assert!(std::fs::symlink_metadata(real.join("j.json")).is_err());
    }

    #[test]
    fn parent_identity_resolves_for_spawned_processes() {
        // The test runner was spawned by something that is still alive.
        let parent = parent_identity().unwrap();
        assert!(parent.pid > 1);
        assert_eq!(owner_liveness(&parent), Liveness::Alive);
    }

    #[test]
    fn journal_with_out_of_range_owner_pid_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        write_journal(&path, &sample_journal(owner(u32::MAX, 1))).unwrap();
        assert!(read_journal(&path).is_err());
    }

    #[test]
    fn state_path_requires_an_absolute_base() {
        assert_eq!(
            state_path_from(Some("/x/j.json"), None, None).unwrap(),
            PathBuf::from("/x/j.json")
        );
        assert_eq!(
            state_path_from(None, Some("/state"), None).unwrap(),
            PathBuf::from("/state/v2ray-rs/antidpi.v1.json")
        );
        assert_eq!(
            state_path_from(None, None, Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/v2ray-rs/antidpi.v1.json")
        );
        assert!(state_path_from(None, None, None).is_err());
        assert!(state_path_from(None, None, Some("")).is_err());
        assert!(state_path_from(None, Some("rel/state"), None).is_err());
        assert!(state_path_from(None, None, Some("rel")).is_err());
        // An empty override is ignored; the home fallback applies.
        assert_eq!(
            state_path_from(Some(""), None, Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/v2ray-rs/antidpi.v1.json")
        );
    }

    #[test]
    fn journal_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("journal.json");
        write_journal(&path, &sample_journal(owner(2_000_000_000, 1))).unwrap();
        let entries = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(entries, 1, "only the journal itself remains");
    }
}
