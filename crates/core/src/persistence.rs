use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use thiserror::Error;

use uuid::Uuid;

use crate::models::{AppSettings, Preset, RoutingRuleSet, Subscription};

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
}

#[derive(Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl AppPaths {
    pub fn new() -> Result<Self, PersistenceError> {
        let dirs =
            ProjectDirs::from("com", "v2ray-rs", "v2ray-rs").ok_or(PersistenceError::NoDirs)?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
        })
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub fn from_paths(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            config_dir,
            data_dir,
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
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
        self.data_dir.join("geodata")
    }

    pub fn presets_dir(&self) -> PathBuf {
        self.data_dir.join("presets")
    }

    pub fn ensure_dirs(&self) -> Result<(), PersistenceError> {
        create_dir_with_permissions(&self.config_dir)?;
        create_dir_with_permissions(&self.data_dir)?;
        Ok(())
    }
}

fn create_dir_with_permissions(path: &Path) -> Result<(), PersistenceError> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), PersistenceError> {
    let dir = path.parent().ok_or_else(|| {
        PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent directory",
        ))
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(data)?;
    tmp.flush()?;
    tmp.persist(path)
        .map_err(|e| PersistenceError::Io(e.error))?;
    Ok(())
}

pub fn save_settings(paths: &AppPaths, settings: &AppSettings) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let toml_str = toml::to_string_pretty(settings)?;
    atomic_write(&paths.settings_path(), toml_str.as_bytes())
}

pub fn load_settings(paths: &AppPaths) -> Result<AppSettings, PersistenceError> {
    let path = paths.settings_path();
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let contents = fs::read_to_string(&path)?;
    match toml::from_str::<AppSettings>(&contents) {
        Ok(settings) => Ok(settings),
        Err(e) => Err(PersistenceError::CorruptConfig(e.to_string())),
    }
}

pub fn load_settings_or_default(paths: &AppPaths) -> AppSettings {
    match load_settings(paths) {
        Ok(s) => s,
        Err(PersistenceError::CorruptConfig(msg)) => {
            eprintln!("Warning: {msg}. Using default settings.");
            AppSettings::default()
        }
        Err(e) => {
            eprintln!("Warning: failed to load settings: {e}. Using defaults.");
            AppSettings::default()
        }
    }
}

pub fn save_subscriptions(
    paths: &AppPaths,
    subscriptions: &[Subscription],
) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(subscriptions)?;
    atomic_write(&paths.subscriptions_path(), json.as_bytes())
}

pub fn load_subscriptions(paths: &AppPaths) -> Result<Vec<Subscription>, PersistenceError> {
    let path = paths.subscriptions_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path)?;
    let subs: Vec<Subscription> = serde_json::from_str(&contents)?;
    Ok(subs)
}

pub fn add_subscription(
    paths: &AppPaths,
    subscription: Subscription,
) -> Result<(), PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    subs.push(subscription);
    save_subscriptions(paths, &subs)
}

pub fn get_subscription(
    paths: &AppPaths,
    id: &Uuid,
) -> Result<Option<Subscription>, PersistenceError> {
    let subs = load_subscriptions(paths)?;
    Ok(subs.into_iter().find(|s| &s.id == id))
}

