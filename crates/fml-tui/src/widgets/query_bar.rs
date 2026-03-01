//! Query bar widget — text input + greed slider at the bottom of the screen.
//!
//! # Editing
//!
//! - `Char(c)` inserts at the cursor.
//! - `Backspace` deletes the character before the cursor.
//! - `TreeNav(Left)` / `TreeNav(Right)` move the cursor (arrow keys or h/l
//!   while this pane is focused, re-mapped by the App shell).
//! - `[` / `]` adjust the greed level (0–10).

use crate::event::{AppEvent, Direction};
use tracing;
use crate::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction as LayoutDir, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

const GREED_MAX: u8 = 10;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct QueryBarState {
    /// The search expression typed by the user.
    pub query: String,
    /// Byte offset of the cursor within `query`.
    pub cursor: usize,
    /// Current greed level (0 = exact match only, 10 = maximum expansion).
    pub greed: u8,
}

impl QueryBarState {
    /// Handle a key event from the app shell.
    ///
    /// Text-editing events (`Char`, `Backspace`, arrow keys) update the query
    /// string. `GreedUp` / `GreedDown` adjust the greed slider; all other
    /// events are ignored.
    pub fn handle(&mut self, event: &AppEvent) {
        match event {
            AppEvent::Char(c) => {
                self.query.insert(self.cursor, *c);
                self.cursor += c.len_utf8();
                tracing::debug!(query = %self.query, cursor = self.cursor, "query: char inserted");
            }
            AppEvent::Backspace => {
                if self.cursor > 0 {
                    // Walk back one char boundary
                    let prev = self.query[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.query.remove(prev);
                    self.cursor = prev;
                    tracing::debug!(query = %self.query, cursor = self.cursor, "query: backspace");
                }
            }
            // Left/right arrows re-mapped from TreeNav by the App shell
            AppEvent::TreeNav(Direction::Left) => {
                if self.cursor > 0 {
                    self.cursor = self.query[..self.cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    tracing::debug!(cursor = self.cursor, "query: cursor left");
                }
            }
            AppEvent::TreeNav(Direction::Right) => {
                if self.cursor < self.query.len() {
                    let next = self.query[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.query.len());
                    self.cursor = next;
                    tracing::debug!(cursor = self.cursor, "query: cursor right");
                }
            }
            AppEvent::GreedUp => {
                if self.greed < GREED_MAX {
                    self.greed += 1;
                    tracing::debug!(greed = self.greed, "query: greed up");
                }
            }
            AppEvent::GreedDown => {
                if self.greed > 0 {
                    self.greed -= 1;
                    tracing::debug!(greed = self.greed, "query: greed down");
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct QueryBar<'a> {
    state: &'a QueryBarState,
    focused: bool,
    theme: &'a Theme,
}

impl<'a> QueryBar<'a> {
    pub fn new(state: &'a QueryBarState, focused: bool, theme: &'a Theme) -> Self {
        Self { state, focused, theme }
    }

    /// Absolute terminal position of the text cursor within this widget's
    /// rendered area. Pass to `frame.set_cursor_position()` after rendering.
    pub fn cursor_position(&self, area: Rect) -> (u16, u16) {
        // The block adds 1-cell borders; text starts at (area.x+1, area.y+1).
        let col = self.state.query[..self.state.cursor].chars().count() as u16;
        let x = (area.x + 1 + col).min(area.right().saturating_sub(1));
        let y = area.y + 1;
        (x, y)
    }
}

impl Widget for QueryBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            self.theme.border_focused
        } else {
            self.theme.border_unfocused
        };

        let block = Block::bordered()
            .title("Query")
            .border_style(border_style);

        let inner = block.inner(area);
        block.render(area, buf);

        // Split inner area: query text (fill) | greed slider (fixed width)
        let chunks = Layout::default()
            .direction(LayoutDir::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(20)])
            .split(inner);

        // Query input
        let query_line = if self.state.query.is_empty() && !self.focused {
            Line::from(Span::styled(
                "press / to search",
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else {
            Line::from(self.state.query.as_str())
        };
        Paragraph::new(query_line).render(chunks[0], buf);

        // Greed slider:  greed: [=====-----] 5
        let filled = self.state.greed as usize;
        let empty = GREED_MAX as usize - filled;
        let slider = format!("greed:[{}{}]{}", "=".repeat(filled), "-".repeat(empty), self.state.greed);
        Paragraph::new(Line::from(slider)).render(chunks[1], buf);
    }
}
