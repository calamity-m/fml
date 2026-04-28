use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

use super::themes::DEFAULT_THEME_NAME;
use crate::error::FmlError;
use crate::log::LogLevel;

/// TUI rendering and interaction configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TuiConfig {
    /// Target frames per second for terminal rendering.
    ///
    /// Higher values produce smoother output but increase draw overhead.
    /// Terminals that cannot keep up will naturally drop frames.
    #[serde(default = "default_frame_rate")]
    pub frame_rate: f64,

    /// Percentage of the content area allocated to the right-hand sidebar.
    ///
    /// Values outside the safe `1..=99` range are clamped so both the main
    /// pane and the sidebar stay visible.
    #[serde(default = "default_sidebar_width_percent")]
    pub sidebar_width_percent: u16,

    /// Built-in theme to apply to the TUI.
    ///
    /// `default` uses the colors from `[tui.default_theme]`. Built-in themes
    /// currently available are `forest`, `kanagawa_dragon`, `mono`, and `ocean`.
    #[serde(default = "default_theme_name")]
    pub theme: String,

    /// User-defined theme used when `theme = "default"`.
    #[serde(default)]
    pub default_theme: ThemeConfig,

    /// Keyboard shortcut configuration.
    ///
    /// Override individual bindings under `[tui.keybindings]`. Omitting this
    /// section entirely keeps the built-in defaults. See [`KeybindingsConfig`]
    /// for the full list of configurable actions, their default keys, and the
    /// reserved fallback keys that remain active even when aliases are remapped
    /// or disabled.
    #[serde(default)]
    pub keybindings: KeybindingsConfig,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            frame_rate: default_frame_rate(),
            sidebar_width_percent: default_sidebar_width_percent(),
            theme: default_theme_name(),
            default_theme: ThemeConfig::default(),
            keybindings: KeybindingsConfig::default(),
        }
    }
}

impl TuiConfig {
    /// Resolve the configured theme selector into the concrete widget palette.
    pub fn resolved_theme(&self) -> Result<ThemeConfig, FmlError> {
        super::themes::resolve_theme(&self.theme, &self.default_theme)
    }
}

fn default_frame_rate() -> f64 {
    60.0
}

fn default_sidebar_width_percent() -> u16 {
    30
}

fn default_theme_name() -> String {
    DEFAULT_THEME_NAME.to_string()
}

/// Theme / color configuration for TUI widgets.
///
/// All colors accept any ratatui [`Color`] variant name (e.g. `"DarkGray"`,
/// `"Yellow"`), hex RGB strings (e.g. `"#FF8000"`), and explicit ratatui
/// constructors such as `"Rgb(255,128,0)"`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ThemeConfig {
    /// Optional background color applied to widget surfaces.
    ///
    /// When unset, widgets use the terminal default background.
    #[serde(default)]
    pub background: Option<Color>,

    /// Foreground color for pane borders that do not have focus.
    #[serde(default = "default_border_unfocused_fg")]
    pub border_unfocused_fg: Color,

    /// Primary accent foreground color for things like status bar keys
    #[serde(default = "default_primary_accent_fg")]
    pub primary_accent_fg: Color,

    /// Secondary accent foreground color for things like the query prompt character (`>`).
    #[serde(default = "default_secondary_accent_fg")]
    pub secondary_accent_fg: Color,

    /// Background color for the currently selected log row.
    #[serde(default = "default_log_selected_bg")]
    pub log_selected_bg: Color,

    /// Text modifier applied on top of the selection background.
    ///
    /// Defaults to `BOLD` so selected rows remain legible even when the level
    /// foreground colour is close to `log_selected_bg`.
    #[serde(default = "default_log_selected_modifier")]
    pub log_selected_modifier: Modifier,

    /// Foreground color for highlighted text that matches the active search.
    #[serde(default = "default_log_match_fg")]
    pub log_match_fg: Color,

    /// Rendering mode for highlighted text that matches the active search.
    #[serde(default)]
    pub log_match_style: LogMatchStyle,

    /// Foreground colors for log rows keyed by parsed severity.
    #[serde(default)]
    pub log_level: LogLevelThemeConfig,

    /// Whether matched text is rendered in bold.
    #[serde(default = "default_true")]
    pub log_match_bold: bool,

    /// Whether secondary status bar text such as separators is rendered dimmed.
    #[serde(default = "default_true")]
    pub status_dim: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: None,
            border_unfocused_fg: default_border_unfocused_fg(),
            primary_accent_fg: default_primary_accent_fg(),
            secondary_accent_fg: default_secondary_accent_fg(),
            log_selected_bg: default_log_selected_bg(),
            log_selected_modifier: default_log_selected_modifier(),
            log_match_fg: default_log_match_fg(),
            log_match_style: LogMatchStyle::default(),
            log_level: LogLevelThemeConfig::default(),
            log_match_bold: true,
            status_dim: true,
        }
    }
}

