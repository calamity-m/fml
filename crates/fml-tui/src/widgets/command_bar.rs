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
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Widget},
};

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
    ///
    /// Returns `Some(AppEvent)` when the bar should close and dispatch an event
    /// to the app (e.g. `Some(Quit)` after `:q`, `Some(NoOp)` on Escape or
    /// empty Enter). Returns `None` to keep the bar open (typing, cursor movement).
    pub fn handle(&mut self, event: &AppEvent) -> Option<AppEvent> {
        // Any keypress dismisses the error display so the user can edit again.
        if self.error.is_some() {
            self.clear();
        }
        self.error = None;

        match event {
            AppEvent::Escape => {
                tracing::debug!("command bar cancelled");
                self.clear();
                Some(AppEvent::NoOp)
            }
            AppEvent::Enter => {
                let input = self.input.clone();

                match AppEvent::parse_str(&input) {
                    Ok(event) => {
                        tracing::debug!(appevent = ?event, "command generating app event");
                        self.clear();
                        Some(event)
                    }
                    Err(msg) => {
                        if msg.is_empty() {
                            self.clear();
                            Some(AppEvent::NoOp)
                        } else {
                            self.error = Some(msg);
                            Some(AppEvent::NoOp)
                        }
                    }
                }
            }
            AppEvent::Char(c) => {
                self.input.insert(self.cursor, *c);
                self.cursor += c.len_utf8();
                None
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
                None
            }
            AppEvent::TreeNav(Direction::Left) => {
                if self.cursor > 0 {
                    self.cursor = self.input[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
                None
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
                None
            }
            _ => None,
        }
    }

    /// Absolute terminal column of the text cursor within `area`.
    ///
    /// The `:` glyph occupies column 0, so the cursor starts at column 1.
    /// but then we have a border - so we'll use column 2.
    pub fn cursor_col(&self, area: Rect) -> u16 {
        let col = 2 + self.input[..self.cursor].chars().count() as u16;
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
    theme: &'a Theme,
}

impl<'a> CommandBar<'a> {
    pub fn new(state: &'a CommandBarState, theme: &'a Theme) -> Self {
        Self { state, theme }
    }
}

impl Widget for CommandBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let block = Block::bordered()
            .title("Command")
            .border_style(self.theme.border_command_bar);
        let inner = block.inner(area);
        block.render(area, buf);

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

        let chunks = Layout::default()
            .direction(LayoutDir::Horizontal)
            .constraints([Constraint::Fill(1)])
            .split(inner);

        Paragraph::new(line).render(chunks[0], buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Type a string into the bar one character at a time.
    fn type_str(s: &mut CommandBarState, text: &str) {
        for c in text.chars() {
            s.handle(&AppEvent::Char(c));
        }
    }

    // ── Typing & editing ─────────────────────────────────────────────────────

    #[test]
    fn char_inserts_and_advances_cursor() {
        let mut s = CommandBarState::default();
        s.handle(&AppEvent::Char('f'));
        s.handle(&AppEvent::Char('o'));
        s.handle(&AppEvent::Char('o'));
        assert_eq!(s.input, "foo");
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn char_returns_none_bar_stays_open() {
        let mut s = CommandBarState::default();
        let result = s.handle(&AppEvent::Char('x'));
        assert_eq!(result, None);
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "foo");
        s.handle(&AppEvent::Backspace);
        assert_eq!(s.input, "fo");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut s = CommandBarState::default();
        s.handle(&AppEvent::Backspace);
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn backspace_returns_none() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "a");
        assert_eq!(s.handle(&AppEvent::Backspace), None);
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    #[test]
    fn cursor_left_moves_back() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "ab");
        s.handle(&AppEvent::TreeNav(Direction::Left));
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn cursor_right_moves_forward() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "ab");
        s.handle(&AppEvent::TreeNav(Direction::Left));
        s.handle(&AppEvent::TreeNav(Direction::Right));
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn cursor_left_at_start_is_noop() {
        let mut s = CommandBarState::default();
        s.handle(&AppEvent::TreeNav(Direction::Left));
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn cursor_right_at_end_is_noop() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "a");
        s.handle(&AppEvent::TreeNav(Direction::Right));
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn insert_at_cursor_mid_string() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "ac");
        s.handle(&AppEvent::TreeNav(Direction::Left)); // cursor before 'c'
        s.handle(&AppEvent::Char('b'));
        assert_eq!(s.input, "abc");
    }

    // ── Escape ────────────────────────────────────────────────────────────────

    #[test]
    fn escape_clears_input_and_returns_noop() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "quit");
        let result = s.handle(&AppEvent::Escape);
        assert_eq!(result, Some(AppEvent::NoOp));
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
        assert!(s.error.is_none());
    }

    // ── Enter: valid commands ─────────────────────────────────────────────────

    #[test]
    fn enter_quit_emits_quit_event() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "q");
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::Quit));
        assert_eq!(s.input, ""); // cleared after dispatch
    }

    #[test]
    fn enter_quit_long_form() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "quit");
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::Quit));
    }

    #[test]
    fn enter_timestamps_emits_timestamps_event() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "ts");
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::Timestamps));
    }

    #[test]
    fn enter_tail_emits_scroll_to_tail() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "tail");
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::ScrollToTail));
    }

    #[test]
    fn enter_theme_emits_theme_event() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "theme gruvbox");
        assert_eq!(
            s.handle(&AppEvent::Enter),
            Some(AppEvent::Theme("gruvbox".to_string()))
        );
    }

    #[test]
    fn enter_greed_emits_greed_event() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "greed 7");
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::Greed(7)));
    }

    #[test]
    fn enter_clears_state_on_success() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "tail");
        s.handle(&AppEvent::Enter);
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
        assert!(s.error.is_none());
    }

    // ── Enter: empty / invalid ────────────────────────────────────────────────

    #[test]
    fn enter_on_empty_input_returns_noop() {
        let mut s = CommandBarState::default();
        assert_eq!(s.handle(&AppEvent::Enter), Some(AppEvent::NoOp));
    }

    #[test]
    fn enter_unknown_command_sets_error_and_returns_noop() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "frobnicate");
        let result = s.handle(&AppEvent::Enter);
        assert_eq!(result, Some(AppEvent::NoOp));
        assert!(s.error.is_some());
        assert!(s.error.as_ref().unwrap().contains("frobnicate"));
    }

    #[test]
    fn enter_theme_without_arg_sets_error() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "theme");
        let result = s.handle(&AppEvent::Enter);
        assert_eq!(result, Some(AppEvent::NoOp));
        assert!(s.error.is_some());
    }

    // ── Error state ───────────────────────────────────────────────────────────

    #[test]
    fn error_is_cleared_on_next_keypress() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "bad");
        s.handle(&AppEvent::Enter); // sets error
        assert!(s.error.is_some());
        s.handle(&AppEvent::Char('x')); // any key clears error
        assert!(s.error.is_none());
    }

    #[test]
    fn input_is_cleared_when_error_is_dismissed() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "bad");
        s.handle(&AppEvent::Enter);
        s.handle(&AppEvent::Char('q')); // dismiss + start fresh
        assert_eq!(s.input, "q");
    }

    // ── clear() ───────────────────────────────────────────────────────────────

    #[test]
    fn clear_resets_all_fields() {
        let mut s = CommandBarState::default();
        type_str(&mut s, "something");
        s.error = Some("oops".to_string());
        s.clear();
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
        assert!(s.error.is_none());
    }
}
