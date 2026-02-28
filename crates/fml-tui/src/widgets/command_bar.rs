//! Vim-style command bar — a single-line overlay at the bottom of the screen.
//!
//! Activated by pressing `:` from any pane except the query bar. Displays a
//! `:` prefix followed by the typed command, exactly like Vim's command-line
//! mode. Pressing `Enter` parses and executes the command; `Escape` cancels.
//!
//! # Supported commands
//!
//! | Command | Action |
//! |---------|--------|
//! | `q`, `quit` | Quit (or close current tab) |
//! | `help` | Toggle the help popup |
//! | `theme <name>` | Switch theme (`default`, `gruvbox`) |
//! | `ts`, `timestamps` | Toggle timestamp display in the log stream |
//! | `tail` | Jump to the live tail |
//! | `greed <0-10>` | Set the search greed level |

use crate::event::{AppEvent, Direction};
use crate::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Widget},
};

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// A parsed, validated command ready to be executed by the app shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    Help,
    Theme(String),
    Timestamps,
    Tail,
    Greed(u8),
}

impl Command {
    /// Parse a raw command string (the text after the `:` prefix).
    ///
    /// Returns `Ok(cmd)` on success, `Err(message)` on failure. An empty
    /// string returns `Err("")` as a sentinel meaning "close without acting".
    pub fn parse(input: &str) -> Result<Command, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err(String::new());
        }

        let (word, rest) = input
            .split_once(char::is_whitespace)
            .map(|(w, r)| (w, r.trim()))
            .unwrap_or((input, ""));

        match word {
            "q" | "quit" => Ok(Command::Quit),
            "help" => Ok(Command::Help),
            "ts" | "timestamps" => Ok(Command::Timestamps),
            "tail" => Ok(Command::Tail),
            "theme" => {
                if rest.is_empty() {
                    Err("usage: theme <default|gruvbox>".to_string())
                } else {
                    Ok(Command::Theme(rest.to_string()))
                }
            }
            "greed" => match rest.parse::<u8>() {
                Ok(n) if n <= 10 => Ok(Command::Greed(n)),
                Ok(_) => Err("greed must be 0–10".to_string()),
                Err(_) => Err("usage: greed <0-10>".to_string()),
            },
            other => Err(format!("unknown command: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Persistent state for the command bar.
#[derive(Debug, Default)]
pub struct CommandBarState {
    /// The text typed after the `:` prefix.
    pub input: String,
    /// Byte offset of the cursor within `input`.
    pub cursor: usize,
    /// Error message from the last failed command, cleared on the next key.
    pub error: Option<String>,
}

impl CommandBarState {
    /// Reset to a blank, error-free state. Call when opening the bar.
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.error = None;
    }

    /// Handle a key event while the command bar is focused.
    pub fn handle(&mut self, event: &AppEvent) {
        // Any keypress dismisses the error display so the user can edit again.
        self.error = None;

        match event {
            AppEvent::Char(c) => {
                self.input.insert(self.cursor, *c);
                self.cursor += c.len_utf8();
            }
            AppEvent::Backspace => {
                if self.cursor > 0 {
                    let prev = self.input[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.remove(prev);
                    self.cursor = prev;
                }
            }
            AppEvent::TreeNav(Direction::Left) => {
                if self.cursor > 0 {
                    self.cursor = self.input[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            AppEvent::TreeNav(Direction::Right) => {
                if self.cursor < self.input.len() {
                    let next = self.input[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.input.len());
                    self.cursor = next;
                }
            }
            _ => {}
        }
    }

    /// Absolute terminal column of the text cursor within `area`.
    ///
    /// The `:` glyph occupies column 0, so the cursor starts at column 1.
    pub fn cursor_col(&self, area: Rect) -> u16 {
        let col = 1 + self.input[..self.cursor].chars().count() as u16;
        (area.x + col).min(area.right().saturating_sub(1))
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Single-row command-bar overlay.
///
/// The caller is responsible for passing a 1-row `Rect` at the bottom of the
/// terminal. `CommandBar` clears that row with [`Clear`] and renders either
/// the `:<input>` prompt or an error message.
pub struct CommandBar<'a> {
    state: &'a CommandBarState,
    _theme: &'a Theme,
}

impl<'a> CommandBar<'a> {
    pub fn new(state: &'a CommandBarState, theme: &'a Theme) -> Self {
        Self { state, _theme: theme }
    }
}

impl Widget for CommandBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let line = if let Some(ref err) = self.state.error {
            Line::from(Span::styled(
                format!("E  {err}"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(vec![
                Span::styled(":", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(self.state.input.as_str()),
            ])
        };

        buf.set_line(area.x, area.y, &line, area.width);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quit() {
        assert_eq!(Command::parse("q"), Ok(Command::Quit));
        assert_eq!(Command::parse("quit"), Ok(Command::Quit));
        assert_eq!(Command::parse("  quit  "), Ok(Command::Quit));
    }

    #[test]
    fn parse_theme() {
        assert_eq!(
            Command::parse("theme gruvbox"),
            Ok(Command::Theme("gruvbox".to_string()))
        );
        assert!(Command::parse("theme").is_err());
    }

    #[test]
    fn parse_greed() {
        assert_eq!(Command::parse("greed 5"), Ok(Command::Greed(5)));
        assert_eq!(Command::parse("greed 0"), Ok(Command::Greed(0)));
        assert_eq!(Command::parse("greed 10"), Ok(Command::Greed(10)));
        assert!(Command::parse("greed 11").is_err());
        assert!(Command::parse("greed abc").is_err());
    }

    #[test]
    fn parse_empty_returns_sentinel_err() {
        assert_eq!(Command::parse(""), Err(String::new()));
        assert_eq!(Command::parse("  "), Err(String::new()));
    }

    #[test]
    fn parse_unknown() {
        let err = Command::parse("frobnicate").unwrap_err();
        assert!(err.contains("frobnicate"));
    }

    #[test]
    fn state_char_insert_and_backspace() {
        let mut s = CommandBarState::default();
        s.handle(&AppEvent::Char('f'));
        s.handle(&AppEvent::Char('o'));
        s.handle(&AppEvent::Char('o'));
        assert_eq!(s.input, "foo");
        assert_eq!(s.cursor, 3);
        s.handle(&AppEvent::Backspace);
        assert_eq!(s.input, "fo");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn state_error_cleared_on_next_key() {
        let mut s = CommandBarState::default();
        s.error = Some("oops".to_string());
        s.handle(&AppEvent::Char('x'));
        assert!(s.error.is_none());
    }
}