impl ThemeConfig {
    /// Base style for widget surfaces, including any configured background.
    pub fn surface_style(&self) -> ratatui::style::Style {
        match self.background {
            Some(color) => ratatui::style::Style::default().bg(color),
            None => ratatui::style::Style::default(),
        }
    }

    /// Style for the currently selected row, combining bg tint and modifier.
    pub fn selected_style(&self) -> Style {
        let mut style = self.surface_style().bg(self.log_selected_bg);
        if !self.log_selected_modifier.is_empty() {
            style = style.add_modifier(self.log_selected_modifier);
        }
        style
    }

    /// Map a parsed log level to the configured row foreground color.
    pub fn log_row_fg(&self, level: Option<LogLevel>) -> Color {
        self.log_level.color_for(level)
    }

    /// Style overlay for text matching the active fuzzy search.
    pub fn match_style(&self) -> Style {
        match self.log_match_style {
            LogMatchStyle::Color => {
                let mut style = Style::default().fg(self.log_match_fg);
                if self.log_match_bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            }
            LogMatchStyle::Bold => Style::default().add_modifier(Modifier::BOLD),
            LogMatchStyle::Underline => Style::default().add_modifier(Modifier::UNDERLINED),
            LogMatchStyle::Reversed => Style::default().add_modifier(Modifier::REVERSED),
        }
    }
}

/// Rendering mode for text matching the active fuzzy search.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogMatchStyle {
    /// Apply the configured match foreground, plus bold when enabled.
    #[default]
    Color,
    /// Apply bold without changing foreground color.
    Bold,
    /// Apply underline without changing foreground color.
    Underline,
    /// Reverse foreground/background for a visible terminal block.
    Reversed,
}

/// Foreground colors applied to log rows for each parsed severity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct LogLevelThemeConfig {
    /// Foreground color for rows whose level could not be parsed.
    #[serde(default = "default_log_default_fg")]
    pub default_fg: Color,

    /// Foreground color for `TRACE` rows.
    #[serde(default = "default_log_trace_fg")]
    pub trace_fg: Color,

    /// Foreground color for `DEBUG` rows.
    #[serde(default = "default_log_debug_fg")]
    pub debug_fg: Color,

    /// Foreground color for `INFO` rows.
    #[serde(default = "default_log_info_fg")]
    pub info_fg: Color,

    /// Foreground color for `WARN` rows.
    #[serde(default = "default_log_warn_fg")]
    pub warn_fg: Color,

    /// Foreground color for `ERROR` rows.
    #[serde(default = "default_log_error_fg")]
    pub error_fg: Color,

    /// Foreground color for `FATAL` rows.
    #[serde(default = "default_log_fatal_fg")]
    pub fatal_fg: Color,
}

