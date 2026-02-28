//! Configuration types for fml.
//!
//! [`Config::load`] reads `~/.config/fml/config.toml`, creating it with
//! hardcoded defaults if it does not yet exist. [`Config::defaults`] returns
//! the same defaults without touching the filesystem (useful in tests).

use serde::Deserialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Embedded defaults
// ---------------------------------------------------------------------------

const DEFAULT_CONFIG: &str = r#"
[ui]
show_timestamps        = true
timestamp_format       = "%H:%M:%S%.3f"
producer_pane_width_pct = 25

[keybindings]
toggle_focus   = "Tab"
query_focus    = "/"
greed_up       = "]"
greed_down     = "["
yank_producer  = "y"
correlate      = "c"
export         = "e"
scroll_to_tail = "G"
"#;

// ---------------------------------------------------------------------------
// Public config types
// ---------------------------------------------------------------------------

/// Top-level application configuration, loaded from `~/.config/fml/config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub keybindings: KeybindingsConfig,
}

/// `[ui]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_show_timestamps")]
    pub show_timestamps: bool,
    #[serde(default = "default_timestamp_format")]
    pub timestamp_format: String,
    #[serde(default = "default_producer_pane_width_pct")]
    pub producer_pane_width_pct: u16,
}

fn default_show_timestamps() -> bool { true }
fn default_timestamp_format() -> String { "%H:%M:%S%.3f".to_string() }
fn default_producer_pane_width_pct() -> u16 { 25 }

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_timestamps: default_show_timestamps(),
            timestamp_format: default_timestamp_format(),
            producer_pane_width_pct: default_producer_pane_width_pct(),
        }
    }
}

/// `[keybindings]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct KeybindingsConfig {
    #[serde(default = "default_toggle_focus")]
    pub toggle_focus: String,
    #[serde(default = "default_query_focus")]
    pub query_focus: String,
    #[serde(default = "default_greed_up")]
    pub greed_up: String,
    #[serde(default = "default_greed_down")]
    pub greed_down: String,
    #[serde(default = "default_yank_producer")]
    pub yank_producer: String,
    #[serde(default = "default_correlate")]
    pub correlate: String,
    #[serde(default = "default_export")]
    pub export: String,
    #[serde(default = "default_scroll_to_tail")]
    pub scroll_to_tail: String,
}

fn default_toggle_focus() -> String { "Tab".to_string() }
fn default_query_focus() -> String { "/".to_string() }
fn default_greed_up() -> String { "]".to_string() }
fn default_greed_down() -> String { "[".to_string() }
fn default_yank_producer() -> String { "y".to_string() }
fn default_correlate() -> String { "c".to_string() }
fn default_export() -> String { "e".to_string() }
fn default_scroll_to_tail() -> String { "G".to_string() }

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            toggle_focus: default_toggle_focus(),
            query_focus: default_query_focus(),
            greed_up: default_greed_up(),
            greed_down: default_greed_down(),
            yank_producer: default_yank_producer(),
            correlate: default_correlate(),
            export: default_export(),
            scroll_to_tail: default_scroll_to_tail(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Config {
    /// Load from `~/.config/fml/config.toml`, layered on top of the built-in
    /// defaults. Creates the file with defaults if it does not exist.
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path();

        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, DEFAULT_CONFIG.trim_start())?;
        }

        config::Config::builder()
            .add_source(config::File::from_str(DEFAULT_CONFIG, config::FileFormat::Toml))
            .add_source(config::File::from(path.as_path()).required(false))
            .build()?
            .try_deserialize()
            .map_err(Into::into)
    }

    /// Return the built-in defaults without touching the filesystem.
    pub fn defaults() -> Self {
        config::Config::builder()
            .add_source(config::File::from_str(DEFAULT_CONFIG, config::FileFormat::Toml))
            .build()
            .expect("built-in default config must be valid TOML")
            .try_deserialize()
            .expect("built-in default config must deserialize correctly")
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn config_path() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
                .join(".config")
        })
        .join("fml")
        .join("config.toml")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load() {
        let cfg = Config::defaults();
        assert!(cfg.ui.show_timestamps);
        assert_eq!(cfg.ui.producer_pane_width_pct, 25);
        assert_eq!(cfg.keybindings.query_focus, "/");
        assert_eq!(cfg.keybindings.greed_up, "]");
    }
}
