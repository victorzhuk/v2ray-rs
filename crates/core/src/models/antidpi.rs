use serde::{Deserialize, Serialize};

use super::validation::ValidationError;

/// Upper bound on the strategy list. The helper rotates the chains it is
/// given; more than a handful of concurrent strategies only burns CPU in the
/// NFQUEUE loop.
pub const MAX_ANTIDPI_STRATEGIES: usize = 16;

/// Upper bound on one strategy chain string, counted as Unicode scalar
/// characters rather than UTF-8 bytes. The helper's own parser rejects
/// anything longer long before this; the cap here keeps a hostile settings
/// file from smuggling megabytes into every config load.
pub const MAX_ANTIDPI_STRATEGY_LEN: usize = 512;

/// Default NFQUEUE number. Matches `fw::DEFAULT_QUEUE` in the helper so a
/// settings file written by hand without a queue binds the queue the helper
/// documents.
pub const DEFAULT_ANTIDPI_QUEUE: u16 = 200;

/// Anti-DPI engine settings (`[antidpi]` section of `settings.toml`).
///
/// An empty `strategies` list delegates to the helper's built-in default
/// chains (`DEFAULT_CHAINS` in `crates/antidpi/src/main.rs`, the
/// keenetic-default chains documented in `docs/antidpi-registry.md` §5): the
/// process layer then omits the `--strategy` flags entirely, so a helper
/// update can refresh its defaults without a settings migration. The strings
/// themselves are strategy chains in the helper's DSL; core only shape-checks
/// them because the dependency direction forbids core → antidpi — the helper's
/// `check` subcommand is the real validator, driven from the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "AntiDpiConfigWire")]
pub struct AntiDpiConfig {
    pub enabled: bool,
    pub queue: u16,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostlist_path: Option<String>,
}

impl Default for AntiDpiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            queue: DEFAULT_ANTIDPI_QUEUE,
            strategies: Vec::new(),
            hostlist_path: None,
        }
    }
}

