use std::env;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};
use thiserror::Error;

use crate::cli::PathOverrides;
use crate::profile::AppProfile;

mod latency;
mod manual_nodes;
mod presets;
mod routing;
mod settings;
mod subscriptions;
mod tun_session;

pub use latency::{load_latency_snapshot, save_latency_snapshot};
pub use manual_nodes::{
    add_manual_node, get_manual_node, load_manual_nodes, load_manual_nodes_or_default,
    remove_manual_node, save_manual_nodes, update_manual_node,
};
pub use presets::{delete_preset, load_custom_presets, save_preset};
pub use routing::{load_routing_rules, save_routing_rules};
pub use settings::{load_settings, load_settings_or_default, save_settings};
pub use subscriptions::{
    add_subscription, get_subscription, load_subscriptions, remove_subscription,
    save_subscriptions, update_subscription,
};
pub use tun_session::{TunSession, clear_tun_session, load_tun_session, save_tun_session};

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("failed to determine XDG directories")]
    NoDirs,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt config file, using defaults: {0}")]
    CorruptConfig(String),
    #[error("invalid override path for {field}: {path} — must be an absolute path")]
    InvalidOverridePath { field: String, path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    runtime_dir: PathBuf,
    state_dir: PathBuf,
    profile: AppProfile,
}

impl AppPaths {
    pub fn new() -> Result<Self, PersistenceError> {
        Self::for_profile(AppProfile::Production)
    }

    pub fn new_dev() -> Result<Self, PersistenceError> {
        Self::for_profile(AppProfile::Development)
    }

