//! Colour theme for the fml TUI.
//!
//! Themes are defined as TOML files. The default theme is embedded in the
//! binary via [`include_str!`] so the application works without any files on
//! disk. Call [`Theme::load_default`] at startup and pass the result through
//! the application as a shared reference.
//!
//! # Colour assignment for producers
//!
//! Producer names are hashed to a stable index into the palette so the same
//! pod/container/file always gets the same colour within a session, regardless
//! of the order in which producers appear.

use config::{Config, File, FileFormat};
use fml_core::LogLevel;
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

const DEFAULT_THEME_SRC: &str = include_str!("themes/default.toml");
const GRUVBOX_DARK_THEME_SRC: &str = include_str!("themes/gruvbox_dark.toml");

// ---------------------------------------------------------------------------
// Raw (serde) types — mirror the TOML structure
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawStyle {
    fg: Option<String>,
    bg: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    dim: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underlined: bool,
}

impl RawStyle {
    fn into_style(self) -> Style {
        let mut style = Style::default();
        if let Some(ref s) = self.fg {
            if let Some(c) = parse_color(s) {
                style = style.fg(c);
            }
        }
        if let Some(ref s) = self.bg {
            if let Some(c) = parse_color(s) {
                style = style.bg(c);
            }
        }
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.dim {
            style = style.add_modifier(Modifier::DIM);
        }
        if self.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.underlined {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        style
    }
}

#[derive(Debug, Deserialize)]
struct RawLevels {
    trace: RawStyle,
    debug: RawStyle,
    info: RawStyle,
    warn: RawStyle,
    error: RawStyle,
    fatal: RawStyle,
}

#[derive(Debug, Deserialize)]
struct RawBorders {
    focused: RawStyle,
    command_bar: RawStyle,
    unfocused: RawStyle,
}

#[derive(Debug, Deserialize)]
struct RawSearch {
    highlight: RawStyle,
}

#[derive(Debug, Deserialize)]
struct RawProducers {
    palette: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawTheme {
    levels: RawLevels,
    borders: RawBorders,
    search: RawSearch,
    producers: RawProducers,
}

// ---------------------------------------------------------------------------
// Public Theme type
// ---------------------------------------------------------------------------

/// Application colour theme.
///
/// Load once at startup with [`Theme::load_default`] and pass as a shared
/// reference throughout the TUI. All styles are pre-resolved ratatui [`Style`]
/// values — no allocation at render time.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Styles for each log level severity.
    pub level_trace: Style,
    pub level_debug: Style,
    pub level_info: Style,
    pub level_warn: Style,
    pub level_error: Style,
    pub level_fatal: Style,

    /// Border style for the currently focused pane.
    pub border_focused: Style,
    /// Border style for the command bar pane
    pub border_command_bar: Style,
    /// Border style for unfocused panes.
    pub border_unfocused: Style,

    /// Inline highlight applied to matched search spans.
    pub search_highlight: Style,

    /// Ordered colour palette used for producer colour cycling.
    producer_palette: Vec<Color>,
}

impl Theme {
    /// Load and parse the embedded default theme.
    ///
    /// # Panics
    ///
    /// Panics if the embedded TOML is malformed. The default theme is
    /// validated at compile time via `include_str!`, so this should never
    /// happen in practice.
    pub fn load_default() -> Self {
        Self::from_toml_str(DEFAULT_THEME_SRC).expect("embedded default theme must be valid TOML")
    }

    /// Load and parse the embedded Gruvbox Dark theme.
    ///
    /// # Panics
    ///
    /// Panics if the embedded TOML is malformed.
    pub fn load_gruvbox_dark() -> Self {
        Self::from_toml_str(GRUVBOX_DARK_THEME_SRC)
            .expect("embedded gruvbox dark theme must be valid TOML")
    }

    /// Parse a theme from a TOML string.
    ///
    /// Returns an error if the string cannot be deserialised into a valid
    /// theme. Unknown keys are ignored so user themes can be forward-compatible
    /// with future theme additions.
    pub fn from_toml_str(src: &str) -> anyhow::Result<Self> {
        let raw: RawTheme = Config::builder()
            .add_source(File::from_str(src, FileFormat::Toml))
            .build()?
            .try_deserialize()?;

        Ok(Self {
            level_trace: raw.levels.trace.into_style(),
            level_debug: raw.levels.debug.into_style(),
            level_info: raw.levels.info.into_style(),
            level_warn: raw.levels.warn.into_style(),
            level_error: raw.levels.error.into_style(),
            level_fatal: raw.levels.fatal.into_style(),
            border_focused: raw.borders.focused.into_style(),
            border_command_bar: raw.borders.command_bar.into_style(),
            border_unfocused: raw.borders.unfocused.into_style(),
            search_highlight: raw.search.highlight.into_style(),
            producer_palette: raw
                .producers
                .palette
                .iter()
                .filter_map(|s| parse_color(s))
                .collect(),
        })
    }

