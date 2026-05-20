use std::sync::Arc;

use ratatui::{
    layout::Alignment,
    text::{Line, Span, Text},
    widgets::{
        Block, List, ListItem, ListState, Paragraph, ScrollDirection, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use tracing::{debug, error};

use crate::{
    config::tui::ThemeConfig,
    event::{Match, Query, TuiEvent},
    log::LogEntry,
    state::{
        events_bus::EventBus,
        tui_state::{
            TuiState,
            log_pane_state::{LogPaneState, ScrollMode},
        },
    },
    tui::{
        keybinds::{self, StaticKeyAction},
        layout::Slot,
        widgets::{FmlWidget, highlight, wrap},
    },
};

pub struct LogPane {}

impl Default for LogPane {
    fn default() -> Self {
        Self::new()
    }
}

impl LogPane {
    pub fn new() -> Self {
        LogPane {}
    }

    fn title(&self, state: &LogPaneState) -> String {
        let base = match state.mode {
            ScrollMode::Tail => "TAIL",
            ScrollMode::History => "HISTORY",
            ScrollMode::Search => "SEARCH",
        };

        let mut title = format!(
            " FML [{base}] | Store {}/{}",
            format_count(state.store_stats.retained),
            format_count(state.store_stats.capacity)
        );
        if let Some(progress) = state.fuzzy_scan_progress() {
            if progress.scanned >= progress.total {
                title.push_str(" | SCAN done");
            } else {
                title.push_str(&format!(
                    " | SCAN {}/{}",
                    format_count(progress.scanned),
                    format_count(progress.total)
                ));
            }
        }
        title.push(' ');
        title
    }

    fn dispatch_search(query: Option<Query>, events_bus: &mut EventBus) {
        let Some(query) = query else {
            return;
        };

        if let Err(err) = events_bus
            .tui_event_tx
            .send(TuiEvent::DispatchLogPaneSearch(query))
        {
            error!("failed to send search event from log pane - {}", err);
        }
    }

    /// Build the renderable `Text` for one log entry.
    ///
    /// When `wrap_width` is `None`, returns a single-line `Text` (the classic
    /// truncated rendering). When `Some(width)`, wraps the `msg` field at
    /// word boundaries with continuation lines indented to `indent_column`
    /// columns, so the seq/level/source prefix anchors the eye and the
    /// continuation `msg` text aligns under itself.
    ///
    /// Match highlights survive the wrap because the underlying
    /// [`wrap::wrap_styled_spans`] preserves per-character style runs.
    fn render_item(
        entry: &Arc<LogEntry>,
        leading_id: String,
        matches: Option<&[Match]>,
        theme: &ThemeConfig,
        wrap_width: Option<u16>,
        indent_column: u16,
    ) -> Text<'static> {
        let base_style = theme.surface_style().fg(theme.log_row_fg(entry.level));
        let match_style = theme.surface_style().patch(theme.match_style());
        let level = entry
            .level
            .map(|l| l.to_string())
            .unwrap_or_else(|| "----".to_string());

        let mut prefix: Vec<Span<'static>> = Vec::new();
        prefix.push(Span::styled(leading_id, base_style));
        prefix.push(Span::styled(" ", base_style));
        prefix.extend(highlight::styled_field(
            &level,
            matches,
            "level",
            base_style,
            match_style,
        ));
        prefix.push(Span::styled(" ", base_style));
        prefix.extend(highlight::styled_field(
            &entry.source.display_name,
            matches,
            "source",
            base_style,
            match_style,
        ));
        prefix.push(Span::styled(" ", base_style));

        let msg_spans =
            highlight::styled_field(&entry.msg, matches, "msg", base_style, match_style);

        match wrap_width {
            None => {
                let mut spans = prefix;
                spans.extend(msg_spans);
                Text::from(Line::from(spans))
            }
            Some(width) => {
                // Pad prefix out to indent_column so the first wrapped chunk
                // begins at the same column the continuation lines align under.
                let prefix_cells: u16 = prefix_display_width(&prefix);
                if let Some(pad) = indent_column.checked_sub(prefix_cells)
                    && pad > 0
                {
                    prefix.push(Span::styled(" ".repeat(pad as usize), base_style));
                }

                let indent_spans =
                    vec![Span::styled(" ".repeat(indent_column as usize), base_style)];
                let msg_lines = wrap::wrap_styled_spans(msg_spans, width, &indent_spans, true);

                // The first wrapped line is unindented — that's where the prefix
                // sits. Continuation lines from `wrap_styled_spans` already carry
                // the hanging indent.
                let mut lines: Vec<Line<'static>> = Vec::with_capacity(msg_lines.len().max(1));
                let mut iter = msg_lines.into_iter();
                let first = iter.next().unwrap_or_default();
                let mut first_spans = prefix;
                first_spans.extend(first.spans);
                lines.push(Line::from(first_spans));
                for line in iter {
                    lines.push(line);
                }
                Text::from(lines)
            }
        }
    }

    fn dispatch_selected_entry(state: &TuiState, events_bus: &mut EventBus) {
        let selected_entry = state.log_pane.selected_entry();
        if selected_entry.is_none() && state.log_pane.selected_seq().is_some() {
            return;
        }

        if let Err(err) = events_bus
            .tui_event_tx
            .send(TuiEvent::NewSelectedEntry(selected_entry))
        {
            error!(
                "failed to send selected entry event from log pane - {}",
                err
            );
        }
    }
}

impl FmlWidget for LogPane {
    fn slot(&self) -> Slot {
        Slot::Main
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut TuiState,
    ) {
        // Draw the outer border and title. `inner_area` is the rect inside the
        // border — this is where the list actually goes.
        let (border_style, border_type) = self.border(&state.focused, &state.selected_theme);
        let block = Block::bordered()
            .title(self.title(&state.log_pane))
            .border_style(border_style)
            .border_type(border_type)
            .style(state.selected_theme.surface_style());

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        // How many rows fit in the pane. Each ListItem is one line, so this is
        // also the number of visible log entries at any time.
        state
            .log_pane
            .set_height(inner_area.height as usize, &mut state.log_pane_cursor_row);

        if state.log_pane.visible_items().is_empty()
            && let Some(message) = state.log_pane.empty_message()
        {
            frame.render_widget(
                Paragraph::new(message)
                    .alignment(Alignment::Center)
                    .style(state.selected_theme.surface_style()),
                inner_area,
            );
            return;
        }

        // Pre-compute the leading-id strings the same way they'll appear in the
        // rendered prefix, so the indent_column derivation matches the actual
        // first-line column the prefix produces.
        let leading_ids: Vec<String> = state
            .log_pane
            .visible_items()
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                if state.log_pane.mode == ScrollMode::Search {
                    state
                        .log_pane
                        .view_start()
                        .saturating_add(idx)
                        .saturating_add(1)
                        .to_string()
                } else {
                    entry.seq.to_string()
                }
            })
            .collect();

        // indent_column = max display width of the prefix (leading_id + " " +
        // level + " " + source.display_name + " ") across the currently-visible
        // entries. Derived per-frame from visible_items rather than from
        // store.bounds() / known-sources, which is what the bigplan recommends
        // for layout stability — but with no two-pass measurement here, the
        // single-pass derivation is sufficient and avoids extra plumbing.
        let indent_column: u16 = state
            .log_pane
            .visible_items()
            .iter()
            .zip(leading_ids.iter())
            .map(|(entry, leading_id)| prefix_width_for(entry, leading_id))
            .max()
            .unwrap_or(0);

        let wrap_width: Option<u16> = if state.log_pane.line_wrap() {
            let usable = inner_area.width.saturating_sub(indent_column);
            // wrap_width <= 0 falls back to truncated rendering for this frame.
            if usable > 0 { Some(usable) } else { None }
        } else {
            None
        };

        // Render whatever domain the state resolved for this mode: retained
        // sequence order for tail/history, rank order for search.
        let items: Vec<ListItem> = state
            .log_pane
            .visible_items()
            .iter()
            .zip(leading_ids)
            .map(|(entry, leading_id)| {
                let matches = (state.log_pane.mode == ScrollMode::Search)
                    .then(|| state.log_pane.fuzzy_matches_for(entry.seq))
                    .flatten();
                ListItem::new(Self::render_item(
                    entry,
                    leading_id,
                    matches,
                    &state.selected_theme,
                    wrap_width,
                    indent_column,
                ))
            })
            .collect();
        // The list itself only knows about display styles. It has no concept of
        // scroll position — that comes from ListState below.
        let list = List::new(items)
            .style(state.selected_theme.surface_style())
            .highlight_symbol("> ")
            // Three independent cues: tinted bg, bold weight, and the leading
            // marker. Even if level fg matches selected bg, the row stays legible.
            .highlight_style(state.selected_theme.selected_style());

        // ListState is constructed fresh every frame — it's just a render detail,
        // not persistent state. `with_selected` tells the List which item in the
        // Vec to highlight.
        //
        // The selected sequence is translated into the Vec<ListItem> slice we
        // just built. These must use the same view window or the highlight
        // lands on the wrong row.
        let mut list_state =
            ListState::default().with_selected(state.log_pane.selected_visible_index());

        frame.render_stateful_widget(list, inner_area, &mut list_state);

        // Render the scrollbar over the right border of the block. We render on
        // `area` (the outer rect including the border), not `inner_area` — this
        // is what makes it sit on top of the border rather than inside the pane.
        //
        // begin/end symbols are None so there are no arrow caps, keeping the
        // corners of the border intact. The track symbol matches the border
        // character so inactive track segments are invisible — only the thumb
        // stands out.
        if let Some(metrics) = state.log_pane.scrollbar_metrics() {
            let mut scrollbar_state = ScrollbarState::new(metrics.content_length)
                .viewport_content_length(metrics.viewport_content_length)
                .position(metrics.position);

            // Inset by 1 row top and bottom so the track sits between the border
            // corners rather than overwriting them.
            let scrollbar_area = ratatui::prelude::Rect {
                y: area.y + 1,
                height: area.height.saturating_sub(2),
                ..area
            };

            debug!(
                "scrollbar metrics - {:?}, log_pane_cursor_row - {}",
                metrics, state.log_pane_cursor_row
            );

            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("▲"))
                    .end_symbol(Some("▼"))
                    .track_symbol(Some("│"))
                    .thumb_symbol("█"),
                scrollbar_area,
                &mut scrollbar_state,
            );
        }
    }

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus) {
        match event {
            TuiEvent::Scroll(scroll) => {
                debug!("received scroll event - {:?}", scroll);

                let query = match scroll {
                    ScrollDirection::Forward => state
                        .log_pane
                        .scroll_forward(&mut state.log_pane_cursor_row),
                    ScrollDirection::Backward => state
                        .log_pane
                        .scroll_backward(&mut state.log_pane_cursor_row),
                };
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
            }
            TuiEvent::ScrollHead => {
                debug!("received scroll head event");
                let query = state.log_pane.jump_head(&mut state.log_pane_cursor_row);
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
            }
            TuiEvent::ScrollTail => {
                debug!("received scroll tail event");
                let query = state.log_pane.jump_tail(&mut state.log_pane_cursor_row);
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
            }
            TuiEvent::Input(key) => {
                debug!("received input event - {:?}", key);
                let (static_key, custom_key) = keybinds::match_key(&key, &state.focused);

                // First we process static keys as higher relevance

                let result = match static_key {
                    StaticKeyAction::ScrollUp => events_bus
                        .tui_event_tx
                        .send(TuiEvent::Scroll(ScrollDirection::Backward)),
                    StaticKeyAction::ScrollDown => events_bus
                        .tui_event_tx
                        .send(TuiEvent::Scroll(ScrollDirection::Forward)),
                    _ => Ok(()),
                };

                if let Err(err) = result {
                    error!("failed to send tui event for static key match - {}", err)
                }

                let result = match custom_key {
                    keybinds::CustomizedKeyAction::ScrollHead => {
                        events_bus.tui_event_tx.send(TuiEvent::ScrollHead)
                    }
                    keybinds::CustomizedKeyAction::ScrollTail => {
                        events_bus.tui_event_tx.send(TuiEvent::ScrollTail)
                    }
                    keybinds::CustomizedKeyAction::ToggleLineWrap => {
                        let next = !state.log_pane.line_wrap();
                        state
                            .log_pane
                            .set_line_wrap(next, &mut state.log_pane_cursor_row);
                        Ok(())
                    }
                    _ => Ok(()),
                };

                if let Err(err) = result {
                    error!("failed to send tui event for static key match - {}", err)
                }
            }
            _ => {}
        }
    }
}

