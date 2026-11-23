use crate::models::BackendType;

use super::{AppPaths, PersistenceError};

/// Marker recording that a TUN session was active. Written when a TUN
/// connection starts and removed on a clean stop; its presence at startup means
/// the previous run exited uncleanly and a route-recovery pass is needed.
pub type TunSession = v2ray_rs_tun_session::Session<BackendType>;
pub use v2ray_rs_tun_session::{
    Epoch as TunEpoch, Ownership as TunOwnership, Phase as TunPhase, SINGBOX_ROUTE_TABLE,
};

pub fn save_tun_session(paths: &AppPaths, session: &TunSession) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    v2ray_rs_tun_session::write(&paths.tun_session_path(), session).map_err(PersistenceError::Io)
}

pub fn prepare_tun_session(
    paths: &AppPaths,
    backend: BackendType,
    iface: &str,
) -> Result<TunSession, PersistenceError> {
    crate::models::validate_tun_interface_name(iface).map_err(std::io::Error::other)?;
    paths.ensure_dirs()?;
    if journal_exists(&paths.legacy_tun_session_path())? {
        return Err(legacy_recovery_pending_error(paths, iface).into());
    }
    if let Some(session) = load_tun_session_result(paths)?
        && !retirable_tun_session(&session)
    {
        let ownership = session
            .ownership
            .as_ref()
            .ok_or_else(|| std::io::Error::other("TUN ownership evidence is missing"))?;
        ownership.verify_epoch()?;
        if session.backend != backend || session.iface != iface {
            return Err(
                std::io::Error::other("another TUN session still owns recovery state").into(),
            );
        }
        if ownership.phase != TunPhase::Complete {
            if !ownership.controller_matches()? {
                return Err(std::io::Error::other(
                    "TUN recovery from another application run is pending",
                )
                .into());
            }
            return Ok(session);
        }
        return Err(
            std::io::Error::other("completed TUN journal contains unresolved resources").into(),
        );
    }
    // A retirable journal still sits on disk here: the fresh session below
    // replaces the file only, so it is safe even when the record came from a
    // previous boot or network namespace.
    let session = TunSession {
        backend,
        iface: iface.to_owned(),
        ownership: Some(TunOwnership::new()?),
    };
    save_tun_session(paths, &session)?;
    Ok(session)
}

/// True when the journal records no kernel state, so retiring it removes or
/// replaces only the file: a finished record, or a sing-box record that
/// reserved nothing yet. Every unfinished record keeps the file.
pub fn retirable_tun_session(session: &TunSession) -> bool {
    let Some(ownership) = session.ownership.as_ref() else {
        return false;
    };
    let unstarted_singbox = session.backend == BackendType::SingBox
        && ownership.phase == TunPhase::Prepared
        && ownership.device_ifindex.is_none()
        && ownership.reservation_ifindex.is_none();
    ownership.resources.is_empty()
        && ownership.pending.is_none()
        && (ownership.phase == TunPhase::Complete || unstarted_singbox)
}

/// Whether the journal at `path` now records a session other than
/// `expected_id`. Only a record without kernel state is ever replaced
/// (`retirable_tun_session`, `renew_completed_tun_session`), so a manager
/// whose session is gone holds nothing to release. An unreadable journal
/// reports `false`: the teardown's own identity check then decides.
pub fn tun_session_replaced(path: &std::path::Path, expected_id: &str) -> bool {
    v2ray_rs_tun_session::read::<BackendType>(path).is_ok_and(|session| {
        session
            .ownership
            .is_some_and(|ownership| ownership.session_id.to_string() != expected_id)
    })
}

/// The legacy marker never clears itself, so the error must name the exact
/// file and the verified-cleanup command the operator has to run.
fn legacy_recovery_pending_error(paths: &AppPaths, requested_iface: &str) -> std::io::Error {
    let legacy = paths.legacy_tun_session_path();
    let recorded = v2ray_rs_tun_session::read::<BackendType>(&legacy).ok();
    let flag = match recorded.as_ref().map(|session| session.backend) {
        Some(BackendType::SingBox) => "--singbox",
        _ => "--xray",
    };
    let iface = recorded
        .map(|session| session.iface)
        .unwrap_or_else(|| requested_iface.to_owned());
    std::io::Error::other(format!(
        "legacy TUN recovery journal {} is pending; verify kernel cleanup with \
         `v2ray-rs-netctl recover --iface {iface} {flag} --force-legacy`, then remove the file",
        legacy.display()
    ))
}

