use std::fs as std_fs;

use chrono::Utc;
use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};

use crate::fs::atomic_write;
use crate::persistence::{AppPaths, PersistenceError};
use crate::profile::AppProfile;

struct LockGuard {
    _flock: Flock<std_fs::File>,
}

impl LockGuard {
    fn acquire(file: std_fs::File) -> Result<Self, InstanceError> {
        let lock_arg = FlockArg::LockExclusiveNonblock;

        match Flock::lock(file, lock_arg) {
            Ok(flock) => Ok(Self { _flock: flock }),
            Err(_) => Err(InstanceError::LockHeld {
                pid: std::process::id(),
                profile: "unknown".to_string(),
            }),
        }
    }
}

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceStamp {
    pub profile: String,
    pub app_id: String,
    pub build_version: String,
    pub schema_version: u32,
    pub first_started_at: String,
    pub last_started_at: String,
    pub last_started_pid: u32,
}

impl InstanceStamp {
    pub fn new(profile: &AppProfile, app_id: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            profile: profile.qualifier().to_string(),
            app_id: app_id.to_string(),
            build_version: BUILD_VERSION.to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            first_started_at: now.clone(),
            last_started_at: now,
            last_started_pid: std::process::id(),
        }
    }

    pub fn load_or_create(paths: &AppPaths) -> Result<Self, InstanceError> {
        let path = paths.instance_stamp_path();

        if path.exists() {
            let content = std_fs::read_to_string(&path)?;
            let stamp: InstanceStamp = serde_json::from_str(&content)?;
            Ok(stamp)
        } else {
            let stamp = Self::new(paths.profile(), &paths.profile().app_id());
            stamp.save_to(&path)?;
            Ok(stamp)
        }
    }

    pub fn update_started(&mut self, paths: &AppPaths) -> Result<(), InstanceError> {
        let now = Utc::now().to_rfc3339();
        self.build_version = BUILD_VERSION.to_string();
        self.last_started_at = now;
        self.last_started_pid = std::process::id();
        self.save_to(&paths.instance_stamp_path())
    }

    fn save_to(&self, path: &std::path::Path) -> Result<(), InstanceError> {
        let data = serde_json::to_vec_pretty(self)?;
        atomic_write(path, &data)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CompatibilityResult {
    Match,
    NeedsForwardMigration,
    IncompatibleProfile,
    IncompatibleAppId,
    TooNew,
}

pub fn check_compatibility(
    stamp: &InstanceStamp,
    current_profile: &AppProfile,
) -> CompatibilityResult {
    let profile_match = stamp.profile == current_profile.qualifier();
    let app_id_match = stamp.app_id == current_profile.app_id();
    let schema_cmp = stamp.schema_version.cmp(&CURRENT_SCHEMA_VERSION);

    if !profile_match {
        return CompatibilityResult::IncompatibleProfile;
    }
    if !app_id_match {
        return CompatibilityResult::IncompatibleAppId;
    }

    match schema_cmp {
        std::cmp::Ordering::Equal => CompatibilityResult::Match,
        std::cmp::Ordering::Less => CompatibilityResult::NeedsForwardMigration,
        std::cmp::Ordering::Greater => CompatibilityResult::TooNew,
    }
}

pub fn reset_instance(
    paths: &AppPaths,
    profile: &AppProfile,
    confirm: bool,
) -> Result<(), InstanceError> {
    if *profile == AppProfile::Production && !confirm {
        return Err(InstanceError::ResetProductionDenied);
    }

    remove_dir_contents(paths.config_dir())?;
    remove_dir_contents(paths.data_dir())?;
    remove_dir_contents(paths.cache_dir())?;
    remove_dir_contents(paths.runtime_dir())?;
    remove_dir_contents(paths.state_dir())?;

    paths.ensure_dirs()?;

    Ok(())
}

fn remove_dir_contents(dir: &std::path::Path) -> Result<(), InstanceError> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std_fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std_fs::remove_dir_all(&path)?;
        } else {
            std_fs::remove_file(&path)?;
        }
    }

    Ok(())
}

