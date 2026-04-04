use std::env;

use config::{Environment, File};
use serde::{Deserialize, Serialize};

use crate::error::FmlError;

pub mod themes;
pub mod tui;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Config {
    /// User-level config directory used to look up `config.toml`.
    ///
    /// The loader bootstraps from the XDG/HOME-derived default path first. If
    /// this field is overridden from the local project config or environment,
    /// configuration is loaded a second time using the configured directory.
    #[serde(default = "default_config_dir")]
    pub config_dir: String,

    /// Master switch for debug/file logging.
    ///
    /// When `false` (the default), no log file is created. The `--debug`
    /// CLI flag overrides this to `true`.
    #[serde(default)]
    pub enable_logging: bool,

    /// Directory where debug log files are written.
    ///
    /// Only used when `enable_logging` is `true`.
    #[serde(default = "default_debug_log_dir")]
    pub debug_log_dir: String,

    /// Fallback log level when the `RUST_LOG` environment variable is unset.
    ///
    /// Accepts any valid `tracing` filter string (e.g. `"info"`, `"debug"`,
    /// `"fml=trace"`).
    #[serde(default = "default_log_level")]
    pub default_log_level: String,

    /// TUI rendering and interaction settings.
    #[serde(default)]
    pub tui: tui::TuiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_dir: default_config_dir(),
            enable_logging: false,
            debug_log_dir: default_debug_log_dir(),
            default_log_level: default_log_level(),
            tui: tui::TuiConfig::default(),
        }
    }
}
pub fn default_config_dir() -> String {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        return format!("{xdg}/fml");
    }

    env::var("HOME")
        .map(|home| format!("{home}/.config/fml"))
        .unwrap_or_else(|_| ".config/fml".to_string())
}

fn default_debug_log_dir() -> String {
    "/tmp".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    /// Loads configuration from multiple sources, merged lowest → highest priority:
    ///
    /// 1. User-level config: `$XDG_CONFIG_HOME/fml/config` (or `$HOME/.config/fml/config`)
    /// 2. Local project config: `.config/fml/config` (relative to working directory)
    /// 3. Environment variables: `FML_*` (use `__` for nested keys, e.g. `FML_TUI__TICK_RATE`)
    ///
    /// All sources are optional. When no config is found, serde defaults produce a
    /// valid [`FmlConfig`].
    pub fn new() -> Result<Self, FmlError> {
        let bootstrap_dir = default_config_dir();
        let config = Self::load_with_config_dir(&bootstrap_dir)?;

        if config.config_dir == bootstrap_dir {
            return Ok(config);
        }

        Self::load_with_config_dir(&config.config_dir)
    }

    fn load_with_config_dir(config_dir: &str) -> Result<Self, FmlError> {
        let s = config::Config::builder()
            .add_source(File::with_name(&format!("{config_dir}/config")).required(false))
            .add_source(File::with_name(".config/fml/config").required(false))
            .add_source(Environment::with_prefix("FML").separator("__"))
            .build()?;

        let config: Self = s.try_deserialize()?;

        Ok(config)
    }
}