pub fn renew_completed_tun_session(
    path: &std::path::Path,
    expected_id: &str,
) -> Result<String, PersistenceError> {
    let mut session: TunSession = v2ray_rs_tun_session::read(path)?;
    let ownership = session
        .ownership
        .as_ref()
        .ok_or_else(|| std::io::Error::other("TUN ownership evidence is missing"))?;
    ownership.verify_epoch()?;
    if ownership.session_id.to_string() != expected_id || !ownership.controller_matches()? {
        return Err(std::io::Error::other("TUN journal identity changed").into());
    }
    if ownership.phase != TunPhase::Complete {
        return Ok(expected_id.to_owned());
    }
    if !ownership.resources.is_empty() || ownership.pending.is_some() {
        return Err(
            std::io::Error::other("completed TUN journal contains unresolved resources").into(),
        );
    }
    let ownership = TunOwnership::new()?;
    let identity = ownership.session_id.to_string();
    session.ownership = Some(ownership);
    v2ray_rs_tun_session::write(path, &session)?;
    Ok(identity)
}

fn journal_exists(path: &std::path::Path) -> std::io::Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn tun_session_pending(paths: &AppPaths) -> Result<bool, PersistenceError> {
    Ok(journal_exists(&paths.tun_session_path())?
        || journal_exists(&paths.legacy_tun_session_path())?)
}

pub fn load_tun_session_result(paths: &AppPaths) -> Result<Option<TunSession>, PersistenceError> {
    let current = paths.tun_session_path();
    let legacy = paths.legacy_tun_session_path();
    let path = if journal_exists(&current)? {
        current
    } else if journal_exists(&legacy)? {
        legacy
    } else {
        return Ok(None);
    };
    v2ray_rs_tun_session::read(&path)
        .map(Some)
        .map_err(PersistenceError::Io)
}

pub fn load_tun_session(paths: &AppPaths) -> Option<TunSession> {
    match load_tun_session_result(paths) {
        Ok(session) => session,
        Err(err) => {
            log::warn!("TUN recovery journal retained: {err}");
            None
        }
    }
}

/// Records completion after the caller verifies that kernel state is absent.
pub fn complete_tun_session(
    path: &std::path::Path,
    expected_id: &str,
) -> Result<(), PersistenceError> {
    update_singbox_phase(path, expected_id, TunPhase::Complete)
}

pub fn record_singbox_launch(
    path: &std::path::Path,
    expected_id: &str,
) -> Result<(), PersistenceError> {
    update_singbox_phase(path, expected_id, TunPhase::Applying)
}

