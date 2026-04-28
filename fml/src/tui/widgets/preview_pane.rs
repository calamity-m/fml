use std::sync::Arc;

use ratatui::{
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState},
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

    fn title(mode: PreviewMode) -> &'static str {
        match mode {
            PreviewMode::Surrounding => " Preview [ SURROUNDING ] ",
        }
    }

    fn render_line(entry: &Arc<LogEntry>, state: &TuiState, is_anchor: bool) -> Line<'static> {
        let mut base_style = state
            .selected_theme
            .surface_style()
            .fg(state.selected_theme.log_row_fg(entry.level));
        if !is_anchor {
            base_style = base_style.add_modifier(Modifier::DIM);
        }
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
            .title(Self::title(state.preview_pane.mode))
            .border_style(
                state
                    .selected_theme
                    .surface_style()
                    .fg(state.selected_theme.border_unfocused_fg),
            )
            .style(state.selected_theme.surface_style());

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        match state.preview_pane.status {
            PreviewStatus::NoSelection => {
                frame.render_widget(
                    ratatui::widgets::Paragraph::new("No log selected")
                        .style(state.selected_theme.surface_style()),
                    inner_area,
                );
            }
            PreviewStatus::Loading => {
                frame.render_widget(
                    ratatui::widgets::Paragraph::new("Loading surrounding logs...")
                        .style(state.selected_theme.surface_style()),
                    inner_area,
                );
            }
            PreviewStatus::AnchorEvicted => {
                frame.render_widget(
                    ratatui::widgets::Paragraph::new("Selected log is no longer retained")
                        .style(state.selected_theme.surface_style()),
                    inner_area,
                );
            }
            PreviewStatus::Ready => {
                let (items, selected) = Self::centered_items(state, inner_area.height as usize);
                let list = List::new(items)
                    .style(state.selected_theme.surface_style())
                    .highlight_symbol("> ")
                    .highlight_style(state.selected_theme.selected_style());
                let mut list_state = ListState::default().with_selected(selected);
                frame.render_stateful_widget(list, inner_area, &mut list_state);
            }
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
}
