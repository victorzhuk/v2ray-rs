use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use prost::Message;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geodata::GeodataError;
use crate::models::BackendType;
use crate::persistence::AppPaths;

#[derive(Debug, Error)]
pub enum GeodataIndexError {
    #[error("protobuf decode failed: {0}")]
    ProtoDecode(String),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("geodata: {0}")]
    Geodata(#[from] GeodataError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeodataIndex {
    pub geoip_tags: Vec<String>,
    pub geosite_tags: Vec<String>,
    pub last_refresh: Option<DateTime<Utc>>,
    pub tag_counts: (usize, usize),
}

impl GeodataIndex {
    pub fn new(geoip_tags: Vec<String>, geosite_tags: Vec<String>) -> Self {
        let tag_counts = (geoip_tags.len(), geosite_tags.len());
        Self {
            geoip_tags,
            geosite_tags,
            last_refresh: Some(Utc::now()),
            tag_counts,
        }
    }
}

pub struct GeodataIndexManager {
    geodata_dir: PathBuf,
}

impl GeodataIndexManager {
    pub fn new(paths: &AppPaths) -> Self {
        let geodata_dir = paths.geodata_dir();
        Self { geodata_dir }
    }

    pub(crate) fn new_from_geodata_dir(geodata_dir: &Path) -> Self {
        Self {
            geodata_dir: geodata_dir.to_path_buf(),
        }
    }

    pub fn index_path(&self, backend: BackendType) -> PathBuf {
        let filename = match backend {
            BackendType::V2ray => "v2ray_index.json",
            BackendType::Xray => "xray_index.json",
            BackendType::SingBox => "singbox_index.json",
        };
        self.geodata_dir.join(filename)
    }

    pub fn load_index(
        &self,
        backend: BackendType,
    ) -> Result<Option<GeodataIndex>, GeodataIndexError> {
        let path = self.index_path(backend);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path)?;
        let index: GeodataIndex = serde_json::from_str(&contents)?;
        Ok(Some(index))
    }

    pub fn save_index(
        &self,
        backend: BackendType,
        index: &GeodataIndex,
    ) -> Result<(), GeodataIndexError> {
        let path = self.index_path(backend);
        let json = serde_json::to_string_pretty(index)?;
        let dir = path.parent().ok_or_else(|| {
            GeodataIndexError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "index path has no parent",
            ))
        })?;

        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }

        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.flush()?;
        tmp.persist(&path)
            .map_err(|e| GeodataIndexError::Io(e.error))?;
        Ok(())
    }

    pub fn build_index(
        &self,
        backend: BackendType,
        geoip_path: &Path,
        geosite_path: &Path,
    ) -> Result<GeodataIndex, GeodataIndexError> {
        let (geoip_tags, geosite_tags) = match backend {
            BackendType::V2ray | BackendType::Xray => {
                let geoip = parse_v2ray_geoip_dat(geoip_path)?;
                let geosite = parse_v2ray_geosite_dat(geosite_path)?;
                (geoip, geosite)
            }
            BackendType::SingBox => {
                let (geoip, geosite) = parse_singbox_db(geoip_path, geosite_path)?;
                (geoip, geosite)
            }
        };

        let index = GeodataIndex::new(geoip_tags, geosite_tags);
        self.save_index(backend, &index)?;
        Ok(index)
    }

}

fn parse_v2ray_geoip_dat(path: &Path) -> Result<Vec<String>, GeodataIndexError> {
    let bytes = std::fs::read(path)?;
    let list = v2ray_geoip::GeoIpList::decode(bytes.as_slice())
        .map_err(|e: prost::DecodeError| GeodataIndexError::ProtoDecode(e.to_string()))?;

    let tags: HashSet<String> = list.entry.into_iter().map(|e| e.country_code).collect();
    let mut sorted: Vec<String> = tags.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

fn parse_v2ray_geosite_dat(path: &Path) -> Result<Vec<String>, GeodataIndexError> {
    let bytes = std::fs::read(path)?;
    let list = v2ray_geosite::GeoSiteList::decode(bytes.as_slice())
        .map_err(|e: prost::DecodeError| GeodataIndexError::ProtoDecode(e.to_string()))?;

    let tags: HashSet<String> = list.entry.into_iter().map(|e| e.country_code).collect();
    let mut sorted: Vec<String> = tags.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

fn parse_singbox_db(
    geoip_path: &Path,
    geosite_path: &Path,
) -> Result<(Vec<String>, Vec<String>), GeodataIndexError> {
    let mut geoip_tags = Vec::new();
    let mut geosite_tags = Vec::new();

    {
        let conn = Connection::open(geoip_path)?;
        let mut stmt = conn.prepare("SELECT country_code FROM geoip")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let tag: String = row.get(0)?;
            geoip_tags.push(tag);
        }
    }

    {
        let conn = Connection::open(geosite_path)?;
        let mut stmt = conn.prepare("SELECT tag FROM geosite")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let tag: String = row.get(0)?;
            geosite_tags.push(tag);
        }
    }

    geoip_tags.sort();
    geoip_tags.dedup();
    geosite_tags.sort();
    geosite_tags.dedup();

    Ok((geoip_tags, geosite_tags))
}