pub fn update_subscription(
    paths: &AppPaths,
    subscription: Subscription,
) -> Result<bool, PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    match subs.iter_mut().find(|s| s.id == subscription.id) {
        Some(existing) => {
            *existing = subscription;
            save_subscriptions(paths, &subs)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn remove_subscription(paths: &AppPaths, id: &Uuid) -> Result<bool, PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    let initial_len = subs.len();
    subs.retain(|s| &s.id != id);
    if subs.len() < initial_len {
        save_subscriptions(paths, &subs)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn save_routing_rules(
    paths: &AppPaths,
    rules: &RoutingRuleSet,
) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(rules)?;
    atomic_write(&paths.routing_rules_path(), json.as_bytes())
}

pub fn load_routing_rules(paths: &AppPaths) -> Result<RoutingRuleSet, PersistenceError> {
    let path = paths.routing_rules_path();
    if !path.exists() {
        return Ok(RoutingRuleSet::new());
    }
    let contents = fs::read_to_string(&path)?;
    let rules: RoutingRuleSet = serde_json::from_str(&contents)?;
    Ok(rules)
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn save_preset(paths: &AppPaths, preset: &Preset) -> Result<(), PersistenceError> {
    let dir = paths.presets_dir();
    create_dir_with_permissions(&dir)?;
    let filename = format!("{}.json", slugify(&preset.name));
    let json = serde_json::to_string_pretty(preset)?;
    atomic_write(&dir.join(filename), json.as_bytes())
}

pub fn load_custom_presets(paths: &AppPaths) -> Result<Vec<Preset>, PersistenceError> {
    let dir = paths.presets_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut presets = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let contents = fs::read_to_string(&path)?;
            if let Ok(preset) = serde_json::from_str::<Preset>(&contents) {
                presets.push(preset);
            }
        }
    }
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(presets)
}

pub fn delete_preset(paths: &AppPaths, name: &str) -> Result<bool, PersistenceError> {
    let dir = paths.presets_dir();
    let filename = format!("{}.json", slugify(name));
    let path = dir.join(filename);
    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn test_paths() -> (TempDir, AppPaths) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::from_paths(tmp.path().join("config"), tmp.path().join("data"));
        (tmp, paths)
    }

    #[test]
    fn test_ensure_dirs_creates_directories() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir().exists());
        assert!(paths.data_dir().exists());

        let config_perms = fs::metadata(paths.config_dir()).unwrap().permissions();
        assert_eq!(config_perms.mode() & 0o777, 0o700);
    }

    #[test]
    fn test_settings_save_load_roundtrip() {
        let (_tmp, paths) = test_paths();
        let mut settings = AppSettings::default();
        settings.socks_port = 9999;
        settings.language = Language::Russian;

        save_settings(&paths, &settings).unwrap();
        let loaded = load_settings(&paths).unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn test_load_settings_missing_file_returns_default() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_settings(&paths).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn test_corrupt_config_falls_back() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        fs::write(paths.settings_path(), "invalid {{{{toml").unwrap();

        let loaded = load_settings_or_default(&paths);
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn test_subscriptions_save_load_roundtrip() {
        let (_tmp, paths) = test_paths();
        let subs = vec![Subscription::new_from_url(
            "Test Sub",
            "https://example.com/sub",
        )];

        save_subscriptions(&paths, &subs).unwrap();
        let loaded = load_subscriptions(&paths).unwrap();

        assert_eq!(subs.len(), loaded.len());
        assert_eq!(subs[0].name, loaded[0].name);
    }

    #[test]
    fn test_routing_rules_save_load_roundtrip() {
        let (_tmp, paths) = test_paths();
        let mut rules = RoutingRuleSet::new();
        rules.add(RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
        });

        save_routing_rules(&paths, &rules).unwrap();
        let loaded = load_routing_rules(&paths).unwrap();

        assert_eq!(rules.rules().len(), loaded.rules().len());
        assert_eq!(
            rules.rules()[0].match_condition,
            loaded.rules()[0].match_condition
        );
    }

    #[test]
    fn test_load_subscriptions_missing_file() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_subscriptions(&paths).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_load_routing_rules_missing_file() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_routing_rules(&paths).unwrap();
        assert!(loaded.rules().is_empty());
    }

    #[test]
    fn test_atomic_write_does_not_corrupt_on_overwrite() {
        let (_tmp, paths) = test_paths();
        let settings1 = AppSettings::default();
        save_settings(&paths, &settings1).unwrap();

        let mut settings2 = AppSettings::default();
        settings2.socks_port = 2222;
        save_settings(&paths, &settings2).unwrap();

        let loaded = load_settings(&paths).unwrap();
        assert_eq!(loaded.socks_port, 2222);
    }

    #[test]
    fn test_add_subscription() {
        let (_tmp, paths) = test_paths();
        let sub1 = Subscription::new_from_url("First", "https://example.com/1");
        let sub2 = Subscription::new_from_url("Second", "https://example.com/2");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, sub1.id);
        assert_eq!(loaded[1].id, sub2.id);
    }

    #[test]
    fn test_get_subscription() {
        let (_tmp, paths) = test_paths();
        let sub = Subscription::new_from_url("Test", "https://example.com/sub");

        add_subscription(&paths, sub.clone()).unwrap();

        let found = get_subscription(&paths, &sub.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, sub.id);

        let not_found = get_subscription(&paths, &Uuid::new_v4()).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_update_subscription() {
        let (_tmp, paths) = test_paths();
        let mut sub = Subscription::new_from_url("Original", "https://example.com/sub");

        add_subscription(&paths, sub.clone()).unwrap();

        sub.name = "Updated".into();
        let updated = update_subscription(&paths, sub.clone()).unwrap();
        assert!(updated);

        let loaded = get_subscription(&paths, &sub.id).unwrap().unwrap();
        assert_eq!(loaded.name, "Updated");
    }

    #[test]
    fn test_remove_subscription() {
        let (_tmp, paths) = test_paths();
        let sub1 = Subscription::new_from_url("First", "https://example.com/1");
        let sub2 = Subscription::new_from_url("Second", "https://example.com/2");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();

        let removed = remove_subscription(&paths, &sub1.id).unwrap();
        assert!(removed);

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, sub2.id);
    }

    #[test]
    fn test_remove_nonexistent() {
        let (_tmp, paths) = test_paths();
        let removed = remove_subscription(&paths, &Uuid::new_v4()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_save_and_load_custom_preset() {
        let (_tmp, paths) = test_paths();
        let presets = crate::models::builtin_presets();
        let preset = &presets[0];

        save_preset(&paths, preset).unwrap();
        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, preset.name);
        assert_eq!(loaded[0].description, preset.description);
    }

    #[test]
    fn test_delete_preset() {
        let (_tmp, paths) = test_paths();
        let presets = crate::models::builtin_presets();
        save_preset(&paths, &presets[0]).unwrap();
        save_preset(&paths, &presets[1]).unwrap();

        let deleted = delete_preset(&paths, &presets[0].name).unwrap();
        assert!(deleted);

        let loaded = load_custom_presets(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, presets[1].name);
    }

    #[test]
    fn test_delete_nonexistent_preset() {
        let (_tmp, paths) = test_paths();
        let deleted = delete_preset(&paths, "nope").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_load_custom_presets_empty() {
        let (_tmp, paths) = test_paths();
        let loaded = load_custom_presets(&paths).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_multiple_independent_subscriptions() {
        let (_tmp, paths) = test_paths();
        let sub1 = Subscription::new_from_url("URL1", "https://example.com/1");
        let sub2 = Subscription::new_from_url("URL2", "https://example.com/2");
        let sub3 = Subscription::new_from_file("File1", "/path/to/file");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();
        add_subscription(&paths, sub3.clone()).unwrap();

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].source, sub1.source);
        assert_eq!(loaded[1].source, sub2.source);
        assert_eq!(loaded[2].source, sub3.source);
        assert!(loaded.iter().all(|s| s.nodes.is_empty()));
    }
}
