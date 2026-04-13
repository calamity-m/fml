use std::env;

use config::{Environment, File};
use serde::{Deserialize, Serialize};

use crate::error::FmlError;

pub mod search;
pub mod store;
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

    /// Store capacity and ingest speed settings
    #[serde(default)]
    pub store: store::StoreConfig,

    /// Search config and settings
    #[serde(default)]
    pub search: search::SearchConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_dir: default_config_dir(),
            enable_logging: false,
            debug_log_dir: default_debug_log_dir(),
            default_log_level: default_log_level(),
            tui: tui::TuiConfig::default(),
            store: store::StoreConfig::default(),
            search: search::SearchConfig::default(),
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
    /// 3. Environment variables: `FML__*` (use `__` as separator, e.g. `FML__TUI__TICK_RATE`)
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

#[cfg(test)]
mod tests {
    use std::fs;

    use serial_test::serial;

    use super::*;

    // --- default_config_dir ---

    #[test]
    #[serial]
    fn default_config_dir_prefers_xdg() {
        let orig_xdg = env::var("XDG_CONFIG_HOME").ok();
        let orig_home = env::var("HOME").ok();

        unsafe {
            env::set_var("XDG_CONFIG_HOME", "/custom/xdg");
            env::set_var("HOME", "/home/someone");
        }

        let result = default_config_dir();

        // Restore
        match orig_xdg {
            Some(v) => unsafe { env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
        }
        match orig_home {
            Some(v) => unsafe { env::set_var("HOME", v) },
            None => unsafe { env::remove_var("HOME") },
        }

        assert_eq!(result, "/custom/xdg/fml");
    }

    #[test]
    #[serial]
    fn default_config_dir_falls_back_to_home() {
        let orig_xdg = env::var("XDG_CONFIG_HOME").ok();
        let orig_home = env::var("HOME").ok();

        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
            env::set_var("HOME", "/home/someone");
        }

        let result = default_config_dir();

        match orig_xdg {
            Some(v) => unsafe { env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
        }
        match orig_home {
            Some(v) => unsafe { env::set_var("HOME", v) },
            None => unsafe { env::remove_var("HOME") },
        }

        assert_eq!(result, "/home/someone/.config/fml");
    }

    #[test]
    #[serial]
    fn default_config_dir_falls_back_to_relative() {
        let orig_xdg = env::var("XDG_CONFIG_HOME").ok();
        let orig_home = env::var("HOME").ok();

        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("HOME");
        }

        let result = default_config_dir();

        match orig_xdg {
            Some(v) => unsafe { env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
        }
        match orig_home {
            Some(v) => unsafe { env::set_var("HOME", v) },
            None => unsafe { env::remove_var("HOME") },
        }

        assert_eq!(result, ".config/fml");
    }

    // --- load_with_config_dir ---

    #[test]
    #[serial]
    fn load_with_missing_dir_returns_defaults() {
        let config = Config::load_with_config_dir("/nonexistent/path/that/wont/exist").unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    #[serial]
    fn load_with_config_dir_reads_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
            enable_logging = true
            default_log_level = "debug"
            "#,
        )
        .unwrap();

        let config = Config::load_with_config_dir(dir.path().to_str().unwrap()).unwrap();
        assert!(config.enable_logging);
        assert_eq!(config.default_log_level, "debug");
    }

    #[test]
    #[serial]
    fn load_with_config_dir_env_overrides_file() {
        // The config crate with separator("__") uses __ to delimit the prefix
        // from the key as well, so env vars must be FML__<key>, not FML_<key>.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, r#"default_log_level = "debug""#).unwrap();

        let orig = env::var("FML__DEFAULT_LOG_LEVEL").ok();
        unsafe { env::set_var("FML__DEFAULT_LOG_LEVEL", "trace") };

        let config = Config::load_with_config_dir(dir.path().to_str().unwrap()).unwrap();

        match orig {
            Some(v) => unsafe { env::set_var("FML__DEFAULT_LOG_LEVEL", v) },
            None => unsafe { env::remove_var("FML__DEFAULT_LOG_LEVEL") },
        }

        assert_eq!(config.default_log_level, "trace");
    }

    // --- new ---

    #[test]
    #[serial]
    fn new_loads_from_default_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("fml");
        fs::create_dir_all(&config_path).unwrap();
        fs::write(
            config_path.join("config.toml"),
            r#"enable_logging = true"#,
        )
        .unwrap();

        let orig_xdg = env::var("XDG_CONFIG_HOME").ok();
        let orig_fml = env::var("FML_ENABLE_LOGGING").ok();
        unsafe {
            env::set_var("XDG_CONFIG_HOME", dir.path().to_str().unwrap());
            env::remove_var("FML_ENABLE_LOGGING");
        }

        let config = Config::new().unwrap();

        match orig_xdg {
            Some(v) => unsafe { env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
        }
        match orig_fml {
            Some(v) => unsafe { env::set_var("FML_ENABLE_LOGGING", v) },
            None => unsafe { env::remove_var("FML_ENABLE_LOGGING") },
        }

        assert!(config.enable_logging);
    }

    #[test]
    #[serial]
    fn new_with_no_config_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();

        let orig_xdg = env::var("XDG_CONFIG_HOME").ok();
        let orig_home = env::var("HOME").ok();
        let orig_vars: Vec<_> = ["FML_ENABLE_LOGGING", "FML_DEFAULT_LOG_LEVEL", "FML_DEBUG_LOG_DIR", "FML_CONFIG_DIR"]
            .iter()
            .map(|k| (*k, env::var(k).ok()))
            .collect();

        unsafe {
            env::set_var("XDG_CONFIG_HOME", dir.path().to_str().unwrap());
            env::set_var("HOME", dir.path().to_str().unwrap());
            for (k, _) in &orig_vars {
                env::remove_var(k);
            }
        }

        let config = Config::new().unwrap();

        match orig_xdg {
            Some(v) => unsafe { env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
        }
        match orig_home {
            Some(v) => unsafe { env::set_var("HOME", v) },
            None => unsafe { env::remove_var("HOME") },
        }
        for (k, v) in &orig_vars {
            match v {
                Some(val) => unsafe { env::set_var(k, val) },
                None => unsafe { env::remove_var(k) },
            }
        }

        assert!(!config.enable_logging);
        assert_eq!(config.default_log_level, "info");
    }
}
