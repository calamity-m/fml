use ratatui::{
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::tui::{
    layout::Slot,
    widgets::{FmlWidget, highlight, wrap},
};

pub struct InfoPane {}

impl Default for InfoPane {
    fn default() -> Self {
        Self::new()
    }
}

impl InfoPane {
    pub fn new() -> Self {
        InfoPane {}
    }

    fn stringify_value(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Null => String::new(),
            value => value.to_string(),
        }
    }
}

impl FmlWidget for InfoPane {
    fn slot(&self) -> crate::tui::layout::Slot {
        Slot::InfoPane
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut crate::state::tui_state::TuiState,
    ) {
        // Draw the outer border and title. InfoPane is not focusable so always
        // uses the unfocused (DarkGray, Plain) border.
        let (border_style, border_type) = self.border(&state.focused, &state.selected_theme);
        let block = Block::bordered()
            .title(" Info ")
            .border_style(border_style)
            .border_type(border_type)
            .style(state.selected_theme.surface_style());

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        let base_style = state.selected_theme.surface_style();
        let label_style = base_style.fg(state.selected_theme.primary_accent_fg);
        let value_style = base_style;
        let match_style = base_style.patch(state.selected_theme.match_style());

        let lines = if let Some(selected_entry) = &state.selected_entry {
            let entry = &selected_entry.entry;
            let matches = Some(selected_entry.matches.as_slice());
            let level = entry
                .level
                .map(|level| level.to_string())
                .unwrap_or_else(|| "----".to_string());
            let group = entry.source.group.as_deref().unwrap_or("");
            let mut lines = vec![
                highlight::field_line(
                    "seq",
                    &entry.seq.to_string(),
                    "seq",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                highlight::field_line(
                    "timestamp",
                    &entry.ts.to_rfc3339(),
                    "timestamp",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                highlight::field_line(
                    "level",
                    &level,
                    "level",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                highlight::field_line(
                    "producer",
                    &entry.source.producer,
                    "producer",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                highlight::field_line(
                    "source",
                    &entry.source.display_name,
                    "source",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                highlight::field_line(
                    "group",
                    group,
                    "group",
                    matches,
                    label_style,
                    value_style,
                    match_style,
                ),
                Line::from(vec![Span::styled("message:", label_style)]),
            ];
            lines.extend(wrap::wrap_styled_spans(
                highlight::styled_field(&entry.msg, matches, "msg", value_style, match_style),
                inner_area.width,
                &[],
                false,
            ));

            let mut custom_fields = entry.fields.iter().collect::<Vec<_>>();
            custom_fields.sort_by(|(left, _), (right, _)| left.cmp(right));
            lines.extend(custom_fields.into_iter().map(|(key, value)| {
                highlight::field_line(
                    key,
                    &Self::stringify_value(value),
                    key,
                    matches,
                    label_style,
                    value_style,
                    match_style,
                )
            }));
            lines
        } else {
            vec![Line::styled("No log selected", value_style)]
        };

        let max_offset = lines.len().saturating_sub(usize::from(inner_area.height));
        state.info_pane_scroll_offset = state.info_pane_scroll_offset.min(max_offset);
        let scroll_offset = state.info_pane_scroll_offset.min(usize::from(u16::MAX)) as u16;

        frame.render_widget(
            Paragraph::new(lines)
                .style(state.selected_theme.surface_style())
                .scroll((scroll_offset, 0)),
            inner_area,
        );
    }

    fn handle_event(
        &self,
        _event: crate::event::TuiEvent,
        _state: &mut crate::state::tui_state::TuiState,
        _events_bus: &mut crate::state::events_bus::EventBus,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::{TimeZone, Utc};
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        config::{search::SearchConfig, tui::TuiConfig},
        event::SelectedEntry,
        log::{LogEntry, LogLevel, Source},
        state::tui_state::TuiState,
    };

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
    fn message_uses_full_inner_width_and_wraps() {
        let mut state =
            TuiState::new(&TuiConfig::default(), &SearchConfig::default()).expect("tui state");
        state.selected_entry = Some(SelectedEntry {
            entry: Arc::new(LogEntry {
                seq: 1,
                msg: "alpha beta gamma delta epsilon".to_string(),
                ts: Utc
                    .with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
                    .single()
                    .expect("fixed timestamp"),
                level: Some(LogLevel::Info),
                source: Source {
                    producer: "fake".to_string(),
                    id: "src-a".to_string(),
                    display_name: "src-a".to_string(),
                    group: None,
                },
                fields: HashMap::new(),
            }),
            matches: Vec::new(),
        });

        let mut terminal = Terminal::new(TestBackend::new(24, 16)).expect("terminal");
        terminal
            .draw(|frame| InfoPane::new().render(frame, frame.area(), &mut state))
            .expect("draw");

        let rendered = buffer_to_string(terminal.backend().buffer());

        assert!(rendered.contains("│message:"));
        assert!(rendered.contains("│alpha beta gamma"));
        assert!(rendered.contains("epsilon"));
    }
}