pub struct InstanceLock {
    _guard: LockGuard,
}

impl InstanceLock {
    pub fn acquire(paths: &AppPaths) -> Result<Self, InstanceError> {
        let path = paths.instance_lock_path();

        if let Some(parent) = path.parent() {
            std_fs::create_dir_all(parent)?;
        }

        let file = std_fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)?;

        let guard = LockGuard::acquire(file).map_err(|e| {
            if let InstanceError::LockHeld { .. } = e {
                let holder_pid = read_holder_pid(paths).unwrap_or(std::process::id());
                InstanceError::LockHeld {
                    pid: holder_pid,
                    profile: paths.profile().qualifier().to_string(),
                }
            } else {
                e
            }
        })?;

        Ok(Self { _guard: guard })
    }
}

fn read_holder_pid(paths: &AppPaths) -> Result<u32, InstanceError> {
    if let Ok(stamp) = InstanceStamp::load_or_create(paths) {
        Ok(stamp.last_started_pid)
    } else {
        Ok(std::process::id())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("persistence: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("another instance is running (PID {pid}), profile '{profile}' is locked")]
    LockHeld { pid: u32, profile: String },
    #[error("refusing to reset production profile without --i-understand")]
    ResetProductionDenied,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_paths() -> (TempDir, AppPaths) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        (tmp, paths)
    }

    #[test]
    fn test_instance_stamp_new() {
        let stamp = InstanceStamp::new(&AppProfile::Test, "test.app");
        assert_eq!(stamp.profile, "v2ray-rs-test");
        assert_eq!(stamp.app_id, "test.app");
        assert_eq!(stamp.build_version, BUILD_VERSION);
        assert_eq!(stamp.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(stamp.last_started_pid, std::process::id());
        assert!(!stamp.first_started_at.is_empty());
        assert!(!stamp.last_started_at.is_empty());
    }

    #[test]
    fn test_instance_stamp_load_or_create_new() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let stamp = InstanceStamp::load_or_create(&paths).unwrap();

        assert_eq!(stamp.profile, "v2ray-rs-test");
        assert_eq!(stamp.app_id, "com.github.v2ray-rs.test");
        assert_eq!(stamp.build_version, BUILD_VERSION);
        assert_eq!(stamp.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(stamp.last_started_pid, std::process::id());

        assert!(paths.instance_stamp_path().exists());
    }

    #[test]
    fn test_instance_stamp_load_existing() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let stamp1 = InstanceStamp::load_or_create(&paths).unwrap();
        let first_started = stamp1.first_started_at.clone();
        let first_last_started = stamp1.last_started_at.clone();

        std::thread::sleep(std::time::Duration::from_millis(10));

        let stamp2 = InstanceStamp::load_or_create(&paths).unwrap();

        assert_eq!(stamp2.profile, stamp1.profile);
        assert_eq!(stamp2.app_id, stamp1.app_id);
        assert_eq!(stamp2.build_version, stamp1.build_version);
        assert_eq!(stamp2.schema_version, stamp1.schema_version);
        assert_eq!(stamp2.first_started_at, first_started);
        assert_eq!(stamp2.last_started_at, first_last_started);
    }

    #[test]
    fn test_instance_stamp_update_started_refreshes_build_version() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let mut stamp = InstanceStamp::load_or_create(&paths).unwrap();
        stamp.build_version = "0.7.4".to_string();
        stamp.save_to(&paths.instance_stamp_path()).unwrap();

        stamp.update_started(&paths).unwrap();

        assert_eq!(stamp.build_version, BUILD_VERSION);
        let reloaded = InstanceStamp::load_or_create(&paths).unwrap();
        assert_eq!(reloaded.build_version, BUILD_VERSION);
    }

    #[test]
    fn test_instance_stamp_update_started() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let mut stamp = InstanceStamp::load_or_create(&paths).unwrap();
        let original_last_started = stamp.last_started_at.clone();

        std::thread::sleep(std::time::Duration::from_millis(10));

        stamp.update_started(&paths).unwrap();

        assert_ne!(stamp.last_started_at, original_last_started);
        assert_eq!(stamp.last_started_pid, std::process::id());

        let reloaded = InstanceStamp::load_or_create(&paths).unwrap();
        assert_eq!(reloaded.last_started_at, stamp.last_started_at);
    }

    #[test]
    fn test_check_compatibility_match() {
        let mut stamp = InstanceStamp::new(&AppProfile::Test, "com.github.v2ray-rs.test");
        stamp.schema_version = CURRENT_SCHEMA_VERSION;

        let result = check_compatibility(&stamp, &AppProfile::Test);
        assert_eq!(result, CompatibilityResult::Match);
    }

    #[test]
    fn test_check_compatibility_needs_forward_migration() {
        let mut stamp = InstanceStamp::new(&AppProfile::Test, "com.github.v2ray-rs.test");
        stamp.schema_version = CURRENT_SCHEMA_VERSION - 1;

        let result = check_compatibility(&stamp, &AppProfile::Test);
        assert_eq!(result, CompatibilityResult::NeedsForwardMigration);
    }

    #[test]
    fn test_check_compatibility_too_new() {
        let mut stamp = InstanceStamp::new(&AppProfile::Test, "com.github.v2ray-rs.test");
        stamp.schema_version = CURRENT_SCHEMA_VERSION + 1;

        let result = check_compatibility(&stamp, &AppProfile::Test);
        assert_eq!(result, CompatibilityResult::TooNew);
    }

    #[test]
    fn test_check_compatibility_incompatible_profile() {
        let stamp = InstanceStamp::new(&AppProfile::Test, "com.github.v2ray-rs.test");

        let result = check_compatibility(&stamp, &AppProfile::Production);
        assert_eq!(result, CompatibilityResult::IncompatibleProfile);
    }

    #[test]
    fn test_check_compatibility_incompatible_app_id() {
        let stamp = InstanceStamp::new(&AppProfile::Test, "different.app");

        let result = check_compatibility(&stamp, &AppProfile::Test);
        assert_eq!(result, CompatibilityResult::IncompatibleAppId);
    }

    #[test]
    fn test_reset_instance_test_profile() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let test_file = paths.data_dir().join("test.txt");
        std_fs::write(&test_file, "test").unwrap();

        assert!(test_file.exists());

        reset_instance(&paths, &AppProfile::Test, false).unwrap();

        assert!(!test_file.exists());
        assert!(paths.data_dir().exists());
    }

    #[test]
    fn test_reset_instance_production_without_confirm() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let result = reset_instance(&paths, &AppProfile::Production, false);
        assert!(matches!(result, Err(InstanceError::ResetProductionDenied)));
    }

    #[test]
    fn test_reset_instance_production_with_confirm() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let test_file = paths.data_dir().join("test.txt");
        std_fs::write(&test_file, "test").unwrap();

        assert!(test_file.exists());

        reset_instance(&paths, &AppProfile::Production, true).unwrap();

        assert!(!test_file.exists());
        assert!(paths.data_dir().exists());
    }

    #[test]
    fn test_reset_instance_recreates_directories() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        reset_instance(&paths, &AppProfile::Test, false).unwrap();

        assert!(paths.config_dir().exists());
        assert!(paths.data_dir().exists());
        assert!(paths.cache_dir().exists());
        assert!(paths.runtime_dir().exists());
        assert!(paths.state_dir().exists());
    }

    #[test]
    fn test_instance_lock_acquire() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let lock = InstanceLock::acquire(&paths).unwrap();

        assert!(paths.instance_lock_path().exists());

        drop(lock);
    }

    #[test]
    fn test_instance_lock_already_held() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        let _lock1 = InstanceLock::acquire(&paths).unwrap();

        let result = InstanceLock::acquire(&paths);
        assert!(matches!(result, Err(InstanceError::LockHeld { .. })));
    }

    #[test]
    fn test_instance_lock_released_on_drop() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();

        {
            let _lock = InstanceLock::acquire(&paths).unwrap();
        }

        let lock2 = InstanceLock::acquire(&paths);
        assert!(lock2.is_ok());
    }
}
