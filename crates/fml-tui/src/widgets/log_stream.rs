//! Log stream widget — the scrollable live-tail pane on the right.
//!
//! # Navigation (when pane is focused)
//!
//! | Key | Action |
//! |-----|--------|
//! | `↑` / `k` | Move cursor up one line (scrolls view if needed) |
//! | `↓` / `j` | Move cursor down one line |
//! | `PageUp` / `Ctrl+u` | Scroll up one page |
//! | `PageDown` / `Ctrl+d` | Scroll down one page |
//! | `G` | Jump to tail and resume live-tail |
//!
//! # Scroll semantics
//!
//! `scroll_offset` = number of entries hidden at the bottom (0 = live tail).
//! `cursor` = absolute index into `entries` (0 = oldest). The cursor is always
//! kept within the visible window; moving it past the edge auto-scrolls.

use std::cell::Cell;

use crate::event::{AppEvent, Direction};
use tracing;
use crate::theme::Theme;
use fml_core::LogEntry;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget,
    },
};

const PAGE_STEP: usize = 10;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct LogStreamState {
    pub entries: Vec<LogEntry>,
    /// Number of entries hidden at the bottom (0 = live tail).
    pub scroll_offset: usize,
    /// Absolute index into `entries` of the highlighted line.
    pub cursor: usize,
    /// When true, new entries accumulate in `buffered_new` instead of advancing the view.
    pub paused: bool,
    /// Count of entries that arrived while paused and are not yet visible.
    pub buffered_new: usize,
    /// Whether timestamps are shown on each log line.
    pub show_timestamps: bool,
    /// Cached from the last render so `handle()` can do cursor-aware scrolling.
    last_height: Cell<usize>,
}

impl LogStreamState {
    pub fn new(entries: Vec<LogEntry>) -> Self {
        let cursor = entries.len().saturating_sub(1);
        Self {
            entries,
            scroll_offset: 0,
            cursor,
            paused: false,
            buffered_new: 0,
            show_timestamps: true,
            last_height: Cell::new(40),
        }
    }

    fn height(&self) -> usize {
        self.last_height.get().max(1)
    }

    /// Returns `(start, end)` — the exclusive range of entries currently visible.
    fn visible_range(&self) -> (usize, usize) {
        let total = self.entries.len();
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(self.height());
        (start, end)
    }

    /// Handle a navigation event from the app shell.
    ///
    /// Scrolling up sets `paused = true`; pressing `G` or scrolling back to the
    /// tail clears it and resets `buffered_new`.
    pub fn handle(&mut self, event: &AppEvent) {
        let total = self.entries.len();
        if total == 0 {
            return;
        }

        match event {
            // ── Line-by-line cursor movement ───────────────────────────────
            AppEvent::TreeNav(Direction::Up) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                self.paused = true;
                // If cursor scrolled above the window, pull the window up
                let (start, _) = self.visible_range();
                if self.cursor < start {
                    self.scroll_offset =
                        total.saturating_sub(self.cursor + self.height());
                }
                tracing::debug!(
                    cursor = self.cursor,
                    scroll_offset = self.scroll_offset,
                    "stream: cursor up"
                );
            }
            AppEvent::TreeNav(Direction::Down) => {
                if self.cursor + 1 < total {
                    self.cursor += 1;
                }
                // If cursor scrolled below the window, push the window down
                let (_, end) = self.visible_range();
                if self.cursor >= end {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    if self.scroll_offset == 0 {
                        self.paused = false;
                        self.buffered_new = 0;
                    }
                }
                tracing::debug!(
                    cursor = self.cursor,
                    scroll_offset = self.scroll_offset,
                    paused = self.paused,
                    "stream: cursor down"
                );
            }

            // ── Page scrolling ─────────────────────────────────────────────
            AppEvent::ScrollUp => {
                self.paused = true;
                self.scroll_offset = (self.scroll_offset + PAGE_STEP).min(total);
                // Keep cursor inside the new window (snap to bottom of view)
                let (_, end) = self.visible_range();
                self.cursor = end.saturating_sub(1);
                tracing::debug!(
                    scroll_offset = self.scroll_offset,
                    cursor = self.cursor,
                    "stream: page up"
                );
            }
            AppEvent::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(PAGE_STEP);
                let (_, end) = self.visible_range();
                self.cursor = end.saturating_sub(1);
                if self.scroll_offset == 0 {
                    self.paused = false;
                    self.buffered_new = 0;
                    self.cursor = total.saturating_sub(1);
                }
                tracing::debug!(
                    scroll_offset = self.scroll_offset,
                    cursor = self.cursor,
                    paused = self.paused,
                    "stream: page down"
                );
            }