impl Default for LogLevelThemeConfig {
    fn default() -> Self {
        Self {
            default_fg: default_log_default_fg(),
            trace_fg: default_log_trace_fg(),
            debug_fg: default_log_debug_fg(),
            info_fg: default_log_info_fg(),
            warn_fg: default_log_warn_fg(),
            error_fg: default_log_error_fg(),
            fatal_fg: default_log_fatal_fg(),
        }
    }
}

impl LogLevelThemeConfig {
    fn color_for(&self, level: Option<LogLevel>) -> Color {
        match level {
            None => self.default_fg,
            Some(LogLevel::Trace) => self.trace_fg,
            Some(LogLevel::Debug) => self.debug_fg,
            Some(LogLevel::Info) => self.info_fg,
            Some(LogLevel::Warn) => self.warn_fg,
            Some(LogLevel::Error) => self.error_fg,
            Some(LogLevel::Fatal) => self.fatal_fg,
        }
    }
}

fn default_border_unfocused_fg() -> Color {
    Color::DarkGray
}

fn default_secondary_accent_fg() -> Color {
    Color::Indexed(244)
}

fn default_primary_accent_fg() -> Color {
    Color::Yellow
}

fn default_log_selected_bg() -> Color {
    Color::Rgb(0x2A, 0x2A, 0x2A)
}

fn default_log_selected_modifier() -> Modifier {
    Modifier::BOLD
}

fn default_log_match_fg() -> Color {
    Color::Yellow
}

fn default_log_default_fg() -> Color {
    Color::Reset
}

fn default_log_trace_fg() -> Color {
    Color::Gray
}

fn default_log_debug_fg() -> Color {
    Color::Cyan
}

fn default_log_info_fg() -> Color {
    Color::Reset
}

fn default_log_warn_fg() -> Color {
    Color::Yellow
}

fn default_log_error_fg() -> Color {
    Color::Red
}

fn default_log_fatal_fg() -> Color {
    Color::LightRed
}

fn default_true() -> bool {
    true
}

/// Keyboard shortcut configuration under `[tui.keybindings]`.
///
/// Each field holds one or more key spec strings. All specs for an action are
/// matched at runtime; the first spec is used as the primary display label in
/// the status bar and help popup. An empty list disables the binding.
///
/// The resolved runtime bindings are also the source of truth for the help
/// popup and status bar, so remapped keys automatically update those surfaces.
/// Runtime precedence stays narrow and explicit: reserved fallbacks (`ctrl+c`,
/// `tab`, `esc`) are checked first, popup-local handling runs before other
/// widgets, then app-level/global bindings are matched before the focused
/// widget sees any unmatched key event.
///
/// # Key spec format
///
/// A key spec is a lowercase string like `"?"`, `"enter"`, `"ctrl+c"`,
/// `"pgdn"`, or `"f1"`. Optional modifier prefixes (`ctrl`, `alt`, `shift`)
/// are joined with `+`. Printable characters are written directly — use `"?"`
/// rather than `"shift+/"`.
///
/// # Reserved fallbacks
///
/// The following bindings are always active and cannot be replaced via config:
///
/// - `ctrl+c` — quit the application (checked before any configured binding)
/// - `tab` — cycle focus between panes
/// - `esc` — close any open popup
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KeybindingsConfig {
    /// Open or close the help popup.
    ///
    /// Active globally. Also closes the popup when one is already open.
    #[serde(default = "default_toggle_help")]
    pub toggle_help: Vec<String>,

    /// Show the entry detail popup for the selected log entry.
    ///
    /// Only fires when the log pane has focus.
    #[serde(default = "default_show_info")]
    pub show_info: Vec<String>,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            toggle_help: default_toggle_help(),
            show_info: default_show_info(),
        }
    }
}


fn default_toggle_help() -> Vec<String> {
    vec!["?".into()]
}

fn default_show_info() -> Vec<String> {
    vec!["enter".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_has_bold_selection_modifier() {
        assert_eq!(
            ThemeConfig::default().log_selected_modifier,
            Modifier::BOLD
        );
    }
}