impl AntiDpiConfig {
    /// Shape-only validation: the strategy DSL belongs to the helper, which
    /// the UI consults via `check --strategy`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.queue == 0 {
            return Err(ValidationError::InvalidAntiDpiConfig(format!(
                "queue must be between 1 and 65535, got {}",
                self.queue
            )));
        }
        if self.strategies.len() > MAX_ANTIDPI_STRATEGIES {
            return Err(ValidationError::InvalidAntiDpiConfig(format!(
                "at most {MAX_ANTIDPI_STRATEGIES} strategies, got {}",
                self.strategies.len()
            )));
        }
        for strategy in &self.strategies {
            if strategy.is_empty() {
                return Err(ValidationError::InvalidAntiDpiConfig(
                    "strategy chains must not be empty".to_string(),
                ));
            }
            if strategy.chars().count() > MAX_ANTIDPI_STRATEGY_LEN {
                return Err(ValidationError::InvalidAntiDpiConfig(format!(
                    "strategy chain longer than {MAX_ANTIDPI_STRATEGY_LEN} characters"
                )));
            }
        }
        if let Some(path) = &self.hostlist_path
            && path.trim().is_empty()
        {
            return Err(ValidationError::InvalidAntiDpiConfig(
                "hostlist path must not be blank when set".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AntiDpiConfigWire {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_queue")]
    queue: u16,
    #[serde(default)]
    strategies: Vec<String>,
    #[serde(default)]
    hostlist_path: Option<String>,
}

fn default_queue() -> u16 {
    DEFAULT_ANTIDPI_QUEUE
}

impl From<AntiDpiConfigWire> for AntiDpiConfig {
    fn from(wire: AntiDpiConfigWire) -> Self {
        Self {
            enabled: wire.enabled,
            queue: wire.queue,
            strategies: wire.strategies,
            hostlist_path: wire.hostlist_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_disabled_with_documented_values() {
        let antidpi = AntiDpiConfig::default();
        assert!(!antidpi.enabled);
        assert_eq!(antidpi.queue, 200);
        assert!(antidpi.strategies.is_empty());
        assert!(antidpi.hostlist_path.is_none());
    }

    #[test]
    fn test_legacy_section_missing_loads_defaults() {
        let antidpi: AntiDpiConfig = toml::from_str("").unwrap();
        assert_eq!(antidpi, AntiDpiConfig::default());

        let partial: AntiDpiConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.queue, 200);
        assert!(partial.strategies.is_empty());
        assert!(partial.hostlist_path.is_none());
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let antidpi: AntiDpiConfig =
            toml::from_str("enabled = true\nfuture_field = \"whatever\"\n").unwrap();
        assert!(antidpi.enabled);
    }

    #[test]
    fn test_round_trip() {
        let antidpi = AntiDpiConfig {
            enabled: true,
            queue: 5,
            strategies: vec![
                "fake:blob=tls:tls_mod=rnd,dupsid,sni=fonts.google.com:tcp_seq=10000".to_string(),
                "multisplit:pos=2,midsld:seqovl=1".to_string(),
            ],
            hostlist_path: Some("/etc/v2ray-rs/hosts.txt".to_string()),
        };
        let toml_str = toml::to_string(&antidpi).unwrap();
        let parsed: AntiDpiConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(antidpi, parsed);
    }

    #[test]
    fn test_round_trip_preserves_empty_strategies() {
        let antidpi = AntiDpiConfig {
            enabled: false,
            queue: 200,
            strategies: Vec::new(),
            hostlist_path: None,
        };
        let toml_str = toml::to_string(&antidpi).unwrap();
        let parsed: AntiDpiConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(antidpi, parsed);
    }

    #[test]
    fn test_validate_accepts_defaults() {
        assert_eq!(AntiDpiConfig::default().validate(), Ok(()));
    }

    #[test]
    fn test_validate_rejects_queue_zero() {
        let antidpi = AntiDpiConfig {
            queue: 0,
            ..AntiDpiConfig::default()
        };
        assert_eq!(
            antidpi.validate(),
            Err(ValidationError::InvalidAntiDpiConfig(
                "queue must be between 1 and 65535, got 0".to_string()
            ))
        );
    }

    #[test]
    fn test_validate_caps_strategy_count() {
        let strategies = (0..MAX_ANTIDPI_STRATEGIES)
            .map(|i| format!("multisplit:pos={i}"))
            .collect::<Vec<_>>();
        let at_cap = AntiDpiConfig {
            strategies: strategies.clone(),
            ..AntiDpiConfig::default()
        };
        assert_eq!(at_cap.validate(), Ok(()));

        let mut over_cap = strategies;
        over_cap.push("multisplit:pos=99".to_string());
        let antidpi = AntiDpiConfig {
            strategies: over_cap,
            ..AntiDpiConfig::default()
        };
        assert_eq!(
            antidpi.validate(),
            Err(ValidationError::InvalidAntiDpiConfig(format!(
                "at most {MAX_ANTIDPI_STRATEGIES} strategies, got {}",
                MAX_ANTIDPI_STRATEGIES + 1
            )))
        );
    }

    #[test]
    fn test_validate_rejects_empty_and_oversized_strategies() {
        let empty = AntiDpiConfig {
            strategies: vec![String::new()],
            ..AntiDpiConfig::default()
        };
        assert_eq!(
            empty.validate(),
            Err(ValidationError::InvalidAntiDpiConfig(
                "strategy chains must not be empty".to_string()
            ))
        );

        let oversized = AntiDpiConfig {
            strategies: vec!["x".repeat(MAX_ANTIDPI_STRATEGY_LEN + 1)],
            ..AntiDpiConfig::default()
        };
        assert_eq!(
            oversized.validate(),
            Err(ValidationError::InvalidAntiDpiConfig(format!(
                "strategy chain longer than {MAX_ANTIDPI_STRATEGY_LEN} characters"
            )))
        );

        let at_cap = AntiDpiConfig {
            strategies: vec!["x".repeat(MAX_ANTIDPI_STRATEGY_LEN)],
            ..AntiDpiConfig::default()
        };
        assert_eq!(at_cap.validate(), Ok(()));
    }

    #[test]
    fn test_validate_counts_unicode_characters_not_bytes() {
        let chars_over = "х".repeat(MAX_ANTIDPI_STRATEGY_LEN + 1);
        assert_eq!(chars_over.chars().count(), MAX_ANTIDPI_STRATEGY_LEN + 1);
        assert_eq!(chars_over.len(), 2 * (MAX_ANTIDPI_STRATEGY_LEN + 1));
        let over = AntiDpiConfig {
            strategies: vec![chars_over],
            ..AntiDpiConfig::default()
        };
        assert!(over.validate().is_err());

        let chars_at_cap = "х".repeat(MAX_ANTIDPI_STRATEGY_LEN);
        assert_eq!(chars_at_cap.len(), 2 * MAX_ANTIDPI_STRATEGY_LEN);
        let at_cap = AntiDpiConfig {
            strategies: vec![chars_at_cap],
            ..AntiDpiConfig::default()
        };
        assert_eq!(at_cap.validate(), Ok(()));
    }

    #[test]
    fn test_validate_rejects_blank_hostlist_path() {
        for blank in [Some(String::new()), Some("   ".to_string())] {
            let antidpi = AntiDpiConfig {
                hostlist_path: blank,
                ..AntiDpiConfig::default()
            };
            assert_eq!(
                antidpi.validate(),
                Err(ValidationError::InvalidAntiDpiConfig(
                    "hostlist path must not be blank when set".to_string()
                ))
            );
        }

        let set = AntiDpiConfig {
            hostlist_path: Some("/etc/v2ray-rs/hosts.txt".to_string()),
            ..AntiDpiConfig::default()
        };
        assert_eq!(set.validate(), Ok(()));
    }
}