            // ── Jump to tail ───────────────────────────────────────────────
            AppEvent::ScrollToTail => {
                self.scroll_offset = 0;
                self.cursor = total.saturating_sub(1);
                self.paused = false;
                self.buffered_new = 0;
                tracing::debug!(cursor = self.cursor, "stream: jumped to tail");
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct LogStream<'a> {
    state: &'a LogStreamState,
    focused: bool,
    theme: &'a Theme,
}

impl<'a> LogStream<'a> {
    pub fn new(state: &'a LogStreamState, focused: bool, theme: &'a Theme) -> Self {
        Self { state, focused, theme }
    }
}

impl Widget for LogStream<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            self.theme.border_focused
        } else {
            self.theme.border_unfocused
        };

        let block = Block::bordered().title("Logs").border_style(border_style);
        let inner = block.inner(area);
        block.render(area, buf);

        let height = inner.height as usize;
        // Cache for handle() — safe because draw always runs before handle()
        self.state.last_height.set(height);

        let total = self.state.entries.len();
        let end = total.saturating_sub(self.state.scroll_offset);
        let start = end.saturating_sub(height);

        // Which row (0-based within visible window) holds the cursor?
        let cursor_row: Option<usize> =
            if self.focused && self.state.cursor >= start && self.state.cursor < end {
                Some(self.state.cursor - start)
            } else {
                None
            };

        let mut lines: Vec<Line<'static>> = self.state.entries[start..end]
            .iter()
            .enumerate()
            .map(|(row, entry)| {
                let mut line =
                    render_entry(entry, self.state.show_timestamps, self.theme);
                if Some(row) == cursor_row {
                    line = line
                        .patch_style(Style::default().add_modifier(Modifier::REVERSED));
                }
                line
            })
            .collect();

        // Pause banner replaces the top visible line
        if self.state.paused {
            let msg = if self.state.buffered_new > 0 {
                format!(
                    " ⏸  paused — {} new lines (G to resume) ",
                    self.state.buffered_new
                )
            } else {
                " ⏸  paused  (G to resume) ".to_string()
            };
            let banner = Line::from(Span::styled(
                msg,
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            if lines.is_empty() {
                lines.push(banner);
            } else {
                lines[0] = banner;
            }
        }

        // Split inner into text (fill) + 1-column scrollbar strip.
        // The strip is inside the block borders so the track height exactly
        // matches the number of visible content rows — keeping thumb position
        // mathematically aligned with the entries on screen.
        let text_area = Rect { width: inner.width.saturating_sub(1), ..inner };
        let sb_area = Rect {
            x: inner.right().saturating_sub(1),
            width: 1,
            ..inner
        };

        Paragraph::new(lines).render(text_area, buf);

        if total > 0 {
            let mut sb_state = ScrollbarState::new(total)
                .position(start)
                .viewport_content_length(height);
            StatefulWidget::render(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                sb_area,
                buf,
                &mut sb_state,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Entry rendering
// ---------------------------------------------------------------------------

fn render_entry(entry: &LogEntry, show_ts: bool, theme: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    if show_ts {
        spans.push(Span::styled(
            format!("{} ", entry.ts.format("%H:%M:%S%.3f")),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }

    spans.push(Span::styled(
        format!("{:<12} ", entry.producer),
        theme.producer_style(&entry.producer),
    ));

    spans.push(Span::styled(
        "│ ".to_string(),
        Style::default().add_modifier(Modifier::DIM),
    ));

    let msg = entry
        .message
        .as_deref()
        .unwrap_or(entry.raw.as_str())
        .to_string();

    spans.push(Span::styled(msg, theme.level_style(entry.level)));

    Line::from(spans)
}
