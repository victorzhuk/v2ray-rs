use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::models::BackendType;
use crate::persistence::AppPaths;

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

pub struct GeodataDownload {
    pub url: String,
    pub filename: String,
}

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
        if !self.geodata_dir.exists() {
            std::fs::create_dir_all(&self.geodata_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&self.geodata_dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    pub fn geodata_dir(&self) -> &Path {
        &self.geodata_dir
    }

    pub fn geoip_path(&self, backend: BackendType) -> PathBuf {
        let filename = match backend {
            BackendType::V2ray | BackendType::Xray => "geoip.dat",
            BackendType::SingBox => "geoip.db",
        };
        self.geodata_dir.join(filename)
    }

    pub fn geosite_path(&self, backend: BackendType) -> PathBuf {
        let filename = match backend {
            BackendType::V2ray | BackendType::Xray => "geosite.dat",
            BackendType::SingBox => "geosite.db",
        };
        self.geodata_dir.join(filename)
    }

    pub fn has_geodata(&self, backend: BackendType) -> bool {
        self.geoip_path(backend).exists() && self.geosite_path(backend).exists()
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
        let dir = self
            .metadata_path
            .parent()
            .ok_or_else(|| GeodataError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "metadata path has no parent",
            )))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.flush()?;
        tmp.persist(&self.metadata_path)
            .map_err(|e| GeodataError::Io(e.error))?;
        Ok(())
    }

    pub fn needs_update(&self, interval_secs: u64) -> bool {
        match self.load_metadata() {
            Ok(Some(metadata)) => {
                let elapsed = Utc::now()
                    .signed_duration_since(metadata.last_check)
                    .num_seconds();
                elapsed >= interval_secs as i64
            }
            _ => true,
        }
    }

    pub fn download_urls(backend: BackendType) -> Vec<GeodataDownload> {
        match backend {
            BackendType::V2ray | BackendType::Xray => vec![
                GeodataDownload {
                    url: "https://github.com/v2fly/geoip/releases/latest/download/geoip.dat"
                        .into(),
                    filename: "geoip.dat".into(),
                },
                GeodataDownload {
                    url: "https://github.com/v2fly/domain-list-community/releases/latest/download/dlc.dat".into(),
                    filename: "geosite.dat".into(),
                },
            ],
            BackendType::SingBox => vec![
                GeodataDownload {
                    url: "https://github.com/SagerNet/sing-geoip/releases/latest/download/geoip.db"
                        .into(),
                    filename: "geoip.db".into(),
                },
                GeodataDownload {
                    url: "https://github.com/SagerNet/sing-geosite/releases/latest/download/geosite.db".into(),
                    filename: "geosite.db".into(),
                },
            ],
        }
    }
}

#[cfg(feature = "geodata-fetch")]
pub fn check_and_download(
    manager: &GeodataManager,
    backend: BackendType,
    interval_secs: u64,
) -> Result<Option<GeodataMetadata>, GeodataError> {
    if manager.has_geodata(backend) && !manager.needs_update(interval_secs) {
        return Ok(None);
    }
    download_geodata(manager, backend).map(Some)
}

#[cfg(feature = "geodata-fetch")]
pub fn download_geodata(
    manager: &GeodataManager,
    backend: BackendType,
) -> Result<GeodataMetadata, GeodataError> {
    manager.ensure_dir()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| GeodataError::Download {
            url: String::new(),
            reason: e.to_string(),
        })?;

    for dl in GeodataManager::download_urls(backend) {
        let target = manager.geodata_dir().join(&dl.filename);
        let response = client.get(&dl.url).send().map_err(|e| {
            GeodataError::Download {
                url: dl.url.clone(),
                reason: e.to_string(),
            }
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

        let dir = target.parent().unwrap();
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut tmp, &bytes)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manager() -> (TempDir, GeodataManager) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::from_paths(tmp.path().join("config"), tmp.path().join("data"));
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
        assert!(manager.needs_update(3600));
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

        assert!(!manager.needs_update(3600));
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

        assert!(manager.needs_update(3600));
    }

    #[test]
    fn test_has_geodata_missing_files() {
        let (_tmp, manager) = test_manager();
        assert!(!manager.has_geodata(BackendType::V2ray));
        assert!(!manager.has_geodata(BackendType::Xray));
        assert!(!manager.has_geodata(BackendType::SingBox));
    }

    #[test]
    fn test_has_geodata_with_files() {
        let (_tmp, manager) = test_manager();
        manager.ensure_dir().unwrap();

        std::fs::write(manager.geoip_path(BackendType::V2ray), b"test").unwrap();
        std::fs::write(manager.geosite_path(BackendType::V2ray), b"test").unwrap();

        assert!(manager.has_geodata(BackendType::V2ray));
        assert!(manager.has_geodata(BackendType::Xray));
    }

    #[test]
    fn test_geoip_path_v2ray() {
        let (_tmp, manager) = test_manager();
        let path = manager.geoip_path(BackendType::V2ray);
        assert!(path.ends_with("geoip.dat"));
    }

    #[test]
    fn test_geoip_path_xray() {
        let (_tmp, manager) = test_manager();
        let path = manager.geoip_path(BackendType::Xray);
        assert!(path.ends_with("geoip.dat"));
    }

    #[test]
    fn test_geoip_path_singbox() {
        let (_tmp, manager) = test_manager();
        let path = manager.geoip_path(BackendType::SingBox);
        assert!(path.ends_with("geoip.db"));
    }

    #[test]
    fn test_geosite_path_v2ray() {
        let (_tmp, manager) = test_manager();
        let path = manager.geosite_path(BackendType::V2ray);
        assert!(path.ends_with("geosite.dat"));
    }

    #[test]
    fn test_geosite_path_singbox() {
        let (_tmp, manager) = test_manager();
        let path = manager.geosite_path(BackendType::SingBox);
        assert!(path.ends_with("geosite.db"));
    }

    #[test]
    fn test_download_urls_v2ray() {
        let urls = GeodataManager::download_urls(BackendType::V2ray);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].url.contains("v2fly/geoip"));
        assert_eq!(urls[0].filename, "geoip.dat");
        assert!(urls[1].url.contains("domain-list-community"));
        assert_eq!(urls[1].filename, "geosite.dat");
    }

    #[test]
    fn test_download_urls_xray() {
        let urls = GeodataManager::download_urls(BackendType::Xray);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].url.contains("v2fly/geoip"));
        assert_eq!(urls[0].filename, "geoip.dat");
    }

    #[test]
    fn test_download_urls_singbox() {
        let urls = GeodataManager::download_urls(BackendType::SingBox);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].url.contains("SagerNet/sing-geoip"));
        assert_eq!(urls[0].filename, "geoip.db");
        assert!(urls[1].url.contains("SagerNet/sing-geosite"));
        assert_eq!(urls[1].filename, "geosite.db");
    }

    #[test]
    fn test_ensure_dir_creates_directory() {
        let (_tmp, manager) = test_manager();
        manager.ensure_dir().unwrap();
        assert!(manager.geodata_dir().exists());
    }
}
