use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fs::atomic_write;
use crate::geodata_index::GeodataIndexManager;
use crate::models::BackendType;
use crate::persistence::AppPaths;

const GEODATA_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_GEODATA_SIZE: u64 = 100 * 1024 * 1024; // 100 MB

const GEOIP_RULESET_URL: &str = "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set";
const GEOSITE_RULESET_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set";

#[derive(Debug, Error)]
pub enum GeodataError {
    #[error("download failed: {url}: {reason}")]
    Download { url: String, reason: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("metadata: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeodataMetadata {
    pub last_check: DateTime<Utc>,
    pub geoip_version: Option<String>,
    pub geosite_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeodataDownload {
    pub url: String,
    pub filename: String,
}

#[derive(Debug)]
pub struct GeodataManager {
    geodata_dir: PathBuf,
    metadata_path: PathBuf,
}

impl GeodataManager {
    pub fn new(paths: &AppPaths) -> Self {
        let geodata_dir = paths.geodata_dir();
        let metadata_path = geodata_dir.join("metadata.json");
        Self {
            geodata_dir,
            metadata_path,
        }
    }

    pub fn ensure_dir(&self) -> Result<(), GeodataError> {
        ensure_dir_0700(&self.geodata_dir)?;
        ensure_dir_0700(&self.rule_sets_dir())?;
        Ok(())
    }

    pub fn geodata_dir(&self) -> &Path {
        &self.geodata_dir
    }

    pub fn rule_sets_dir(&self) -> PathBuf {
        self.geodata_dir.join("rule-sets")
    }

    pub fn rule_set_path(&self, full_tag: &str) -> PathBuf {
        self.rule_sets_dir().join(format!("{full_tag}.srs"))
    }

    pub fn has_rule_set(&self, full_tag: &str) -> bool {
        self.rule_set_path(full_tag).exists()
    }

    pub fn geoip_path(&self) -> PathBuf {
        self.geodata_dir.join("geoip.dat")
    }

    pub fn geosite_path(&self) -> PathBuf {
        self.geodata_dir.join("geosite.dat")
    }

    pub fn has_geodata(&self) -> bool {
        self.geoip_path().exists() && self.geosite_path().exists()
    }

    pub fn load_metadata(&self) -> Result<Option<GeodataMetadata>, GeodataError> {
        if !self.metadata_path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&self.metadata_path)?;
        let metadata: GeodataMetadata = serde_json::from_str(&contents)?;
        Ok(Some(metadata))
    }

    pub fn save_metadata(&self, metadata: &GeodataMetadata) -> Result<(), GeodataError> {
        self.ensure_dir()?;
        let json = serde_json::to_string_pretty(metadata)?;
        atomic_write(&self.metadata_path, json.as_bytes()).map_err(GeodataError::Io)?;
        Ok(())
    }

    pub fn needs_update(&self, interval: Duration) -> bool {
        match self.load_metadata() {
            Ok(Some(metadata)) => {
                let elapsed = Utc::now()
                    .signed_duration_since(metadata.last_check)
                    .num_seconds();
                elapsed >= interval.as_secs() as i64
            }
            Ok(None) => true,
            Err(e) => {
                log::warn!("failed to load geodata metadata, assuming update needed: {e}");
                true
            }
        }
    }

    pub fn download_urls() -> Vec<GeodataDownload> {
        vec![
            GeodataDownload {
                url: "https://github.com/v2fly/geoip/releases/latest/download/geoip.dat".into(),
                filename: "geoip.dat".into(),
            },
            GeodataDownload {
                url: "https://github.com/v2fly/domain-list-community/releases/latest/download/dlc.dat".into(),
                filename: "geosite.dat".into(),
            },
        ]
    }

    pub fn reindex(&self, backend: BackendType) -> Result<(), GeodataError> {
        let index_manager = GeodataIndexManager::new_from_geodata_dir(&self.geodata_dir);

        match backend {
            BackendType::V2ray | BackendType::Xray => {
                let geoip_path = self.geoip_path();
                let geosite_path = self.geosite_path();

                if !geoip_path.exists() || !geosite_path.exists() {
                    return Err(GeodataError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "geodata files not found",
                    )));
                }

                index_manager
                    .build_index(backend, &geoip_path, &geosite_path)
                    .map_err(|e| GeodataError::Io(std::io::Error::other(e.to_string())))?;
            }
            BackendType::SingBox => {
                index_manager
                    .build_singbox_index(&self.rule_sets_dir())
                    .map_err(|e| GeodataError::Io(std::io::Error::other(e.to_string())))?;
            }
        }

        Ok(())
    }
}

fn ensure_dir_0700(path: &Path) -> Result<(), GeodataError> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn rule_set_url(full_tag: &str) -> Option<String> {
    if full_tag.starts_with("geoip-") {
        Some(format!("{GEOIP_RULESET_URL}/{full_tag}.srs"))
    } else if full_tag.starts_with("geosite-") {
        Some(format!("{GEOSITE_RULESET_URL}/{full_tag}.srs"))
    } else {
        None
    }
}

