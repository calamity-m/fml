use std::sync::Arc;

use ratatui::{
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

use crate::{
    log::LogEntry,
    state::tui_state::{
        TuiState,
        preview_pane_state::{PreviewMode, PreviewStatus},
    },
    tui::{layout::Slot, widgets::FmlWidget},
};

pub struct PreviewPane {}

impl Default for PreviewPane {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewPane {
    pub fn new() -> Self {
        PreviewPane {}
    }

    fn title(mode: &PreviewMode) -> &'static str {
        match mode {
            PreviewMode::Surrounding => " Preview [ SURROUNDING ] ",
            PreviewMode::FieldMatched { .. } => " Preview [ FIELD MATCHED ] ",
        }
    }

    fn status_message(status: PreviewStatus) -> Option<&'static str> {
        match status {
            PreviewStatus::NoSelection => Some("No log selected"),
            PreviewStatus::Loading => Some("Loading surrounding logs..."),
            PreviewStatus::AnchorEvicted => Some("Selected log anchor no longer retained"),
            PreviewStatus::NoMatches => Some("No matching retained logs"),
            PreviewStatus::Ready => None,
        }
    }

    fn entry_style(
        entry: &Arc<LogEntry>,
        state: &TuiState,
        is_anchor: bool,
    ) -> ratatui::style::Style {
        let mut style = if is_anchor {
            state.selected_theme.selected_style()
        } else {
            state.selected_theme.surface_style()
        }
        .fg(state.selected_theme.log_row_fg(entry.level));

        if !is_anchor {
            style = style.add_modifier(Modifier::DIM);
        }

        style
    }

    fn render_line(entry: &Arc<LogEntry>, state: &TuiState, is_anchor: bool) -> Line<'static> {
        let base_style = Self::entry_style(entry, state, is_anchor);
        let level = entry
            .level
            .map(|level| level.to_string())
            .unwrap_or_else(|| "----".to_string());

        Line::from(vec![
            Span::styled(entry.seq.to_string(), base_style),
            Span::styled(" ", base_style),
            Span::styled(level, base_style),
            Span::styled(" ", base_style),
            Span::styled(entry.source.display_name.clone(), base_style),
            Span::styled(" ", base_style),
            Span::styled(entry.msg.clone(), base_style),
        ])
    }

    fn centered_items(state: &TuiState, height: usize) -> (Vec<ListItem<'static>>, Option<usize>) {
        let anchor_row = height / 2;
        let anchor_seq = state.tui_preview_anchor_seq();
        let anchor_index = anchor_seq.and_then(|seq| {
            state
                .preview_pane
                .items()
                .iter()
                .position(|entry| entry.seq == seq)
        });

        let mut items = Vec::with_capacity(height);
        for row in 0..height {
            let entry = anchor_index.and_then(|anchor_index| {
                if row <= anchor_row {
                    let distance = anchor_row - row;
                    anchor_index
                        .checked_sub(distance)
                        .and_then(|idx| state.preview_pane.items().get(idx))
                } else {
                    let distance = row - anchor_row;
                    state.preview_pane.items().get(anchor_index + distance)
                }
            });

            let is_anchor = anchor_index.is_some() && row == anchor_row;
            items.push(ListItem::new(
                entry
                    .map(|entry| Self::render_line(entry, state, is_anchor))
                    .unwrap_or_default(),
            ));
        }

        (items, anchor_index.map(|_| anchor_row))
    }

    fn wrapped_items(state: &TuiState, width: u16, height: usize) -> Vec<Line<'static>> {
        let anchor_seq = state.tui_preview_anchor_seq();
        let anchor_index = anchor_seq.and_then(|seq| {
            state
                .preview_pane
                .items()
                .iter()
                .position(|entry| entry.seq == seq)
        });

        let Some(anchor_index) = anchor_index else {
            return Vec::new();
        };

        let entry_lines = state
            .preview_pane
            .items()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                Self::wrapped_entry_lines(entry, state, width, index == anchor_index)
            })
            .collect::<Vec<_>>();

        let anchor_row = height / 2;
        let mut before = entry_lines[..anchor_index]
            .iter()
            .flat_map(|lines| lines.iter().cloned())
            .collect::<Vec<_>>();
        if before.len() > anchor_row {
            before = before.split_off(before.len() - anchor_row);
        }

        let mut lines = Vec::with_capacity(height);
        lines.extend(std::iter::repeat_with(Line::default).take(anchor_row - before.len()));
        lines.extend(before);
        lines.extend(entry_lines[anchor_index].iter().cloned());

        for line in entry_lines[anchor_index + 1..]
            .iter()
            .flat_map(|lines| lines.iter().cloned())
        {
            if lines.len() >= height {
                break;
            }
            lines.push(line);
        }

        lines.truncate(height);
        lines
    }

    fn wrapped_entry_lines(
        entry: &Arc<LogEntry>,
        state: &TuiState,
        width: u16,
        is_anchor: bool,
    ) -> Vec<Line<'static>> {
        let style = Self::entry_style(entry, state, is_anchor);
        let level = entry
            .level
            .map(|level| level.to_string())
            .unwrap_or_else(|| "----".to_string());
        let marker = if is_anchor { "> " } else { "  " };
        let prefix = format!(
            "{marker}{} {level} {} ",
            entry.seq, entry.source.display_name
        );
        let continuation_prefix = " ".repeat(prefix.chars().count());

        Self::wrap_entry_text(&prefix, &continuation_prefix, &entry.msg, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, style)))
            .collect()
    }

    fn wrap_entry_text(
        prefix: &str,
        continuation_prefix: &str,
        message: &str,
        width: u16,
    ) -> Vec<String> {
        let width = usize::from(width).max(1);
        let prefix_width = prefix.chars().count();

        if prefix_width >= width {
            return Self::wrap_words(&format!("{prefix}{message}"), width, width);
        }

        let first_width = width.saturating_sub(prefix_width).max(1);
        let continuation_width = width
            .saturating_sub(continuation_prefix.chars().count())
            .max(1);
        let message_lines = Self::wrap_words(message, first_width, continuation_width);

        message_lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    format!("{prefix}{line}")
                } else {
                    format!("{continuation_prefix}{line}")
                }
            })
            .collect()
    }

    fn wrap_words(text: &str, first_width: usize, continuation_width: usize) -> Vec<String> {
        let chars = text.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            while chars.get(start).is_some_and(|ch| ch.is_whitespace()) {
                start += 1;
            }

            if start >= chars.len() {
                break;
            }

            let width = if lines.is_empty() {
                first_width
            } else {
                continuation_width
            };
            let hard_end = start.saturating_add(width).min(chars.len());
            let end = if hard_end < chars.len() {
                chars[start..hard_end]
                    .iter()
                    .rposition(|ch| ch.is_whitespace())
                    .filter(|idx| *idx > 0)
                    .map(|idx| start + idx)
                    .unwrap_or(hard_end)
            } else {
                hard_end
            };

            lines.push(chars[start..end].iter().collect());
            start = end;
        }

        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }
}

