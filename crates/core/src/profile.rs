use std::env;

use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ProfileError {
    #[error("invalid custom profile name: {name}")]
    InvalidCustomName { name: String },
    #[error("parse error: {reason} (input: {input})")]
    Parse { input: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppProfile {
    Production,
    Development,
    Test,
    Custom(String),
}

impl AppProfile {
    pub fn qualifier(&self) -> String {
        match self {
            AppProfile::Production => "v2ray-rs".to_string(),
            AppProfile::Development => "v2ray-rs-dev".to_string(),
            AppProfile::Test => "v2ray-rs-test".to_string(),
            AppProfile::Custom(name) => format!("v2ray-rs-{name}"),
        }
    }

    pub fn app_id(&self) -> String {
        match self {
            AppProfile::Production => "com.github.v2ray-rs".to_string(),
            AppProfile::Development => "com.github.v2ray-rs.dev".to_string(),
            AppProfile::Test => "com.github.v2ray-rs.test".to_string(),
            AppProfile::Custom(name) => format!("com.github.v2ray-rs.{name}"),
        }
    }

    pub fn parse(input: &str) -> Result<Self, ProfileError> {
        let lower = input.to_lowercase();
        match lower.as_str() {
            "production" => Ok(AppProfile::Production),
            "development" => Ok(AppProfile::Development),
            "test" => Ok(AppProfile::Test),
            _ => {
                validate_custom_name(input)?;
                Ok(AppProfile::Custom(input.to_string()))
            }
        }
    }

    pub fn resolve(cli: Option<&str>, env: &dyn Env) -> Result<Self, ProfileError> {
        if let Some(cli_profile) = cli {
            return Self::parse(cli_profile);
        }

        if let Some(env_val) = env.get("V2RAY_RS_PROFILE") {
            return Self::parse(&env_val);
        }

        if let Some(dev_val) = env.get("V2RAY_RS_DEV") {
            if !dev_val.is_empty() {
                log::warn!(
                    "V2RAY_RS_DEV is deprecated, use V2RAY_RS_PROFILE=development instead; will be removed in 0.8.0"
                );
                return Ok(AppProfile::Development);
            }
        }

        let default = if cfg!(debug_assertions) {
            AppProfile::Development
        } else {
            AppProfile::Production
        };

        Ok(default)
    }
}

fn validate_custom_name(name: &str) -> Result<(), ProfileError> {
    if name.is_empty() {
        return Err(ProfileError::InvalidCustomName {
            name: name.to_string(),
        });
    }

    let mut chars = name.chars();

    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(ProfileError::InvalidCustomName {
            name: name.to_string(),
        });
    }

    let mut count = 1;
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' && c != '-' {
            return Err(ProfileError::InvalidCustomName {
                name: name.to_string(),
            });
        }
        count += 1;
        if count > 31 {
            return Err(ProfileError::InvalidCustomName {
                name: name.to_string(),
            });
        }
    }

    Ok(())
}