#[cfg(feature = "geodata-fetch")]
pub fn check_and_download(
    manager: &GeodataManager,
    interval: Duration,
) -> Result<Option<GeodataMetadata>, GeodataError> {
    if manager.has_geodata() && !manager.needs_update(interval) {
        return Ok(None);
    }
    download_geodata(manager).map(Some)
}

#[cfg(feature = "geodata-fetch")]
use std::io::Write;

#[cfg(feature = "geodata-fetch")]
pub fn download_geodata(manager: &GeodataManager) -> Result<GeodataMetadata, GeodataError> {
    manager.ensure_dir()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(GEODATA_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| GeodataError::Download {
            url: String::new(),
            reason: e.to_string(),
        })?;

    for dl in GeodataManager::download_urls() {
        let target = manager.geodata_dir().join(&dl.filename);
        let response = client
            .get(&dl.url)
            .send()
            .map_err(|e| GeodataError::Download {
                url: dl.url.clone(),
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(GeodataError::Download {
                url: dl.url,
                reason: format!("HTTP {}", response.status()),
            });
        }

        let bytes = response.bytes().map_err(|e| GeodataError::Download {
            url: dl.url.clone(),
            reason: e.to_string(),
        })?;

        if bytes.len() as u64 > MAX_GEODATA_SIZE {
            return Err(GeodataError::Download {
                url: dl.url,
                reason: format!(
                    "response too large: {} bytes (max {MAX_GEODATA_SIZE})",
                    bytes.len()
                ),
            });
        }

        let dir = target.parent().ok_or_else(|| {
            GeodataError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "download target path has no parent",
            ))
        })?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist(&target)
            .map_err(|e| GeodataError::Io(e.error))?;
    }

    let metadata = GeodataMetadata {
        last_check: chrono::Utc::now(),
        geoip_version: None,
        geosite_version: None,
    };
    manager.save_metadata(&metadata)?;
    Ok(metadata)
}