trait PreviewAnchor {
    fn tui_preview_anchor_seq(&self) -> Option<u64>;
}

impl PreviewAnchor for TuiState {
    fn tui_preview_anchor_seq(&self) -> Option<u64> {
        self.preview_pane.anchor_seq
    }
}

impl FmlWidget for PreviewPane {
    fn slot(&self) -> crate::tui::layout::Slot {
        Slot::PreviewPane
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut TuiState,
    ) {
        let block = Block::bordered()
            .title(Self::title(&state.preview_pane.mode))
            .border_style(
                state
                    .selected_theme
                    .surface_style()
                    .fg(state.selected_theme.border_unfocused_fg),
            )
            .style(state.selected_theme.surface_style());

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        if let Some(message) = Self::status_message(state.preview_pane.status) {
            frame.render_widget(
                Paragraph::new(message).style(state.selected_theme.surface_style()),
                inner_area,
            );
            return;
        }

        if state.line_wrap {
            let lines = Self::wrapped_items(state, inner_area.width, inner_area.height as usize);
            frame.render_widget(
                Paragraph::new(lines).style(state.selected_theme.surface_style()),
                inner_area,
            );
        } else {
            let (items, selected) = Self::centered_items(state, inner_area.height as usize);
            let list = List::new(items)
                .style(state.selected_theme.surface_style())
                .highlight_symbol("> ")
                .highlight_style(state.selected_theme.selected_style());
            let mut list_state = ListState::default().with_selected(selected);
            frame.render_stateful_widget(list, inner_area, &mut list_state);
        }
    }

    fn handle_event(
        &self,
        _event: crate::event::TuiEvent,
        _state: &mut TuiState,
        _events_bus: &mut crate::state::events_bus::EventBus,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use ratatui::{Terminal, backend::TestBackend, prelude::Rect};

    use super::*;
    use crate::{
        config::{search::SearchConfig, tui::TuiConfig},
        log::{LogLevel, Source},
    };

    fn entry(seq: u64) -> Arc<LogEntry> {
        entry_with_msg(seq, &format!("entry {seq}"))
    }

    fn entry_with_msg(seq: u64, msg: &str) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq,
            msg: msg.to_string(),
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

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn selected_anchor_renders_in_middle_row() {
        let mut terminal = Terminal::new(TestBackend::new(32, 7)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.preview_pane.start_surrounding(3);
        state
            .preview_pane
            .apply_surrounding(3, vec![entry(1), entry(2), entry(3), entry(4), entry(5)]);

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 32, 7), &mut state))
            .expect("draw");