pub trait Env {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct StdEnv;

impl Env for StdEnv {
    fn get(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockEnv {
        vars: std::collections::HashMap<String, String>,
    }

    impl MockEnv {
        fn new() -> Self {
            Self {
                vars: std::collections::HashMap::new(),
            }
        }

        fn set(&mut self, key: &str, value: &str) {
            self.vars.insert(key.to_string(), value.to_string());
        }
    }

    impl Env for MockEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    #[test]
    fn test_qualifier() {
        assert_eq!(AppProfile::Production.qualifier(), "v2ray-rs");
        assert_eq!(AppProfile::Development.qualifier(), "v2ray-rs-dev");
        assert_eq!(AppProfile::Test.qualifier(), "v2ray-rs-test");
        assert_eq!(
            AppProfile::Custom("qa".to_string()).qualifier(),
            "v2ray-rs-qa"
        );
    }

    #[test]
    fn test_app_id() {
        assert_eq!(AppProfile::Production.app_id(), "com.github.v2ray-rs");
        assert_eq!(
            AppProfile::Development.app_id(),
            "com.github.v2ray-rs.dev"
        );
        assert_eq!(AppProfile::Test.app_id(), "com.github.v2ray-rs.test");
        assert_eq!(
            AppProfile::Custom("qa".to_string()).app_id(),
            "com.github.v2ray-rs.qa"
        );
    }

    #[test]
    fn test_parse_standard() {
        assert_eq!(
            AppProfile::parse("production").unwrap(),
            AppProfile::Production
        );
        assert_eq!(
            AppProfile::parse("PRODUCTION").unwrap(),
            AppProfile::Production
        );
        assert_eq!(
            AppProfile::parse("Production").unwrap(),
            AppProfile::Production
        );
        assert_eq!(
            AppProfile::parse("development").unwrap(),
            AppProfile::Development
        );
        assert_eq!(
            AppProfile::parse("test").unwrap(),
            AppProfile::Test
        );
    }

    #[test]
    fn test_parse_custom_valid() {
        assert_eq!(
            AppProfile::parse("qa").unwrap(),
            AppProfile::Custom("qa".to_string())
        );
        assert_eq!(
            AppProfile::parse("my-profile").unwrap(),
            AppProfile::Custom("my-profile".to_string())
        );
        assert_eq!(
            AppProfile::parse("test-env").unwrap(),
            AppProfile::Custom("test-env".to_string())
        );
        assert_eq!(
            AppProfile::parse("a").unwrap(),
            AppProfile::Custom("a".to_string())
        );
        assert_eq!(
            AppProfile::parse("9").unwrap(),
            AppProfile::Custom("9".to_string())
        );
    }

    #[test]
    fn test_parse_custom_invalid() {
        let too_long = "a".repeat(32);
        let invalid_names = vec![
            "Bad Name!",
            "",
            &too_long,
            "UPPERCASE",
            "-starts-with-dash",
            "_starts-with-underscore",
            "has spaces",
            "has.dot",
            "has@special",
        ];

        for name in invalid_names {
            let result = AppProfile::parse(name);
            assert!(result.is_err(), "should reject: {}", name);
        }
    }

    #[test]
    fn test_resolve_cli_takes_precedence() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_PROFILE", "development");
        env.set("V2RAY_RS_DEV", "1");

        let result = AppProfile::resolve(Some("test"), &env);
        assert_eq!(result.unwrap(), AppProfile::Test);
    }

    #[test]
    fn test_resolve_env_profile() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_PROFILE", "test");
        env.set("V2RAY_RS_DEV", "1");

        let result = AppProfile::resolve(None, &env);
        assert_eq!(result.unwrap(), AppProfile::Test);
    }

    #[test]
    fn test_resolve_env_dev_deprecated() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_DEV", "1");

        let result = AppProfile::resolve(None, &env);
        assert_eq!(result.unwrap(), AppProfile::Development);
    }

    #[test]
    fn test_resolve_default_debug() {
        let env = MockEnv::new();

        let result = AppProfile::resolve(None, &env);
        if cfg!(debug_assertions) {
            assert_eq!(result.unwrap(), AppProfile::Development);
        } else {
            assert_eq!(result.unwrap(), AppProfile::Production);
        }
    }

    #[test]
    fn test_resolve_empty_dev_env() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_DEV", "");

        let result = AppProfile::resolve(None, &env);
        if cfg!(debug_assertions) {
            assert_eq!(result.unwrap(), AppProfile::Development);
        } else {
            assert_eq!(result.unwrap(), AppProfile::Production);
        }
    }

    #[test]
    fn test_resolve_custom_from_cli() {
        let env = MockEnv::new();

        let result = AppProfile::resolve(Some("qa"), &env);
        assert_eq!(result.unwrap(), AppProfile::Custom("qa".to_string()));
    }

    #[test]
    fn test_resolve_custom_from_env() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_PROFILE", "staging");

        let result = AppProfile::resolve(None, &env);
        assert_eq!(
            result.unwrap(),
            AppProfile::Custom("staging".to_string())
        );
    }

    #[test]
    fn test_validate_custom_name_edge_cases() {
        assert!(validate_custom_name("a").is_ok());
        assert!(validate_custom_name("9").is_ok());
        assert!(validate_custom_name("a".repeat(31).as_str()).is_ok());
        assert!(validate_custom_name("test-123_profile").is_ok());

        assert!(validate_custom_name("").is_err());
        assert!(validate_custom_name("A").is_err());
        assert!(validate_custom_name("-a").is_err());
        assert!(validate_custom_name("_a").is_err());
        assert!(validate_custom_name("a b").is_err());
        assert!(validate_custom_name("a.b").is_err());
        assert!(validate_custom_name("a".repeat(32).as_str()).is_err());
        assert!(validate_custom_name("a".repeat(100).as_str()).is_err());
    }
}
