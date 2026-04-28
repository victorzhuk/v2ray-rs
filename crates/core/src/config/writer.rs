use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::config::{ConfigError, generator_for};
use crate::fs::atomic_write;
use crate::models::{AppSettings, BackendType, ProxyNode, RoutingRule};
use crate::persistence::AppPaths;

pub struct ConfigWriter {
    output_dir: PathBuf,
}

impl ConfigWriter {
    pub fn new(settings: &AppSettings, paths: &AppPaths) -> Self {
        let output_dir = settings
            .backend
            .config_output_dir
            .clone()
            .unwrap_or_else(|| paths.generated_dir());

        Self { output_dir }
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { output_dir: dir }
    }

    pub fn output_path(&self, backend: BackendType) -> PathBuf {
        let filename = match backend {
            BackendType::V2ray => "v2ray.json",
            BackendType::Xray => "xray.json",
            BackendType::SingBox => "sing-box.json",
        };
        self.output_dir.join(filename)
    }

    pub fn write_config(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<PathBuf, ConfigError> {
        validate_runtime_inputs(nodes, settings)?;

        let backend = settings.backend.backend_type;
        let generator = generator_for(backend);
        let config = generator.generate(nodes, rules, settings)?;
        let json = serde_json::to_string(&config)?;

        std::fs::create_dir_all(&self.output_dir)?;
        #[cfg(unix)]
        std::fs::set_permissions(&self.output_dir, std::fs::Permissions::from_mode(0o700))?;

        let path = self.output_path(backend);
        atomic_write(&path, json.as_bytes()).map_err(ConfigError::Io)?;

        Ok(path)
    }
}

fn validate_runtime_inputs(nodes: &[ProxyNode], settings: &AppSettings) -> Result<(), ConfigError> {
    for node in nodes {
        node.validate()?;
    }

    if settings.dns.enabled {
        settings.dns.validate()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use crate::profile::AppProfile;

    fn sample_nodes() -> Vec<ProxyNode> {
        vec![ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "ss.example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some("Test SS".into()),
        })]
    }

    fn sample_rules() -> Vec<RoutingRule> {
        vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
        }]
    }

    #[test]
    fn test_output_path_v2ray() {
        let writer = ConfigWriter::with_dir(PathBuf::from("/tmp/test"));
        assert_eq!(
            writer.output_path(BackendType::V2ray),
            PathBuf::from("/tmp/test/v2ray.json")
        );
    }

    #[test]
    fn test_output_path_xray() {
        let writer = ConfigWriter::with_dir(PathBuf::from("/tmp/test"));
        assert_eq!(
            writer.output_path(BackendType::Xray),
            PathBuf::from("/tmp/test/xray.json")
        );
    }

    #[test]
    fn test_output_path_singbox() {
        let writer = ConfigWriter::with_dir(PathBuf::from("/tmp/test"));
        assert_eq!(
            writer.output_path(BackendType::SingBox),
            PathBuf::from("/tmp/test/sing-box.json")
        );
    }

    #[test]
    fn test_write_config_creates_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let settings = AppSettings::default();

        let path = writer
            .write_config(&sample_nodes(), &sample_rules(), &settings)
            .unwrap();

        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert!(parsed["outbounds"].is_array());
    }

    #[test]
    fn test_write_config_v2ray() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let mut settings = AppSettings::default();
        settings.backend.backend_type = BackendType::V2ray;

        let path = writer
            .write_config(&sample_nodes(), &[], &settings)
            .unwrap();

        assert!(path.to_str().unwrap().contains("v2ray.json"));
        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["outbounds"][0]["protocol"], "shadowsocks");
    }

    #[test]
    fn test_write_config_singbox() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let mut settings = AppSettings::default();
        settings.backend.backend_type = BackendType::SingBox;

        let path = writer
            .write_config(&sample_nodes(), &[], &settings)
            .unwrap();

        assert!(path.to_str().unwrap().contains("sing-box.json"));
        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["outbounds"][0]["type"], "shadowsocks");
    }

    #[test]
    fn test_write_config_overwrites_atomically() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let settings = AppSettings::default();

        let path = writer
            .write_config(&sample_nodes(), &[], &settings)
            .unwrap();
        let first_contents = std::fs::read_to_string(&path).unwrap();

        let path2 = writer
            .write_config(&sample_nodes(), &sample_rules(), &settings)
            .unwrap();
        let second_contents = std::fs::read_to_string(&path2).unwrap();

        assert_ne!(first_contents, second_contents);
        assert!(second_contents.contains("geoip"));
    }

    #[test]
    fn test_write_config_error_on_empty_nodes() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let settings = AppSettings::default();

        let result = writer.write_config(&[], &[], &settings);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_config_rejects_invalid_proxy_node() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let settings = AppSettings::default();
        let nodes = vec![ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "example.com".into(),
            port: 8388,
            method: String::new(),
            password: "secret".into(),
            remark: None,
        })];

        let result = writer.write_config(&nodes, &[], &settings);
        assert!(matches!(result, Err(ConfigError::InvalidProxyNode(_))));
    }

    #[test]
    fn test_write_config_accepts_dns_protocols_that_fall_back_in_generators() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
        let mut settings = AppSettings::default();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "test".into(),
            protocol: DnsProtocol::Dot,
            address: "1.1.1.1".into(),
            port: None,
            detour: None,
        }];

        let result = writer.write_config(&sample_nodes(), &[], &settings);
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_creates_output_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("output");
        let writer = ConfigWriter::with_dir(nested.clone());
        let settings = AppSettings::default();

        let path = writer
            .write_config(&sample_nodes(), &[], &settings)
            .unwrap();

        assert!(nested.exists());
        assert!(path.exists());
    }

    #[test]
    fn test_config_writer_new_uses_user_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, dir.path());
        let mut settings = AppSettings::default();
        settings.backend.config_output_dir = Some(PathBuf::from("/custom/path"));

        let writer = ConfigWriter::new(&settings, &paths);
        assert_eq!(
            writer.output_path(BackendType::Xray),
            PathBuf::from("/custom/path/xray.json")
        );
    }

    #[test]
    fn test_config_writer_new_uses_default_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, dir.path());
        let settings = AppSettings::default();

        let writer = ConfigWriter::new(&settings, &paths);
        let expected = dir
            .path()
            .join("runtime")
            .join("generated")
            .join("xray.json");
        assert_eq!(writer.output_path(BackendType::Xray), expected);
    }

    #[test]
    fn test_full_pipeline_all_backends() {
        let dir = tempfile::TempDir::new().unwrap();
        let nodes = sample_nodes();
        let rules = sample_rules();

        for backend in [BackendType::V2ray, BackendType::Xray, BackendType::SingBox] {
            let writer = ConfigWriter::with_dir(dir.path().to_path_buf());
            let mut settings = AppSettings::default();
            settings.backend.backend_type = backend;

            let path = writer.write_config(&nodes, &rules, &settings).unwrap();
            assert!(path.exists());

            let contents = std::fs::read_to_string(&path).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();

            match backend {
                BackendType::V2ray | BackendType::Xray => {
                    assert!(parsed["routing"]["rules"].is_array());
                }
                BackendType::SingBox => {
                    assert!(parsed["route"]["rules"].is_array());
                }
            }
        }
    }
}