mod v2ray_geoip {
    include!(concat!(env!("OUT_DIR"), "/v2ray_geoip.rs"));
}

mod v2ray_geosite {
    include!(concat!(env!("OUT_DIR"), "/v2ray_geosite.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_index_manager() -> (TempDir, GeodataIndexManager) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::from_paths(tmp.path().join("config"), tmp.path().join("data"));
        let manager = GeodataIndexManager::new(&paths);
        (tmp, manager)
    }

    #[test]
    fn test_index_path_v2ray() {
        let (_tmp, manager) = test_index_manager();
        let path = manager.index_path(BackendType::V2ray);
        assert!(path.ends_with("v2ray_index.json"));
    }

    #[test]
    fn test_index_path_xray() {
        let (_tmp, manager) = test_index_manager();
        let path = manager.index_path(BackendType::Xray);
        assert!(path.ends_with("xray_index.json"));
    }

    #[test]
    fn test_index_path_singbox() {
        let (_tmp, manager) = test_index_manager();
        let path = manager.index_path(BackendType::SingBox);
        assert!(path.ends_with("singbox_index.json"));
    }

    #[test]
    fn test_save_and_load_index() {
        let (_tmp, manager) = test_index_manager();
        let index = GeodataIndex::new(
            vec!["US".to_string(), "CN".to_string()],
            vec!["google".to_string(), "netflix".to_string()],
        );

        manager.save_index(BackendType::V2ray, &index).unwrap();
        let loaded = manager.load_index(BackendType::V2ray).unwrap().unwrap();

        assert_eq!(index.geoip_tags, loaded.geoip_tags);
        assert_eq!(index.geosite_tags, loaded.geosite_tags);
        assert_eq!(index.tag_counts, loaded.tag_counts);
    }

    #[test]
    fn test_load_index_missing() {
        let (_tmp, manager) = test_index_manager();
        let loaded = manager.load_index(BackendType::Xray).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_geodata_index_new() {
        let geoip_tags = vec!["US".to_string(), "CN".to_string(), "RU".to_string()];
        let geosite_tags = vec!["google".to_string(), "netflix".to_string()];
        let index = GeodataIndex::new(geoip_tags.clone(), geosite_tags.clone());

        assert_eq!(index.geoip_tags, geoip_tags);
        assert_eq!(index.geosite_tags, geosite_tags);
        assert_eq!(index.tag_counts, (3, 2));
        assert!(index.last_refresh.is_some());
    }

    #[test]
    fn test_geodata_index_serialization() {
        let index = GeodataIndex::new(vec!["US".to_string()], vec!["category-a".to_string()]);

        let json = serde_json::to_string(&index).unwrap();
        let deserialized: GeodataIndex = serde_json::from_str(&json).unwrap();

        assert_eq!(index.geoip_tags, deserialized.geoip_tags);
        assert_eq!(index.geosite_tags, deserialized.geosite_tags);
        assert_eq!(index.tag_counts, deserialized.tag_counts);
    }

    #[test]
    fn test_parse_singbox_db() {
        let tmp = TempDir::new().unwrap();
        let geoip_path = tmp.path().join("geoip.db");
        let geosite_path = tmp.path().join("geosite.db");

        {
            let conn = Connection::open(&geoip_path).unwrap();
            conn.execute("CREATE TABLE geoip (country_code TEXT)", [])
                .unwrap();
            conn.execute(
                "INSERT INTO geoip (country_code) VALUES (?1), (?2), (?3)",
                ["US", "CN", "RU"],
            )
            .unwrap();
        }

        {
            let conn = Connection::open(&geosite_path).unwrap();
            conn.execute("CREATE TABLE geosite (tag TEXT)", []).unwrap();
            conn.execute(
                "INSERT INTO geosite (tag) VALUES (?1), (?2)",
                ["google", "netflix"],
            )
            .unwrap();
        }

        let (geoip_tags, geosite_tags) = parse_singbox_db(&geoip_path, &geosite_path).unwrap();

        assert_eq!(geoip_tags, vec!["CN", "RU", "US"]);
        assert_eq!(geosite_tags, vec!["google", "netflix"]);
    }
}