    pub fn for_profile(profile: AppProfile) -> Result<Self, PersistenceError> {
        let qualifier = profile.qualifier();
        let dirs = ProjectDirs::from("com", "v2ray-rs", qualifier.as_str())
            .ok_or(PersistenceError::NoDirs)?;

        let base_dirs = BaseDirs::new().ok_or(PersistenceError::NoDirs)?;

        // Primary: XDG_RUNTIME_DIR/<qualifier>/
        // Fallback: data_dir/runtime (when XDG_RUNTIME_DIR is not set)
        let runtime_dir = if env::var("XDG_RUNTIME_DIR").is_ok() {
            base_dirs
                .runtime_dir()
                .map(|r| r.join(qualifier.as_str()))
                .unwrap_or_else(|| dirs.data_dir().join("runtime"))
        } else {
            dirs.data_dir().join("runtime")
        };

        // Primary: XDG_STATE_HOME/<qualifier>/
        // Fallback: data_dir/state (when XDG_STATE_HOME is not set)
        let state_dir = if env::var("XDG_STATE_HOME").is_ok() {
            base_dirs
                .state_dir()
                .map(|s| s.join(qualifier.as_str()))
                .unwrap_or_else(|| dirs.data_dir().join("state"))
        } else {
            dirs.data_dir().join("state")
        };

        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
            runtime_dir,
            state_dir,
            profile,
        })
    }

    pub fn for_profile_in(profile: AppProfile, root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            runtime_dir: root.join("runtime"),
            state_dir: root.join("state"),
            profile,
        }
    }

    #[deprecated(
        since = "0.6.0",
        note = "Use for_profile_in(AppProfile::Test, root) instead"
    )]
    pub fn from_paths(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        // For backward compatibility, cache/runtime/state are derived from data_dir
        Self {
            data_dir: data_dir.clone(),
            config_dir,
            cache_dir: data_dir.join("cache"),
            runtime_dir: data_dir.join("runtime"),
            state_dir: data_dir.join("state"),
            profile: AppProfile::Test,
        }
    }

    pub fn with_overrides(
        profile: AppProfile,
        overrides: &PathOverrides,
    ) -> Result<Self, PersistenceError> {
        let mut paths = Self::for_profile(profile)?;

        if let Some(ref config_dir) = overrides.config_dir {
            let expanded = expand_home(config_dir)?;
            validate_absolute_path("config_dir", &expanded)?;
            paths.config_dir = expanded;
        }

        if let Some(ref data_dir) = overrides.data_dir {
            let expanded = expand_home(data_dir)?;
            validate_absolute_path("data_dir", &expanded)?;
            paths.data_dir = expanded;
        }

        if let Some(ref cache_dir) = overrides.cache_dir {
            let expanded = expand_home(cache_dir)?;
            validate_absolute_path("cache_dir", &expanded)?;
            paths.cache_dir = expanded;
        }

        if let Some(ref runtime_dir) = overrides.runtime_dir {
            let expanded = expand_home(runtime_dir)?;
            validate_absolute_path("runtime_dir", &expanded)?;
            paths.runtime_dir = expanded;
        }

        if let Some(ref state_dir) = overrides.state_dir {
            let expanded = expand_home(state_dir)?;
            validate_absolute_path("state_dir", &expanded)?;
            paths.state_dir = expanded;
        }

        Ok(paths)
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn profile(&self) -> &AppProfile {
        &self.profile
    }

    pub fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn subscriptions_path(&self) -> PathBuf {
        self.data_dir.join("subscriptions.json")
    }

    pub fn routing_rules_path(&self) -> PathBuf {
        self.data_dir.join("routing_rules.json")
    }

    pub fn geodata_dir(&self) -> PathBuf {
        self.cache_dir.join("geodata")
    }

    pub fn geodata_index_dir(&self) -> PathBuf {
        self.cache_dir.join("geodata-index")
    }

    pub fn latency_snapshot_path(&self) -> PathBuf {
        self.state_dir.join("latency_snapshot.json")
    }

    pub fn pid_file_path(&self) -> PathBuf {
        self.runtime_dir.join("backend.pid")
    }

    pub fn generated_dir(&self) -> PathBuf {
        self.runtime_dir.join("generated")
    }

    pub fn instance_stamp_path(&self) -> PathBuf {
        self.state_dir.join("instance.json")
    }

    pub fn tun_session_path(&self) -> PathBuf {
        self.state_dir.join("tun_session.json")
    }

    pub fn instance_lock_path(&self) -> PathBuf {
        self.runtime_dir.join("v2ray-rs.lock")
    }

    pub fn presets_dir(&self) -> PathBuf {
        self.data_dir.join("presets")
    }

    pub fn custom_nodes_path(&self) -> PathBuf {
        self.data_dir.join("custom_nodes.json")
    }

    pub fn ensure_dirs(&self) -> Result<(), PersistenceError> {
        create_dir_with_permissions(&self.config_dir)?;
        create_dir_with_permissions(&self.data_dir)?;
        create_dir_with_permissions(&self.cache_dir)?;
        create_dir_with_permissions(&self.runtime_dir)?;
        create_dir_with_permissions(&self.state_dir)?;
        self.relocate_legacy_files();
        Ok(())
    }

    fn relocate_legacy_files(&self) {
        self.relocate_pid_file();
        self.relocate_generated_dir();
        self.relocate_geodata_dir();
        self.relocate_latency_snapshot();
    }

    fn relocate_pid_file(&self) {
        let legacy_path = self.data_dir.join("backend.pid");
        let new_path = self.pid_file_path();

        if !legacy_path.exists() || new_path.exists() {
            return;
        }

        match move_file_best_effort(&legacy_path, &new_path) {
            Ok(()) => log::info!("relocated PID file: {:?} -> {:?}", legacy_path, new_path),
            Err(e) => log::warn!(
                "failed to relocate PID file {:?} to {:?}: {}",
                legacy_path,
                new_path,
                e
            ),
        }
    }

    fn relocate_generated_dir(&self) {
        let legacy_dir = self.data_dir.join("generated");
        let new_dir = self.generated_dir();

        if !legacy_dir.exists() || !is_dir_empty(&new_dir) {
            return;
        }

        match fs::read_dir(&legacy_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let src = entry.path();
                    if src.is_file() {
                        let dst = new_dir.join(entry.file_name());
                        match move_file_best_effort(&src, &dst) {
                            Ok(()) => {
                                log::info!("relocated generated config: {:?} -> {:?}", src, dst)
                            }
                            Err(e) => {
                                log::warn!("failed to relocate {:?} to {:?}: {}", src, dst, e)
                            }
                        }
                    }
                }
                let _ = fs::remove_dir(&legacy_dir);
            }
            Err(e) => log::warn!(
                "failed to read legacy generated dir {:?}: {}",
                legacy_dir,
                e
            ),
        }
    }

    fn relocate_geodata_dir(&self) {
        let legacy_dir = self.data_dir.join("geodata");
        let new_dir = self.geodata_dir();

        if !legacy_dir.exists() || !is_dir_empty(&new_dir) {
            return;
        }

        match fs::read_dir(&legacy_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let src = entry.path();
                    if src.is_file() {
                        let dst = new_dir.join(entry.file_name());
                        match move_file_best_effort(&src, &dst) {
                            Ok(()) => log::info!("relocated geodata file: {:?} -> {:?}", src, dst),
                            Err(e) => {
                                log::warn!("failed to relocate {:?} to {:?}: {}", src, dst, e)
                            }
                        }
                    }
                }
                let _ = fs::remove_dir(&legacy_dir);
            }
            Err(e) => log::warn!("failed to read legacy geodata dir {:?}: {}", legacy_dir, e),
        }
    }

    fn relocate_latency_snapshot(&self) {
        let legacy_path = self.data_dir.join("latency_snapshot.json");
        let new_path = self.latency_snapshot_path();

        if !legacy_path.exists() || new_path.exists() {
            return;
        }

        match move_file_best_effort(&legacy_path, &new_path) {
            Ok(()) => log::info!(
                "relocated latency snapshot: {:?} -> {:?}",
                legacy_path,
                new_path
            ),
            Err(e) => log::warn!(
                "failed to relocate latency snapshot {:?} to {:?}: {}",
                legacy_path,
                new_path,
                e
            ),
        }
    }
}