        let rendered = buffer_to_string(terminal.backend().buffer());
        let lines = rendered.lines().collect::<Vec<_>>();

        assert!(lines[1].contains("1 INFO src-a entry 1"));
        assert!(lines[2].contains("2 INFO src-a entry 2"));
        assert!(lines[3].contains("> 3 INFO src-a entry 3"));
        assert!(lines[4].contains("4 INFO src-a entry 4"));
        assert!(lines[5].contains("5 INFO src-a entry 5"));
    }

    #[test]
    fn missing_context_does_not_shift_anchor() {
        let mut terminal = Terminal::new(TestBackend::new(32, 7)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.preview_pane.start_surrounding(1);
        state
            .preview_pane
            .apply_surrounding(1, vec![entry(1), entry(2), entry(3)]);

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 32, 7), &mut state))
            .expect("draw");

        let rendered = buffer_to_string(terminal.backend().buffer());
        let lines = rendered.lines().collect::<Vec<_>>();

        assert!(!lines[1].contains("INFO"));
        assert!(!lines[2].contains("INFO"));
        assert!(lines[3].contains("> 1 INFO src-a entry 1"));
        assert!(lines[4].contains("2 INFO src-a entry 2"));
        assert!(lines[5].contains("3 INFO src-a entry 3"));
    }

    #[test]
    fn expanded_preview_wraps_long_anchor_snapshot() {
        let mut terminal = Terminal::new(TestBackend::new(48, 10)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.line_wrap = true;
        state.preview_pane.start_active_mode(2);
        state.preview_pane.apply_surrounding(
            2,
            vec![
                entry_with_msg(1, "short before"),
                entry_with_msg(
                    2,
                    "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda",
                ),
                entry_with_msg(3, "short after"),
            ],
        );

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 48, 10), &mut state))
            .expect("draw");

        insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
    }

    #[test]
    fn expanded_preview_clips_overflow_lines_snapshot() {
        let mut terminal = Terminal::new(TestBackend::new(44, 8)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.line_wrap = true;
        state.preview_pane.start_active_mode(3);
        state.preview_pane.apply_surrounding(
            3,
            vec![
                entry_with_msg(1, "old context alpha beta gamma delta epsilon"),
                entry_with_msg(2, "near context zeta eta theta iota kappa lambda mu"),
                entry_with_msg(3, "anchor message nu xi omicron pi rho sigma tau"),
                entry_with_msg(4, "future context upsilon phi chi psi omega"),
                entry_with_msg(5, "hidden future line"),
            ],
        );

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 44, 8), &mut state))
            .expect("draw");

        insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
    }

    #[test]
    fn expanded_anchor_rows_use_selected_style() {
        let mut terminal = Terminal::new(TestBackend::new(48, 10)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.line_wrap = true;
        state.preview_pane.start_active_mode(2);
        state.preview_pane.apply_surrounding(
            2,
            vec![
                entry(1),
                entry_with_msg(2, "alpha beta gamma delta epsilon zeta eta theta"),
                entry(3),
            ],
        );

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 48, 10), &mut state))
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let selected_bg = state
            .selected_theme
            .selected_style()
            .bg
            .expect("selected style has a background");
        let anchor_rows = (0..buffer.area().height)
            .filter(|y| {
                let row = (0..buffer.area().width)
                    .map(|x| buffer[(x, *y)].symbol())
                    .collect::<String>();
                row.contains("> 2 INFO") || row.contains("zeta eta")
            })
            .collect::<Vec<_>>();

        assert!(anchor_rows.len() >= 2);
        for row in anchor_rows {
            assert_eq!(buffer[(1, row)].bg, selected_bg);
        }
    }

    #[test]
    fn expanded_preview_wraps_in_narrow_pane() {
        let mut terminal = Terminal::new(TestBackend::new(26, 8)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.line_wrap = true;
        state.preview_pane.start_active_mode(1);
        state
            .preview_pane
            .apply_surrounding(1, vec![entry_with_msg(1, "alpha beta gamma delta")]);

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 26, 8), &mut state))
            .expect("draw");

        let rendered = buffer_to_string(terminal.backend().buffer());

        assert!(rendered.contains("> 1 INFO"));
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta"));
    }

    #[test]
    fn expanded_anchor_evicted_renders_status_message() {
        let mut terminal = Terminal::new(TestBackend::new(48, 5)).expect("terminal");
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.line_wrap = true;
        state.preview_pane.start_active_mode(2);
        state
            .preview_pane
            .apply_surrounding(2, vec![entry(1), entry(3)]);

        terminal
            .draw(|frame| PreviewPane::new().render(frame, Rect::new(0, 0, 48, 5), &mut state))
            .expect("draw");

        let rendered = buffer_to_string(terminal.backend().buffer());

        assert!(rendered.contains("Selected log anchor no longer retained"));
    }
}
