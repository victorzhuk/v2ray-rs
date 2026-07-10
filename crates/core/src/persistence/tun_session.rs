use serde::{Deserialize, Serialize};

use crate::fs::atomic_write;
use crate::models::BackendType;

use super::{AppPaths, PersistenceError, read_file};

/// Marker recording that a TUN session was active. Written when a TUN
/// connection starts and removed on a clean stop; its presence at startup means
/// the previous run exited uncleanly and a route-recovery pass is needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunSession {
    pub backend: BackendType,
    pub iface: String,
}

pub fn save_tun_session(paths: &AppPaths, session: &TunSession) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(session)?;
    atomic_write(&paths.tun_session_path(), json.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_tun_session(paths: &AppPaths) -> Option<TunSession> {
    let path = paths.tun_session_path();
    if !path.exists() {
        return None;
    }
    let contents = match read_file(&path) {
        Ok(contents) => contents,
        Err(err) => {
            log::warn!("read tun session marker: {err}");
            return None;
        }
    };
    match serde_json::from_str(&contents) {
        Ok(session) => Some(session),
        Err(err) => {
            // An unreadable marker would otherwise linger forever and silently
            // disable the recovery pass on every launch.
            log::warn!("corrupt tun session marker, discarding: {err}");
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

pub fn clear_tun_session(paths: &AppPaths) -> Result<(), PersistenceError> {
    match std::fs::remove_file(paths.tun_session_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PersistenceError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_clear_roundtrip() {
        let (_tmp, paths) = super::super::test_paths();
        assert!(load_tun_session(&paths).is_none());

        let session = TunSession {
            backend: BackendType::Xray,
            iface: "tun0".to_string(),
        };
        save_tun_session(&paths, &session).unwrap();
        assert_eq!(load_tun_session(&paths), Some(session));

        clear_tun_session(&paths).unwrap();
        assert!(load_tun_session(&paths).is_none());
        // Clearing an absent marker is a no-op.
        clear_tun_session(&paths).unwrap();
    }

    #[test]
    fn corrupt_marker_discarded_on_load() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(paths.tun_session_path(), "{not json").unwrap();

        assert!(load_tun_session(&paths).is_none());
        assert!(
            !paths.tun_session_path().exists(),
            "corrupt marker should be removed"
        );
    }
}