fn expand_home(path: &Path) -> Result<PathBuf, PersistenceError> {
    let path_str = path.to_str().ok_or_else(|| {
        PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path contains invalid UTF-8",
        ))
    })?;

    if let Some(rest) = path_str.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/'))
    {
        let home = directories::BaseDirs::new()
            .ok_or(PersistenceError::NoDirs)?
            .home_dir()
            .to_path_buf();

        return Ok(if rest.is_empty() {
            home
        } else {
            home.join(&rest[1..])
        });
    }

    Ok(path.to_path_buf())
}

fn validate_absolute_path(field: &str, path: &Path) -> Result<(), PersistenceError> {
    if !path.is_absolute() {
        return Err(PersistenceError::InvalidOverridePath {
            field: field.to_string(),
            path: path.to_path_buf(),
        });
    }

    Ok(())
}

pub(crate) fn create_dir_with_permissions(path: &Path) -> Result<(), PersistenceError> {
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(path)?;
    Ok(())
}

fn is_dir_empty(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

fn move_file_best_effort(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if let Err(e) = fs::rename(src, dst) {
        if e.kind() == std::io::ErrorKind::CrossesDevices {
            fs::copy(src, dst)?;
            fs::remove_file(src)?;
        } else {
            return Err(e);
        }
    }
    Ok(())
}

pub(super) fn read_file(path: &Path) -> Result<String, PersistenceError> {
    fs::read_to_string(path).map_err(|e| {
        PersistenceError::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })
}

pub(super) fn json_uuid(value: &serde_json::Value) -> Option<uuid::Uuid> {
    value
        .as_str()
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefMigration {
    Unchanged,
    Updated,
    DropParent,
}

#[cfg(test)]
pub(super) fn test_paths() -> (tempfile::TempDir, AppPaths) {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    (tmp, paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn test_ensure_dirs_creates_directories() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir().exists());
        assert!(paths.data_dir().exists());
        assert!(paths.cache_dir().exists());
        assert!(paths.runtime_dir().exists());
        assert!(paths.state_dir().exists());

        let config_perms = fs::metadata(paths.config_dir()).unwrap().permissions();
        assert_eq!(config_perms.mode() & 0o777, 0o700);

        let data_perms = fs::metadata(paths.data_dir()).unwrap().permissions();
        assert_eq!(data_perms.mode() & 0o777, 0o700);

        let cache_perms = fs::metadata(paths.cache_dir()).unwrap().permissions();
        assert_eq!(cache_perms.mode() & 0o777, 0o700);

        let runtime_perms = fs::metadata(paths.runtime_dir()).unwrap().permissions();
        assert_eq!(runtime_perms.mode() & 0o777, 0o700);

        let state_perms = fs::metadata(paths.state_dir()).unwrap().permissions();
        assert_eq!(state_perms.mode() & 0o777, 0o700);
    }

    #[test]
    fn test_for_profile_in_creates_structure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let paths = AppPaths::for_profile_in(AppProfile::Test, root);

        assert_eq!(paths.config_dir(), root.join("config"));
        assert_eq!(paths.data_dir(), root.join("data"));
        assert_eq!(paths.cache_dir(), root.join("cache"));
        assert_eq!(paths.runtime_dir(), root.join("runtime"));
        assert_eq!(paths.state_dir(), root.join("state"));
        assert_eq!(paths.profile(), &AppProfile::Test);
    }

    #[test]
    fn test_new_accessors() {
        let (_tmp, paths) = test_paths();

        assert_eq!(
            paths.pid_file_path(),
            paths.runtime_dir().join("backend.pid")
        );

        assert_eq!(paths.generated_dir(), paths.runtime_dir().join("generated"));

        assert_eq!(paths.geodata_dir(), paths.cache_dir().join("geodata"));

        assert_eq!(
            paths.geodata_index_dir(),
            paths.cache_dir().join("geodata-index")
        );

        assert_eq!(
            paths.latency_snapshot_path(),
            paths.state_dir().join("latency_snapshot.json")
        );

        assert_eq!(
            paths.instance_stamp_path(),
            paths.state_dir().join("instance.json")
        );

        assert_eq!(
            paths.instance_lock_path(),
            paths.runtime_dir().join("v2ray-rs.lock")
        );
    }

    #[test]
    fn test_runtime_dir_fallback() {
        // Temporarily unset XDG_RUNTIME_DIR
        let original_runtime = env::var("XDG_RUNTIME_DIR").ok();

        unsafe { env::remove_var("XDG_RUNTIME_DIR") };

        let result = AppPaths::new();
        assert!(
            result.is_ok(),
            "should create AppPaths even without XDG_RUNTIME_DIR"
        );

        if let Ok(paths) = result {
            // Runtime dir should fall back to data_dir/runtime
            assert!(paths.runtime_dir().ends_with("runtime"));
        }

        // Restore original value
        if let Some(val) = original_runtime {
            unsafe { env::set_var("XDG_RUNTIME_DIR", val) };
        }
    }

    #[test]
    fn test_state_dir_fallback() {
        // Temporarily unset XDG_STATE_HOME
        let original_state = env::var("XDG_STATE_HOME").ok();

        unsafe { env::remove_var("XDG_STATE_HOME") };

        let result = AppPaths::new();
        assert!(
            result.is_ok(),
            "should create AppPaths even without XDG_STATE_HOME"
        );

        if let Ok(paths) = result {
            // State dir should fall back to data_dir/state
            assert!(paths.state_dir().ends_with("state"));
        }

        // Restore original value
        if let Some(val) = original_state {
            unsafe { env::set_var("XDG_STATE_HOME", val) };
        }
    }

    #[test]
    fn test_geodata_dir_uses_cache_dir() {
        let (_tmp, paths) = test_paths();

        let expected = paths.cache_dir().join("geodata");
        assert_eq!(paths.geodata_dir(), expected);
        assert!(!expected.starts_with(paths.data_dir()));
    }

    #[test]
    fn test_latency_snapshot_path_uses_state_dir() {
        let (_tmp, paths) = test_paths();

        let expected = paths.state_dir().join("latency_snapshot.json");
        assert_eq!(paths.latency_snapshot_path(), expected);
        assert!(!expected.starts_with(paths.data_dir()));
    }

    #[test]
    fn test_new_and_new_dev_delegation() {
        let prod = AppPaths::new();
        assert!(prod.is_ok());
        assert_eq!(prod.unwrap().profile(), &AppProfile::Production);

        let dev = AppPaths::new_dev();
        assert!(dev.is_ok());
        assert_eq!(dev.unwrap().profile(), &AppProfile::Development);
    }

    #[test]
    fn test_with_overrides_applies_all() {
        let overrides = PathOverrides {
            config_dir: Some(PathBuf::from("/custom/config")),
            data_dir: Some(PathBuf::from("/custom/data")),
            cache_dir: Some(PathBuf::from("/custom/cache")),
            runtime_dir: Some(PathBuf::from("/custom/runtime")),
            state_dir: Some(PathBuf::from("/custom/state")),
            install_icons: Some(true),
        };

        let paths = AppPaths::with_overrides(AppProfile::Test, &overrides).unwrap();

        assert_eq!(paths.config_dir(), PathBuf::from("/custom/config"));
        assert_eq!(paths.data_dir(), PathBuf::from("/custom/data"));
        assert_eq!(paths.cache_dir(), PathBuf::from("/custom/cache"));
        assert_eq!(paths.runtime_dir(), PathBuf::from("/custom/runtime"));
        assert_eq!(paths.state_dir(), PathBuf::from("/custom/state"));
        assert_eq!(paths.profile(), &AppProfile::Test);
    }

    #[test]
    fn test_with_overrides_partial() {
        let overrides = PathOverrides {
            config_dir: Some(PathBuf::from("/custom/config")),
            data_dir: None,
            cache_dir: None,
            runtime_dir: None,
            state_dir: None,
            install_icons: None,
        };

        let base_paths = AppPaths::for_profile(AppProfile::Test).unwrap();
        let paths = AppPaths::with_overrides(AppProfile::Test, &overrides).unwrap();

        assert_eq!(paths.config_dir(), PathBuf::from("/custom/config"));
        assert_eq!(paths.data_dir(), base_paths.data_dir());
        assert_eq!(paths.cache_dir(), base_paths.cache_dir());
        assert_eq!(paths.runtime_dir(), base_paths.runtime_dir());
        assert_eq!(paths.state_dir(), base_paths.state_dir());
    }

    #[test]
    fn test_with_overrides_invalid_relative_path() {
        let overrides = PathOverrides {
            config_dir: Some(PathBuf::from("relative/path")),
            data_dir: None,
            cache_dir: None,
            runtime_dir: None,
            state_dir: None,
            install_icons: None,
        };

        let result = AppPaths::with_overrides(AppProfile::Test, &overrides);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, PersistenceError::InvalidOverridePath { .. }));
    }

    #[test]
    fn test_with_overrides_no_overrides() {
        let overrides = PathOverrides::default();

        let base_paths = AppPaths::for_profile(AppProfile::Test).unwrap();
        let paths = AppPaths::with_overrides(AppProfile::Test, &overrides).unwrap();

        assert_eq!(paths.config_dir(), base_paths.config_dir());
        assert_eq!(paths.data_dir(), base_paths.data_dir());
        assert_eq!(paths.cache_dir(), base_paths.cache_dir());
        assert_eq!(paths.runtime_dir(), base_paths.runtime_dir());
        assert_eq!(paths.state_dir(), base_paths.state_dir());
    }

    #[test]
    fn test_expand_home_with_tilde() {
        let home = directories::BaseDirs::new()
            .expect("failed to get home dir")
            .home_dir()
            .to_path_buf();

        let expanded = expand_home(&PathBuf::from("~")).unwrap();
        assert_eq!(expanded, home);

        let expanded = expand_home(&PathBuf::from("~/Documents")).unwrap();
        assert_eq!(expanded, home.join("Documents"));

        let expanded = expand_home(&PathBuf::from("~/some/nested/path")).unwrap();
        assert_eq!(expanded, home.join("some/nested/path"));
    }

    #[test]
    fn test_expand_home_without_tilde() {
        let path = PathBuf::from("/absolute/path");
        let expanded = expand_home(&path).unwrap();
        assert_eq!(expanded, path);

        let path = PathBuf::from("/another/absolute");
        let expanded = expand_home(&path).unwrap();
        assert_eq!(expanded, path);
    }

    #[test]
    fn test_expand_home_tilde_in_middle() {
        let path = PathBuf::from("/path/to/~something");
        let expanded = expand_home(&path).unwrap();
        assert_eq!(expanded, path);
    }

    #[test]
    fn test_validate_absolute_path_valid() {
        let result = validate_absolute_path("config_dir", Path::new("/absolute/path"));
        assert!(result.is_ok());

        let result = validate_absolute_path("data_dir", Path::new("/another/path"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_absolute_path_invalid() {
        let result = validate_absolute_path("config_dir", Path::new("relative/path"));
        assert!(result.is_err());

        let result = validate_absolute_path("data_dir", Path::new("./another/relative"));
        assert!(result.is_err());

        let result = validate_absolute_path("cache_dir", Path::new("../parent"));
        assert!(result.is_err());
    }

    #[test]
    fn production_development_test_have_disjoint_qualifiers() {
        let prod = AppProfile::Production;
        let dev = AppProfile::Development;
        let test = AppProfile::Test;
        assert_ne!(prod.qualifier(), dev.qualifier());
        assert_ne!(prod.qualifier(), test.qualifier());
        assert_ne!(dev.qualifier(), test.qualifier());
        assert_ne!(prod.app_id(), dev.app_id());
        assert_ne!(prod.app_id(), test.app_id());
        assert_ne!(dev.app_id(), test.app_id());
    }

    #[test]
    fn for_profile_in_creates_five_distinct_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir().exists());
        assert!(paths.data_dir().exists());
        assert!(paths.cache_dir().exists());
        assert!(paths.runtime_dir().exists());
        assert!(paths.state_dir().exists());

        let dirs = [
            paths.config_dir(),
            paths.data_dir(),
            paths.cache_dir(),
            paths.runtime_dir(),
            paths.state_dir(),
        ];
        for i in 0..dirs.len() {
            for j in (i + 1)..dirs.len() {
                assert_ne!(
                    dirs[i],
                    dirs[j],
                    "{} and {} should be distinct",
                    dirs[i].display(),
                    dirs[j].display()
                );
            }
        }
    }

    #[test]
    fn test_profile_isolation_for_profile_in() {
        // Test that for_profile_in() allows different profiles with the same root
        // Note: for_profile_in() doesn't embed the profile in the path structure,
        // but stores it for tracking. Different profiles with same root point to same directories.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let prod_paths = AppPaths::for_profile_in(AppProfile::Production, root);
        let dev_paths = AppPaths::for_profile_in(AppProfile::Development, root);
        let test_paths = AppPaths::for_profile_in(AppProfile::Test, root);

        // Each instance tracks a different profile
        assert_eq!(prod_paths.profile(), &AppProfile::Production);
        assert_eq!(dev_paths.profile(), &AppProfile::Development);
        assert_eq!(test_paths.profile(), &AppProfile::Test);

        // Profiles are distinct
        assert_ne!(
            prod_paths.profile(),
            dev_paths.profile(),
            "Production and Development profiles should be different"
        );
        assert_ne!(
            prod_paths.profile(),
            test_paths.profile(),
            "Production and Test profiles should be different"
        );
        assert_ne!(
            dev_paths.profile(),
            test_paths.profile(),
            "Development and Test profiles should be different"
        );

        // Verify that with different roots, each profile can have isolated directories
        let prod_root = root.join("prod");
        let dev_root = root.join("dev");
        let test_root = root.join("test");

        let prod_paths_isolated = AppPaths::for_profile_in(AppProfile::Production, &prod_root);
        let dev_paths_isolated = AppPaths::for_profile_in(AppProfile::Development, &dev_root);
        let test_paths_isolated = AppPaths::for_profile_in(AppProfile::Test, &test_root);

        // Each should have its own distinct config directory
        assert_eq!(prod_paths_isolated.config_dir(), prod_root.join("config"));
        assert_eq!(dev_paths_isolated.config_dir(), dev_root.join("config"));
        assert_eq!(test_paths_isolated.config_dir(), test_root.join("config"));

        assert_ne!(
            prod_paths_isolated.config_dir(),
            dev_paths_isolated.config_dir(),
            "Production and Dev config dirs should be different"
        );
        assert_ne!(
            prod_paths_isolated.config_dir(),
            test_paths_isolated.config_dir(),
            "Production and Test config dirs should be different"
        );
        assert_ne!(
            dev_paths_isolated.config_dir(),
            test_paths_isolated.config_dir(),
            "Dev and Test config dirs should be different"
        );
    }

    #[test]
    fn test_profile_isolation_verification() {
        // This test demonstrates that each profile uses completely disjoint paths
        // by showing the actual paths each scenario would use.

        println!("\n=== Profile Isolation Verification ===\n");

        // Scenario 1: cargo run (debug build, no args) → Development profile
        println!("Scenario 1: cargo run (debug build, no args)");
        println!("  Profile: Development");
        let dev_paths = AppPaths::for_profile(AppProfile::Development).unwrap();
        println!("  qualifier: {}", dev_paths.profile().qualifier());
        println!("  config_dir: {}", dev_paths.config_dir().display());
        println!("  data_dir:   {}", dev_paths.data_dir().display());
        println!("  cache_dir:  {}", dev_paths.cache_dir().display());
        println!("  runtime_dir: {}", dev_paths.runtime_dir().display());
        println!("  state_dir:  {}", dev_paths.state_dir().display());
        println!(
            "  instance_stamp: {}",
            dev_paths.instance_stamp_path().display()
        );
        println!(
            "  lock_path:   {}",
            dev_paths.instance_lock_path().display()
        );
        println!();

        // Scenario 2: cargo test → Test profile (isolated temp dir)
        println!("Scenario 2: cargo test");
        println!("  Profile: Test");
        let (_tmp, test_paths) = test_paths();
        println!("  qualifier: {}", test_paths.profile().qualifier());
        println!("  config_dir: {}", test_paths.config_dir().display());
        println!("  data_dir:   {}", test_paths.data_dir().display());
        println!("  cache_dir:  {}", test_paths.cache_dir().display());
        println!("  runtime_dir: {}", test_paths.runtime_dir().display());
        println!("  state_dir:  {}", test_paths.state_dir().display());
        println!(
            "  instance_stamp: {}",
            test_paths.instance_stamp_path().display()
        );
        println!(
            "  lock_path:   {}",
            test_paths.instance_lock_path().display()
        );
        println!();

        // Scenario 3: cargo run --release (no args) → Production profile
        println!("Scenario 3: cargo run --release (no args)");
        println!("  Profile: Production");
        let prod_paths = AppPaths::for_profile(AppProfile::Production).unwrap();
        println!("  qualifier: {}", prod_paths.profile().qualifier());
        println!("  config_dir: {}", prod_paths.config_dir().display());
        println!("  data_dir:   {}", prod_paths.data_dir().display());
        println!("  cache_dir:  {}", prod_paths.cache_dir().display());
        println!("  runtime_dir: {}", prod_paths.runtime_dir().display());
        println!("  state_dir:  {}", prod_paths.state_dir().display());
        println!(
            "  instance_stamp: {}",
            prod_paths.instance_stamp_path().display()
        );
        println!(
            "  lock_path:   {}",
            prod_paths.instance_lock_path().display()
        );
        println!();

        // Scenario 4: cargo run -- --profile qa → Custom("qa") profile
        println!("Scenario 4: cargo run -- --profile qa");
        println!("  Profile: Custom('qa')");
        let qa_paths = AppPaths::for_profile(AppProfile::Custom("qa".to_string())).unwrap();
        println!("  qualifier: {}", qa_paths.profile().qualifier());
        println!("  config_dir: {}", qa_paths.config_dir().display());
        println!("  data_dir:   {}", qa_paths.data_dir().display());
        println!("  cache_dir:  {}", qa_paths.cache_dir().display());
        println!("  runtime_dir: {}", qa_paths.runtime_dir().display());
        println!("  state_dir:  {}", qa_paths.state_dir().display());
        println!(
            "  instance_stamp: {}",
            qa_paths.instance_stamp_path().display()
        );
        println!("  lock_path:   {}", qa_paths.instance_lock_path().display());
        println!();

        // Verify all profiles use disjoint paths
        println!("=== Disjoint Path Verification ===\n");

        // Collect all config directories
        let profiles = [
            ("Development", dev_paths.clone()),
            ("Test", test_paths.clone()),
            ("Production", prod_paths.clone()),
            ("Custom('qa')", qa_paths.clone()),
        ];

        // Check that no two profiles share the same config directory
        for (i, (name_i, paths_i)) in profiles.iter().enumerate() {
            for (j, (name_j, paths_j)) in profiles.iter().enumerate() {
                if i < j {
                    assert_ne!(
                        paths_i.config_dir(),
                        paths_j.config_dir(),
                        "{} and {} should have different config dirs: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.config_dir().display(),
                        paths_j.config_dir().display()
                    );
                    assert_ne!(
                        paths_i.data_dir(),
                        paths_j.data_dir(),
                        "{} and {} should have different data dirs: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.data_dir().display(),
                        paths_j.data_dir().display()
                    );
                    assert_ne!(
                        paths_i.cache_dir(),
                        paths_j.cache_dir(),
                        "{} and {} should have different cache dirs: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.cache_dir().display(),
                        paths_j.cache_dir().display()
                    );
                    assert_ne!(
                        paths_i.runtime_dir(),
                        paths_j.runtime_dir(),
                        "{} and {} should have different runtime dirs: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.runtime_dir().display(),
                        paths_j.runtime_dir().display()
                    );
                    assert_ne!(
                        paths_i.state_dir(),
                        paths_j.state_dir(),
                        "{} and {} should have different state dirs: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.state_dir().display(),
                        paths_j.state_dir().display()
                    );
                    assert_ne!(
                        paths_i.instance_stamp_path(),
                        paths_j.instance_stamp_path(),
                        "{} and {} should have different instance stamp paths: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.instance_stamp_path().display(),
                        paths_j.instance_stamp_path().display()
                    );
                    assert_ne!(
                        paths_i.instance_lock_path(),
                        paths_j.instance_lock_path(),
                        "{} and {} should have different lock paths: {} vs {}",
                        name_i,
                        name_j,
                        paths_i.instance_lock_path().display(),
                        paths_j.instance_lock_path().display()
                    );
                }
            }
        }

        // Verify profile qualifiers are all different
        println!("Profile Qualifiers:");
        for (name, paths) in &profiles {
            println!("  {}: {}", name, paths.profile().qualifier());
        }
        println!();

        // Count unique qualifiers
        let qualifiers: Vec<_> = profiles
            .iter()
            .map(|(_, p)| p.profile().qualifier())
            .collect();
        let unique_qualifiers: std::collections::HashSet<_> = qualifiers.iter().collect();
        assert_eq!(
            unique_qualifiers.len(),
            profiles.len(),
            "Each profile should have a unique qualifier"
        );
        println!("✓ All {} profiles have unique qualifiers", profiles.len());
        println!();

        // Verify Test profile is truly isolated (uses temp dir, not qualifier-based)
        let test_config_str = test_paths.config_dir().to_string_lossy();
        assert!(
            test_config_str.starts_with("/tmp"),
            "Test config dir should be under /tmp for isolation: {}",
            test_config_str
        );
        println!("✓ Test profile uses isolated temporary directory (/tmp/*)");

        // Verify Production doesn't use Development qualifier
        let dev_qualifier = dev_paths.profile().qualifier();
        let prod_config_str = prod_paths.config_dir().to_string_lossy();
        assert!(
            !prod_config_str.ends_with(&dev_qualifier),
            "Production config dir should not use Development qualifier: {}",
            prod_config_str
        );
        println!("✓ Production profile is isolated from Development");

        // Verify Custom profile uses its own qualifier
        let qa_qualifier = qa_paths.profile().qualifier();
        let qa_config_str = qa_paths.config_dir().to_string_lossy();
        assert!(
            qa_config_str.ends_with(&qa_qualifier),
            "Custom('qa') config dir should end with its qualifier '{}': {}",
            qa_qualifier,
            qa_config_str
        );
        println!("✓ Custom('qa') profile uses its own qualifier");
        println!();

        println!("=== All Profile Paths Are Disjoint ===\n");
    }
}