    /// Return the [`Style`] for a given [`LogLevel`], or the default style
    /// when the level is unknown.
    pub fn level_style(&self, level: Option<LogLevel>) -> Style {
        match level {
            Some(LogLevel::Trace) => self.level_trace,
            Some(LogLevel::Debug) => self.level_debug,
            Some(LogLevel::Info) => self.level_info,
            Some(LogLevel::Warn) => self.level_warn,
            Some(LogLevel::Error) => self.level_error,
            Some(LogLevel::Fatal) => self.level_fatal,
            None => Style::default(),
        }
    }

    /// Return a stable [`Style`] for a producer name.
    ///
    /// The colour is determined by hashing the name and taking the result
    /// modulo the palette length. The same name always maps to the same colour
    /// within a session, regardless of the order producers appear.
    pub fn producer_style(&self, producer: &str) -> Style {
        if self.producer_palette.is_empty() {
            return Style::default();
        }
        let idx = stable_hash(producer) % self.producer_palette.len();
        Style::default().fg(self.producer_palette[idx])
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple djb2-style hash that is stable across Rust versions and process
/// restarts, making producer colour assignment deterministic.
fn stable_hash(s: &str) -> usize {
    s.bytes().fold(5381usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    })
}

/// Parse a colour name into a ratatui [`Color`].
///
/// Accepts:
/// - Named terminal colours (case-insensitive): `red`, `dark_gray`, etc.
/// - Hex RGB: `#rrggbb`
/// - 256-colour indexed: `indexed:N`
fn parse_color(s: &str) -> Option<Color> {
    match s.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "dark_gray" | "darkgray" | "dark_grey" | "darkgrey" => Some(Color::DarkGray),
        "light_red" => Some(Color::LightRed),
        "light_green" => Some(Color::LightGreen),
        "light_yellow" => Some(Color::LightYellow),
        "light_blue" => Some(Color::LightBlue),
        "light_magenta" => Some(Color::LightMagenta),
        "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        s if s.starts_with('#') && s.len() == 7 => {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        s if s.starts_with("indexed:") => {
            let n: u8 = s["indexed:".len()..].parse().ok()?;
            Some(Color::Indexed(n))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_loads() {
        let theme = Theme::load_default();
        // Spot-check a few resolved styles.
        assert_ne!(theme.level_error, Style::default());
        assert_ne!(theme.border_focused, Style::default());
        assert_ne!(theme.search_highlight, Style::default());
        assert!(!theme.producer_palette.is_empty());
    }

    #[test]
    fn gruvbox_dark_theme_loads() {
        let theme = Theme::load_gruvbox_dark();
        assert_ne!(theme.level_error, Style::default());
        assert_ne!(theme.border_focused, Style::default());
        assert_ne!(theme.search_highlight, Style::default());
        assert!(!theme.producer_palette.is_empty());
    }

    #[test]
    fn producer_style_is_stable() {
        let theme = Theme::load_default();
        let a = theme.producer_style("api-7f9b4d");
        let b = theme.producer_style("api-7f9b4d");
        assert_eq!(a, b);
    }

    #[test]
    fn different_producers_can_differ() {
        let theme = Theme::load_default();
        // Not strictly guaranteed, but with 6 palette colours and distinct
        // names it is overwhelmingly likely.
        let styles: Vec<_> = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]
            .iter()
            .map(|n| theme.producer_style(n))
            .collect();
        let unique: std::collections::HashSet<_> = styles.iter().collect();
        assert!(unique.len() > 1, "all producers mapped to the same colour");
    }

    #[test]
    fn parse_hex_color() {
        assert_eq!(parse_color("#ff0080"), Some(Color::Rgb(255, 0, 128)));
    }

    #[test]
    fn parse_indexed_color() {
        assert_eq!(parse_color("indexed:42"), Some(Color::Indexed(42)));
    }

    #[test]
    fn parse_unknown_color_returns_none() {
        assert_eq!(parse_color("chartreuse"), None);
    }
}