/// Fetches only the sing-box rule-sets not already cached locally. Individual
/// `.srs` files are versioned independently upstream and never expire once
/// downloaded, so "missing" is the only staleness signal we track.
#[cfg(feature = "geodata-fetch")]
pub fn download_singbox_rule_sets(
    manager: &GeodataManager,
    tags: &[String],
) -> Result<GeodataMetadata, GeodataError> {
    manager.ensure_dir()?;
    let _ = std::fs::remove_file(manager.geodata_dir().join("geoip.db"));
    let _ = std::fs::remove_file(manager.geodata_dir().join("geosite.db"));

    let client = reqwest::blocking::Client::builder()
        .timeout(GEODATA_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| GeodataError::Download {
            url: String::new(),
            reason: e.to_string(),
        })?;

    for tag in tags {
        if manager.has_rule_set(tag) {
            continue;
        }
        let Some(url) = rule_set_url(tag) else {
            log::warn!("unrecognized sing-box rule-set tag, skipping: {tag}");
            continue;
        };

        let response = client
            .get(&url)
            .send()
            .map_err(|e| GeodataError::Download {
                url: url.clone(),
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(GeodataError::Download {
                url,
                reason: format!("HTTP {}", response.status()),
            });
        }

        let bytes = response.bytes().map_err(|e| GeodataError::Download {
            url: url.clone(),
            reason: e.to_string(),
        })?;

        if bytes.len() as u64 > MAX_GEODATA_SIZE {
            return Err(GeodataError::Download {
                url,
                reason: format!(
                    "response too large: {} bytes (max {MAX_GEODATA_SIZE})",
                    bytes.len()
                ),
            });
        }

        let mut tmp = tempfile::NamedTempFile::new_in(manager.rule_sets_dir())?;
        tmp.write_all(&bytes)?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist(manager.rule_set_path(tag))
            .map_err(|e| GeodataError::Io(e.error))?;
    }

    let metadata = GeodataMetadata {
        last_check: chrono::Utc::now(),
        geoip_version: None,
        geosite_version: None,
    };
    manager.save_metadata(&metadata)?;
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AppProfile;
    use tempfile::TempDir;

    fn test_manager() -> (TempDir, GeodataManager) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let manager = GeodataManager::new(&paths);
        (tmp, manager)
    }

    #[test]
    fn test_metadata_save_load_roundtrip() {
        let (_tmp, manager) = test_manager();
        let metadata = GeodataMetadata {
            last_check: Utc::now(),
            geoip_version: Some("1.0".into()),
            geosite_version: Some("2.0".into()),
        };

        manager.save_metadata(&metadata).unwrap();
        let loaded = manager.load_metadata().unwrap().unwrap();

        assert_eq!(
            metadata.last_check.timestamp(),
            loaded.last_check.timestamp()
        );
        assert_eq!(metadata.geoip_version, loaded.geoip_version);
        assert_eq!(metadata.geosite_version, loaded.geosite_version);
    }

    #[test]
    fn test_load_metadata_missing_file() {
        let (_tmp, manager) = test_manager();
        let loaded = manager.load_metadata().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_needs_update_no_metadata() {
        let (_tmp, manager) = test_manager();
        assert!(manager.needs_update(Duration::from_secs(3600)));
    }

    #[test]
    fn test_needs_update_recent_check() {
        let (_tmp, manager) = test_manager();
        let metadata = GeodataMetadata {
            last_check: Utc::now(),
            geoip_version: None,
            geosite_version: None,
        };
        manager.save_metadata(&metadata).unwrap();

        assert!(!manager.needs_update(Duration::from_secs(3600)));
    }

    #[test]
    fn test_needs_update_old_check() {
        let (_tmp, manager) = test_manager();
        let old_time = Utc::now() - chrono::Duration::seconds(7200);
        let metadata = GeodataMetadata {
            last_check: old_time,
            geoip_version: None,
            geosite_version: None,
        };
        manager.save_metadata(&metadata).unwrap();

        assert!(manager.needs_update(Duration::from_secs(3600)));
    }

    #[test]
    fn test_has_geodata_missing_files() {
        let (_tmp, manager) = test_manager();
        assert!(!manager.has_geodata());
    }

    #[test]
    fn test_has_geodata_with_files() {
        let (_tmp, manager) = test_manager();
        manager.ensure_dir().unwrap();

        std::fs::write(manager.geoip_path(), b"test").unwrap();
        std::fs::write(manager.geosite_path(), b"test").unwrap();

        assert!(manager.has_geodata());
    }

    #[test]
    fn test_geoip_path() {
        let (_tmp, manager) = test_manager();
        assert!(manager.geoip_path().ends_with("geoip.dat"));
    }

    #[test]
    fn test_geosite_path() {
        let (_tmp, manager) = test_manager();
        assert!(manager.geosite_path().ends_with("geosite.dat"));
    }

    #[test]
    fn test_download_urls() {
        let urls = GeodataManager::download_urls();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].url.contains("v2fly/geoip"));
        assert_eq!(urls[0].filename, "geoip.dat");
        assert!(urls[1].url.contains("domain-list-community"));
        assert_eq!(urls[1].filename, "geosite.dat");
    }

    #[test]
    fn test_rule_sets_dir() {
        let (_tmp, manager) = test_manager();
        assert!(manager.rule_sets_dir().ends_with("geodata/rule-sets"));
    }

    #[test]
    fn test_rule_set_path() {
        let (_tmp, manager) = test_manager();
        let path = manager.rule_set_path("geoip-ru");
        assert!(path.ends_with("rule-sets/geoip-ru.srs"));
    }

    #[test]
    fn test_has_rule_set() {
        let (_tmp, manager) = test_manager();
        assert!(!manager.has_rule_set("geoip-ru"));

        manager.ensure_dir().unwrap();
        std::fs::write(manager.rule_set_path("geoip-ru"), b"test").unwrap();
        assert!(manager.has_rule_set("geoip-ru"));
    }

    #[test]
    fn test_rule_set_url_geoip() {
        let url = rule_set_url("geoip-ru").unwrap();
        assert!(url.contains("SagerNet/sing-geoip"));
        assert!(url.ends_with("geoip-ru.srs"));
    }

    #[test]
    fn test_rule_set_url_geosite() {
        let url = rule_set_url("geosite-google").unwrap();
        assert!(url.contains("SagerNet/sing-geosite"));
        assert!(url.ends_with("geosite-google.srs"));
    }

    #[test]
    fn test_rule_set_url_unrecognized_tag() {
        assert!(rule_set_url("bogus-tag").is_none());
    }

    #[test]
    fn test_ensure_dir_creates_directory() {
        let (_tmp, manager) = test_manager();
        manager.ensure_dir().unwrap();
        assert!(manager.geodata_dir().exists());
        assert!(manager.rule_sets_dir().exists());
    }

    #[test]
    fn test_reindex_singbox_builds_index_from_rule_sets() {
        let (_tmp, manager) = test_manager();
        manager.ensure_dir().unwrap();

        use crate::geodata_index::GeodataIndexManager;

        std::fs::write(manager.rule_set_path("geoip-us"), b"fake").unwrap();
        std::fs::write(manager.rule_set_path("geosite-google"), b"fake").unwrap();

        manager.reindex(BackendType::SingBox).unwrap();

        let index_manager = GeodataIndexManager::new_from_geodata_dir(manager.geodata_dir());
        let index = index_manager.load_index(BackendType::SingBox).unwrap();
        assert!(index.is_some());
        let index = index.unwrap();
        assert_eq!(index.geoip_tags, vec!["us"]);
        assert_eq!(index.geosite_tags, vec!["google"]);
    }

    #[test]
    fn test_reindex_missing_files_returns_error() {
        let (_tmp, manager) = test_manager();
        let result = manager.reindex(BackendType::V2ray);
        assert!(result.is_err());
    }
}