fn update_singbox_phase(
    path: &std::path::Path,
    expected_id: &str,
    phase: TunPhase,
) -> Result<(), PersistenceError> {
    let mut session: TunSession = v2ray_rs_tun_session::read(path)?;
    let ownership = session
        .ownership
        .as_mut()
        .ok_or_else(|| std::io::Error::other("TUN ownership evidence is missing"))?;
    ownership.verify_epoch()?;
    if ownership.session_id.to_string() != expected_id
        || session.backend != BackendType::SingBox
        || !ownership.controller_matches()?
    {
        return Err(std::io::Error::other("TUN journal identity changed").into());
    }
    if !ownership.resources.is_empty()
        || ownership.pending.is_some()
        || ownership.phase == TunPhase::Complete && phase != TunPhase::Complete
    {
        return Err(std::io::Error::other("TUN journal cannot enter the requested phase").into());
    }
    ownership.phase = phase;
    v2ray_rs_tun_session::write(path, &session)?;
    Ok(())
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
            ownership: None,
        };
        save_tun_session(&paths, &session).unwrap();
        assert_eq!(load_tun_session(&paths), Some(session));

        clear_tun_session(&paths).unwrap();
        assert!(load_tun_session(&paths).is_none());
        // Clearing an absent marker is a no-op.
        clear_tun_session(&paths).unwrap();
    }

    #[test]
    fn corrupt_marker_retained_on_load() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(paths.tun_session_path(), "{not json").unwrap();

        assert!(load_tun_session(&paths).is_none());
        assert!(
            paths.tun_session_path().exists(),
            "corrupt marker must be retained"
        );
    }
    #[test]
    fn legacy_recovery_is_readable_but_blocks_new_sessions() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        let session = TunSession {
            backend: BackendType::Xray,
            iface: "tun0".into(),
            ownership: None,
        };
        v2ray_rs_tun_session::write(&paths.legacy_tun_session_path(), &session).unwrap();
        let original = std::fs::read(paths.legacy_tun_session_path()).unwrap();
        assert_eq!(load_tun_session_result(&paths).unwrap(), Some(session));
        let error = prepare_tun_session(&paths, BackendType::Xray, "tun0")
            .unwrap_err()
            .to_string();
        assert!(error.contains("--force-legacy"), "{error}");
        assert!(error.contains("--xray"), "{error}");
        assert!(error.contains("tun0"), "{error}");
        assert!(
            error.contains(paths.legacy_tun_session_path().to_string_lossy().as_ref()),
            "{error}"
        );
        assert!(!paths.tun_session_path().exists());
        assert_eq!(
            std::fs::read(paths.legacy_tun_session_path()).unwrap(),
            original
        );
    }

    #[test]
    fn foreign_epoch_record_is_replaced_only_when_finished() {
        for (backend, phase, replaced) in [
            (BackendType::Xray, TunPhase::Complete, true),
            (BackendType::SingBox, TunPhase::Prepared, true),
            (BackendType::Xray, TunPhase::Applying, false),
        ] {
            let (_tmp, paths) = super::super::test_paths();
            let mut session = TunSession {
                backend,
                iface: "tun0".into(),
                ownership: Some(TunOwnership::new().unwrap()),
            };
            let ownership = session.ownership.as_mut().unwrap();
            ownership.phase = phase;
            ownership.epoch.namespace_inode += 1;
            let old = ownership.session_id;
            save_tun_session(&paths, &session).unwrap();
            let result = prepare_tun_session(&paths, backend, "tun0");
            if replaced {
                let prepared = result.unwrap();
                assert_ne!(prepared.ownership.as_ref().unwrap().session_id, old);
                assert_eq!(prepared.backend, backend);
                assert_eq!(prepared.iface, "tun0");
            } else {
                assert!(result.is_err());
                assert_eq!(load_tun_session_result(&paths).unwrap(), Some(session));
            }
        }
    }

    #[test]
    fn completed_journal_with_unresolved_state_is_refused_not_replaced() {
        for (resources, pending) in [
            (vec![v2ray_rs_tun_session::Resource::Rule(vec![0])], None),
            (
                Vec::new(),
                Some(v2ray_rs_tun_session::Resource::Route(vec![1])),
            ),
        ] {
            let (_tmp, paths) = super::super::test_paths();
            let mut session = TunSession {
                backend: BackendType::Xray,
                iface: "tun0".into(),
                ownership: Some(TunOwnership::new().unwrap()),
            };
            let ownership = session.ownership.as_mut().unwrap();
            ownership.phase = TunPhase::Complete;
            ownership.resources = resources;
            ownership.pending = pending;
            save_tun_session(&paths, &session).unwrap();
            assert!(prepare_tun_session(&paths, BackendType::Xray, "tun0").is_err());
            assert_eq!(load_tun_session_result(&paths).unwrap(), Some(session));
        }
    }

    #[test]
    fn preparation_preserves_owned_state_and_refuses_another_controller() {
        let (_tmp, paths) = super::super::test_paths();
        let mut session = prepare_tun_session(&paths, BackendType::Xray, "tun0").unwrap();
        assert_eq!(
            prepare_tun_session(&paths, BackendType::Xray, "tun0").unwrap(),
            session
        );
        session
            .ownership
            .as_mut()
            .unwrap()
            .controller
            .start_time_ticks += 1;
        save_tun_session(&paths, &session).unwrap();
        assert!(prepare_tun_session(&paths, BackendType::Xray, "tun0").is_err());
        assert_eq!(load_tun_session_result(&paths).unwrap(), Some(session));
    }
    #[test]
    fn completed_session_renews_but_unfinished_session_keeps_identity() {
        for backend in [BackendType::Xray, BackendType::SingBox] {
            let (_tmp, paths) = super::super::test_paths();
            let mut session = prepare_tun_session(&paths, backend, "tun0").unwrap();
            let id = session.ownership.as_ref().unwrap().session_id.to_string();
            assert_eq!(
                renew_completed_tun_session(&paths.tun_session_path(), &id).unwrap(),
                id
            );
            let ownership = session.ownership.as_mut().unwrap();
            ownership.device_ifindex = Some(4);
            ownership.reservation_ifindex = Some(4);
            ownership.phase = TunPhase::Complete;
            save_tun_session(&paths, &session).unwrap();
            let next = renew_completed_tun_session(&paths.tun_session_path(), &id).unwrap();
            assert_ne!(next, id);
            assert!(renew_completed_tun_session(&paths.tun_session_path(), &id).is_err());
            let renewed = load_tun_session(&paths).unwrap().ownership.unwrap();
            assert_eq!(renewed.phase, TunPhase::Prepared);
            assert!(renewed.device_ifindex.is_none());
            assert!(renewed.reservation_ifindex.is_none());
        }
    }

    #[test]
    fn reserved_interface_cannot_create_a_recovery_journal() {
        let (_tmp, paths) = super::super::test_paths();
        assert!(
            prepare_tun_session(
                &paths,
                BackendType::Xray,
                v2ray_rs_tun_session::RESERVATION_NAME
            )
            .is_err()
        );
        assert!(!paths.tun_session_path().exists());
    }

    #[test]
    fn replaced_session_is_detected_only_for_the_retired_id() {
        let (_tmp, paths) = super::super::test_paths();
        let first = prepare_tun_session(&paths, BackendType::SingBox, "tun0").unwrap();
        let first_id = first.ownership.unwrap().session_id.to_string();
        assert!(!tun_session_replaced(&paths.tun_session_path(), &first_id));

        // The unstarted sing-box record is retirable, so a fresh prepare
        // replaces the file with a new session id.
        prepare_tun_session(&paths, BackendType::SingBox, "tun0").unwrap();
        assert!(tun_session_replaced(&paths.tun_session_path(), &first_id));
        assert!(!tun_session_replaced(
            &paths.tun_session_path().with_extension("missing"),
            &first_id
        ));
    }
}
