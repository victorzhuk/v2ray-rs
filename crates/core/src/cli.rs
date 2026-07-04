use std::path::PathBuf;

use crate::profile::Env;

/// Directory overrides parsed from the CLI, before merging with the
/// environment. The clap parsing surface is an application-entrypoint concern
/// and lives in the ui binary; this crate only needs the resolved values.
#[derive(Debug, Default)]
pub struct CliPaths {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub install_icons: bool,
}

#[derive(Debug, Default)]
pub struct PathOverrides {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub install_icons: Option<bool>,
}

impl PathOverrides {
    pub fn resolve(cli: &CliPaths, env: &dyn Env) -> Self {
        Self {
            config_dir: cli
                .config_dir
                .clone()
                .or_else(|| env.get("V2RAY_RS_CONFIG_DIR").map(PathBuf::from)),
            data_dir: cli
                .data_dir
                .clone()
                .or_else(|| env.get("V2RAY_RS_DATA_DIR").map(PathBuf::from)),
            cache_dir: cli
                .cache_dir
                .clone()
                .or_else(|| env.get("V2RAY_RS_CACHE_DIR").map(PathBuf::from)),
            runtime_dir: cli
                .runtime_dir
                .clone()
                .or_else(|| env.get("V2RAY_RS_RUNTIME_DIR").map(PathBuf::from)),
            state_dir: cli
                .state_dir
                .clone()
                .or_else(|| env.get("V2RAY_RS_STATE_DIR").map(PathBuf::from)),
            install_icons: Some(cli.install_icons).filter(|&v| v).or_else(|| {
                env.get("V2RAY_RS_INSTALL_ICONS")
                    .and_then(|v| parse_bool(&v))
            }),
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
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
    fn test_resolve_cli_takes_precedence() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_CONFIG_DIR", "/from/env");
        env.set("V2RAY_RS_DATA_DIR", "/from/env/data");
        env.set("V2RAY_RS_CACHE_DIR", "/from/env/cache");
        env.set("V2RAY_RS_RUNTIME_DIR", "/from/env/runtime");
        env.set("V2RAY_RS_STATE_DIR", "/from/env/state");
        env.set("V2RAY_RS_INSTALL_ICONS", "true");

        let cli = CliPaths {
            config_dir: Some(PathBuf::from("/from/cli")),
            data_dir: Some(PathBuf::from("/from/cli/data")),
            cache_dir: Some(PathBuf::from("/from/cli/cache")),
            runtime_dir: Some(PathBuf::from("/from/cli/runtime")),
            state_dir: Some(PathBuf::from("/from/cli/state")),
            install_icons: true,
        };

        let overrides = PathOverrides::resolve(&cli, &env);

        assert_eq!(overrides.config_dir, Some(PathBuf::from("/from/cli")));
        assert_eq!(overrides.data_dir, Some(PathBuf::from("/from/cli/data")));
        assert_eq!(overrides.cache_dir, Some(PathBuf::from("/from/cli/cache")));
        assert_eq!(
            overrides.runtime_dir,
            Some(PathBuf::from("/from/cli/runtime"))
        );
        assert_eq!(overrides.state_dir, Some(PathBuf::from("/from/cli/state")));
        assert_eq!(overrides.install_icons, Some(true));
    }

    #[test]
    fn test_resolve_env_vars() {
        let mut env = MockEnv::new();
        env.set("V2RAY_RS_CONFIG_DIR", "/from/env");
        env.set("V2RAY_RS_DATA_DIR", "/from/env/data");
        env.set("V2RAY_RS_CACHE_DIR", "/from/env/cache");
        env.set("V2RAY_RS_RUNTIME_DIR", "/from/env/runtime");
        env.set("V2RAY_RS_STATE_DIR", "/from/env/state");
        env.set("V2RAY_RS_INSTALL_ICONS", "true");

        let cli = CliPaths {
            config_dir: None,
            data_dir: None,
            cache_dir: None,
            runtime_dir: None,
            state_dir: None,
            install_icons: false,
        };

        let overrides = PathOverrides::resolve(&cli, &env);

        assert_eq!(overrides.config_dir, Some(PathBuf::from("/from/env")));
        assert_eq!(overrides.data_dir, Some(PathBuf::from("/from/env/data")));
        assert_eq!(overrides.cache_dir, Some(PathBuf::from("/from/env/cache")));
        assert_eq!(
            overrides.runtime_dir,
            Some(PathBuf::from("/from/env/runtime"))
        );
        assert_eq!(overrides.state_dir, Some(PathBuf::from("/from/env/state")));
        assert_eq!(overrides.install_icons, Some(true));
    }

    #[test]
    fn test_resolve_none_when_empty() {
        let env = MockEnv::new();
        let cli = CliPaths {
            config_dir: None,
            data_dir: None,
            cache_dir: None,
            runtime_dir: None,
            state_dir: None,
            install_icons: false,
        };

        let overrides = PathOverrides::resolve(&cli, &env);

        assert!(overrides.config_dir.is_none());
        assert!(overrides.data_dir.is_none());
        assert!(overrides.cache_dir.is_none());
        assert!(overrides.runtime_dir.is_none());
        assert!(overrides.state_dir.is_none());
        assert!(overrides.install_icons.is_none());
    }

    #[test]
    fn test_parse_bool_valid() {
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("YES"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("ON"), Some(true));

        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("FALSE"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("NO"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("OFF"), Some(false));
    }

    #[test]
    fn test_parse_bool_invalid() {
        assert_eq!(parse_bool("invalid"), None);
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn test_cli_install_icons_flag() {
        let env = MockEnv::new();

        let cli = CliPaths {
            config_dir: None,
            data_dir: None,
            cache_dir: None,
            runtime_dir: None,
            state_dir: None,
            install_icons: true,
        };

        let overrides = PathOverrides::resolve(&cli, &env);
        assert_eq!(overrides.install_icons, Some(true));
    }
}
