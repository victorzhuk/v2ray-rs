use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use uuid::Uuid;
pub const JOURNAL_VERSION: u32 = 1;
pub const SINGBOX_ROUTE_TABLE: u32 = 2022;
pub const RESERVATION_NAME: &str = "v2ray-rs-own";
const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session<B> {
    pub backend: B,
    pub iface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Epoch {
    pub boot_id: Uuid,
    pub namespace_device: u64,
    pub namespace_inode: u64,
}

impl Epoch {
    pub fn current() -> io::Result<Self> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let ns = std::fs::metadata("/proc/self/ns/net")?;
        Ok(Self {
            boot_id: Uuid::parse_str(boot.trim()).map_err(io::Error::other)?,
            namespace_device: ns.dev(),
            namespace_inode: ns.ino(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
}

impl ProcessIdentity {
    pub fn current() -> io::Result<Self> {
        let stat = std::fs::read_to_string("/proc/self/stat")?;
        let (_, fields) = stat
            .rsplit_once(") ")
            .ok_or_else(|| io::Error::other("invalid process stat"))?;
        let start_time_ticks = fields
            .split_whitespace()
            .nth(19)
            .ok_or_else(|| io::Error::other("process start time is missing"))?
            .parse()
            .map_err(io::Error::other)?;
        Ok(Self {
            pid: std::process::id(),
            start_time_ticks,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Prepared,
    Reserved,
    Applying,
    Active,
    Cleaning,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "kebab-case")]
pub enum Resource {
    Route(Vec<u8>),
    Rule(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ownership {
    pub version: u32,
    pub session_id: Uuid,
    pub epoch: Epoch,
    pub controller: ProcessIdentity,
    pub phase: Phase,
    pub device_ifindex: Option<u32>,
    pub reservation_ifindex: Option<u32>,
    pub resources: Vec<Resource>,
    pub pending: Option<Resource>,
}

impl Ownership {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            version: JOURNAL_VERSION,
            session_id: Uuid::new_v4(),
            epoch: Epoch::current()?,
            controller: ProcessIdentity::current()?,
            phase: Phase::Prepared,
            device_ifindex: None,
            reservation_ifindex: None,
            resources: Vec::new(),
            pending: None,
        })
    }
    pub fn controller_matches(&self) -> io::Result<bool> {
        Ok(self.controller == ProcessIdentity::current()?)
    }

    pub fn alias(&self) -> String {
        format!("v2ray-rs:{}", self.session_id)
    }
    pub fn verify_epoch(&self) -> io::Result<()> {
        if self.version != JOURNAL_VERSION {
            return Err(io::Error::other("unsupported TUN journal version"));
        }
        if self.epoch != Epoch::current()? {
            return Err(io::Error::other(
                "TUN journal belongs to another boot or network namespace",
            ));
        }
        Ok(())
    }
}

pub fn read<B: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Session<B>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // getuid has no preconditions and leaves the caller's credentials unchanged.
    let caller_uid = unsafe { libc::getuid() };
    if !metadata.is_file()
        || metadata.uid() != caller_uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(io::Error::other(
            "TUN journal must be an owner-only regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(io::Error::other("TUN journal exceeds size limit"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub fn write<B: Serialize>(path: &Path, session: &Session<B>) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("TUN journal has no parent"))?;
    let bytes = serde_json::to_vec_pretty(session).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(io::Error::other("TUN journal exceeds size limit"));
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn unsafe_journal_files_are_refused_without_modification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let session = Session {
            backend: "xray",
            iface: "tun0".to_owned(),
            ownership: Some(Ownership::new().unwrap()),
        };
        write(&path, &session).unwrap();
        let original = std::fs::read(&path).unwrap();
        let alias = dir.path().join("alias.json");
        symlink(&path, &alias).unwrap();
        assert!(read::<String>(&alias).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read::<String>(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn interrupted_mutation_intent_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let mut session = Session {
            backend: "xray".to_owned(),
            iface: "tun0".to_owned(),
            ownership: Some(Ownership::new().unwrap()),
        };
        let ownership = session.ownership.as_mut().unwrap();
        ownership.phase = Phase::Applying;
        ownership.pending = Some(Resource::Rule(vec![0]));
        write(&path, &session).unwrap();
        assert_eq!(read::<String>(&path).unwrap(), session);
        session.ownership.as_mut().unwrap().version += 1;
        write(&path, &session).unwrap();
        assert!(
            read::<String>(&path)
                .unwrap()
                .ownership
                .unwrap()
                .verify_epoch()
                .is_err()
        );
    }
}