/// Display-cell width the rendered prefix for one entry would occupy:
/// `leading_id + " " + level(4) + " " + source.display_name + " "`. Matches
/// the spans assembled in [`LogPane::render_item`] so first-line padding lines
/// up with the hanging indent on continuation lines.
fn prefix_width_for(entry: &Arc<LogEntry>, leading_id: &str) -> u16 {
    use unicode_width::UnicodeWidthStr;
    let id = leading_id.width();
    // Level is rendered as 4 chars whether the entry has one or not ("----").
    let level = 4;
    let source = entry.source.display_name.width();
    // 3 single-cell spaces between id/level/source and after source.
    u16::try_from(id + 1 + level + 1 + source + 1).unwrap_or(u16::MAX)
}

/// Display-cell width of a span sequence, used to align the wrapped-mode
/// first-line prefix with the hanging indent on continuation lines.
fn prefix_display_width(spans: &[Span<'static>]) -> u16 {
    use unicode_width::UnicodeWidthChar;
    let total: usize = spans
        .iter()
        .flat_map(|s| s.content.chars())
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum();
    u16::try_from(total).unwrap_or(u16::MAX)
}

fn format_count(value: usize) -> String {
    match value {
        1_000_000.. => compact_count(value, 1_000_000, "M"),
        1_000.. => compact_count(value, 1_000, "K"),
        _ => value.to_string(),
    }
}

fn compact_count(value: usize, unit: usize, suffix: &str) -> String {
    let whole = value / unit;
    let decimal = (value % unit) / (unit / 10);
    if decimal == 0 || whole >= 10 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{decimal}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use ratatui::style::{Color, Modifier, Style};

    use super::*;
    use crate::{
        config::tui::{LogMatchStyle, ThemeConfig},
        event::{Match, TuiEvent},
        log::{LogLevel, Source},
        state::{events_bus::EventBus, tui_state::TuiState},
    };

    fn entry(level: LogLevel) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq: 42,
            msg: "error".to_string(),
            ts: Utc::now(),
            level: Some(level),
            source: Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "payments".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    fn sequenced_entry(seq: u64) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq,
            msg: format!("entry {seq}"),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "src-a".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    #[test]
    fn styled_field_splits_non_adjacent_match_indices() {
        let base = Style::default().fg(Color::Red);
        let matched = Style::default().fg(Color::Yellow);
        let matches = [Match {
            key: "msg".to_string(),
            indices: vec![0, 2],
        }];

        let spans = highlight::styled_field("error", Some(&matches), "msg", base, matched);

        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "e");
        assert_eq!(spans[0].style.fg, Some(Color::Yellow));
        assert_eq!(spans[1].content, "r");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[2].content, "r");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
        assert_eq!(spans[3].content, "or");
        assert_eq!(spans[3].style.fg, Some(Color::Red));
    }

    #[test]
    fn render_line_applies_level_color_and_match_override() {
        let theme = ThemeConfig {
            log_match_style: LogMatchStyle::Underline,
            ..ThemeConfig::default()
        };
        let matches = [Match {
            key: "msg".to_string(),
            indices: vec![0],
        }];

        let text = LogPane::render_item(
            &entry(LogLevel::Error),
            "1".to_string(),
            Some(&matches),
            &theme,
            None,
            0,
        );
        assert_eq!(text.lines.len(), 1, "truncated mode = single Line");
        let line = &text.lines[0];
        let highlighted = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "e")
            .expect("expected highlighted message span");
        let unhighlighted = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "rror")
            .expect("expected unhighlighted message span");

        assert_eq!(highlighted.style.fg, None);
        assert!(
            highlighted
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert_eq!(unhighlighted.style.fg, Some(Color::Red));
        assert!(
            !unhighlighted
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    fn long_msg_entry() -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq: 1,
            msg: "alpha beta gamma delta epsilon zeta eta theta iota".to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "src-a".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    #[test]
    fn render_item_truncated_mode_is_single_line() {
        let theme = ThemeConfig::default();
        let text = LogPane::render_item(&long_msg_entry(), "1".to_string(), None, &theme, None, 0);
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn render_item_wrapped_mode_emits_multiple_lines_with_hanging_indent() {
        let theme = ThemeConfig::default();
        // indent_column = "1 INFO src-a " = 1 + 1 + 4 + 1 + 5 + 1 = 13 cells
        let indent_column = 13;
        // Pick a narrow wrap_width so the msg definitely wraps.
        let text = LogPane::render_item(
            &long_msg_entry(),
            "1".to_string(),
            None,
            &theme,
            Some(15),
            indent_column,
        );
        assert!(
            text.lines.len() >= 2,
            "expected multi-line wrap, got {} line(s)",
            text.lines.len()
        );

        // First line: starts with the prefix (leading_id "1").
        let first: String = text.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            first.starts_with("1 "),
            "first line should start with prefix, got {first:?}"
        );

        // Continuation lines: must start with at least `indent_column` spaces
        // so the wrapped text aligns under the msg column.
        for (idx, line) in text.lines.iter().enumerate().skip(1) {
            let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                content.starts_with(&" ".repeat(indent_column as usize)),
                "continuation line {idx} missing hanging indent: {content:?}"
            );
        }
    }

    #[test]
    fn render_item_wrapped_mode_pads_prefix_to_indent_column() {
        let theme = ThemeConfig::default();
        // Inflated indent_column to force first-line padding past natural width.
        let indent_column = 30;
        let text = LogPane::render_item(
            &long_msg_entry(),
            "1".to_string(),
            None,
            &theme,
            Some(40),
            indent_column,
        );
        // First-line prefix portion (before msg starts) should be padded to
        // `indent_column` cells. Locate the leading whitespace span.
        let first: String = text.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        // The leading "1 INFO src-a " is 13 cells; pad to 30 means 17 trailing
        // spaces before the msg text begins.
        let trimmed_start = first
            .find("alpha")
            .expect("msg should appear on first line");
        assert_eq!(
            trimmed_start as u16, indent_column,
            "msg should start at column {indent_column}, but starts at {trimmed_start} ({first:?})"
        );
    }

    #[test]
    fn render_item_wrapped_match_highlight_survives_continuation() {
        use ratatui::style::Modifier;
        let theme = ThemeConfig {
            log_match_style: LogMatchStyle::Underline,
            ..ThemeConfig::default()
        };
        // Match characters at indices 0 and 31 (the latter is the 'z' of
        // "zeta", which falls on a continuation line at wrap_width=15).
        let matches = [Match {
            key: "msg".to_string(),
            indices: vec![0, 31],
        }];
        let text = LogPane::render_item(
            &long_msg_entry(),
            "1".to_string(),
            Some(&matches),
            &theme,
            Some(15),
            13,
        );

        let mut underlined_on_continuation = false;
        for (line_idx, line) in text.lines.iter().enumerate() {
            for span in &line.spans {
                let underlined = span.style.add_modifier.contains(Modifier::UNDERLINED);
                if line_idx > 0 && underlined && span.content.contains('z') {
                    underlined_on_continuation = true;
                }
            }
        }
        assert!(
            underlined_on_continuation,
            "expected at least one underlined character on a continuation line (match highlight should survive wrap)"
        );
    }

    #[test]
    fn render_item_falls_back_to_truncated_when_wrap_width_unavailable() {
        let theme = ThemeConfig::default();
        // wrap_width = None simulates the renderer's fallback when
        // inner_area.width <= indent_column.
        let text = LogPane::render_item(&long_msg_entry(), "1".to_string(), None, &theme, None, 13);
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn format_count_compacts_large_values_for_titles() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1K");
        assert_eq!(format_count(1_500), "1.5K");
        assert_eq!(format_count(1_000_000), "1M");
    }

    #[test]
    fn scroll_emits_selected_entry_event() {
        let mut state = TuiState::new(
            &crate::config::tui::TuiConfig::default(),
            &crate::config::search::SearchConfig::default(),
        )
        .expect("tui state");
        let mut events_bus = EventBus::new();
        state.log_pane.set_height(3, &mut state.log_pane_cursor_row);
        state.log_pane.apply_update(
            crate::state::tui_state::log_pane_state::LogPaneUpdate::Tail {
                entries: (1..=5).map(sequenced_entry).collect(),
                retained_bounds: (1, 5),
            },
            &mut state.log_pane_cursor_row,
        );

        LogPane::new().handle_event(
            TuiEvent::Scroll(ScrollDirection::Backward),
            &mut state,
            &mut events_bus,
        );

        match events_bus.tui_event_rx.try_recv().expect("search event") {
            TuiEvent::DispatchLogPaneSearch(Query::History { middle_seq_id, .. }) => {
                assert_eq!(middle_seq_id, 4);
            }
            event => panic!("expected filtered search dispatch event, got {event:?}"),
        }

        match events_bus
            .tui_event_rx
            .try_recv()
            .expect("selected entry event")
        {
            TuiEvent::NewSelectedEntry(Some(selected_entry)) => {
                assert_eq!(selected_entry.entry.seq, 4);
            }
            event => panic!("expected selected entry event, got {event:?}"),
        }
    }

    /// **Spike**: confirm the assumptions the wrapped-mode renderer relies on
    /// from ratatui's `List` widget. Documented in the bigplan as a gating
    /// pre-flight: if either property fails, the wrapped-mode design needs a
    /// manual selection-render pass.
    ///
    /// Assertions:
    /// (a) `highlight_style` applies across **all** visual lines of a multi-line
    ///     selected `ListItem`.
    /// (b) `highlight_symbol` renders only on the **first** line of the
    ///     selected item — continuation lines are not prefixed.
    #[test]
    fn ratatui_list_multiline_highlight_spike() {
        use ratatui::{
            Terminal,
            backend::TestBackend,
            buffer::Buffer,
            style::{Color, Style},
            text::{Line, Text},
            widgets::{List, ListItem, ListState},
        };

        let items = vec![
            ListItem::new(Text::from(vec![Line::raw("alpha-1"), Line::raw("alpha-2")])),
            ListItem::new(Text::from(vec![Line::raw("beta-1"), Line::raw("beta-2")])),
        ];
        let list = List::new(items)
            .highlight_symbol("> ")
            .highlight_style(Style::default().bg(Color::Blue));
        let mut state = ListState::default().with_selected(Some(0));

        let mut terminal = Terminal::new(TestBackend::new(20, 4)).expect("terminal");
        terminal
            .draw(|frame| {
                frame.render_stateful_widget(list, frame.area(), &mut state);
            })
            .expect("draw");

        let buf: &Buffer = terminal.backend().buffer();
        let row = |y: u16| -> String {
            (0..buf.area().width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        let row_bg = |y: u16, x: u16| buf[(x, y)].style().bg;

        // (b) highlight_symbol on first line only.
        assert!(
            row(0).starts_with("> "),
            "expected highlight symbol on line 1, got {:?}",
            row(0)
        );
        assert!(
            !row(1).starts_with("> "),
            "highlight symbol should NOT be on continuation line, got {:?}",
            row(1)
        );

        // (a) highlight_style (bg=Blue) applies across BOTH visual lines of the
        // selected item. Check the first content cell of each row.
        assert_eq!(
            row_bg(0, 2),
            Some(Color::Blue),
            "selected line 1 missing highlight bg"
        );
        assert_eq!(
            row_bg(1, 2),
            Some(Color::Blue),
            "selected line 2 (continuation) missing highlight bg — wrapped mode requires this"
        );

        // Unselected item must not carry the bg.
        assert_ne!(row_bg(2, 0), Some(Color::Blue));
    }

    #[test]
    fn pressing_w_toggles_log_pane_line_wrap() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut state = TuiState::new(
            &crate::config::tui::TuiConfig::default(),
            &crate::config::search::SearchConfig::default(),
        )
        .expect("tui state");
        let mut events_bus = EventBus::new();
        assert!(!state.log_pane.line_wrap());

        let pane = LogPane::new();
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        pane.handle_event(TuiEvent::Input(key), &mut state, &mut events_bus);
        assert!(state.log_pane.line_wrap());

        pane.handle_event(TuiEvent::Input(key), &mut state, &mut events_bus);
        assert!(!state.log_pane.line_wrap());
    }

    #[test]
    fn tui_state_seeds_line_wrap_from_config() {
        let cfg = crate::config::tui::TuiConfig {
            line_wrap: true,
            ..crate::config::tui::TuiConfig::default()
        };
        let state =
            TuiState::new(&cfg, &crate::config::search::SearchConfig::default()).expect("state");
        assert!(state.log_pane.line_wrap());
    }

    #[test]
    fn scroll_does_not_emit_clear_when_selected_seq_is_pending_fetch() {
        let mut state = TuiState::new(
            &crate::config::tui::TuiConfig::default(),
            &crate::config::search::SearchConfig::default(),
        )
        .expect("tui state");
        let mut events_bus = EventBus::new();
        state.log_pane.set_height(3, &mut state.log_pane_cursor_row);
        state.log_pane.apply_update(
            crate::state::tui_state::log_pane_state::LogPaneUpdate::History {
                entries: (3..=5).map(sequenced_entry).collect(),
                retained_bounds: (1, 5),
            },
            &mut state.log_pane_cursor_row,
        );
        state
            .log_pane
            .set_selected_seq(Some(3), &mut state.log_pane_cursor_row);

        LogPane::new().handle_event(
            TuiEvent::Scroll(ScrollDirection::Backward),
            &mut state,
            &mut events_bus,
        );

        match events_bus.tui_event_rx.try_recv().expect("search event") {
            TuiEvent::DispatchLogPaneSearch(Query::History { middle_seq_id, .. }) => {
                assert_eq!(middle_seq_id, 2);
            }
            event => panic!("expected history search dispatch event, got {event:?}"),
        }
        assert!(
            events_bus.tui_event_rx.try_recv().is_err(),
            "pending selected seq should not emit NewSelectedEntry(None)"
        );
    }
}
