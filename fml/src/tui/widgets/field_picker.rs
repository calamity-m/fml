use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use serde_json::Value;

use crate::{
    state::tui_state::{ActivePopup, TuiState},
    tui::widgets::{FmlPopupWidget, PopupSize, header_style, popup_area, render_footer_hints},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPickerRow {
    pub key: String,
    pub value: String,
    pub selected: bool,
}

pub struct FieldPicker {}

impl Default for FieldPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldPicker {
    pub fn new() -> Self {
        FieldPicker {}
    }
}

impl FmlPopupWidget for FieldPicker {
    fn popup(&self) -> ActivePopup {
        ActivePopup::FieldPicker
    }

    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState) {
        if state.active_popup() != Some(self.popup()) {
            return;
        }

        let rows = field_picker_rows(state);

        let approx_width = if area.width < 44 {
            area.width.saturating_sub(2).max(40u16.min(area.width))
        } else {
            area.width.saturating_sub(4).clamp(40, 72)
        };
        let narrow = approx_width < 48;
        let header_rows: u16 = if narrow { 0 } else { 2 };
        let footer_rows: u16 = 2; // 1 blank spacer + 1 hint line
        let content_rows: u16 = rows.len().max(1) as u16;

        let desired_height = header_rows
            .saturating_add(content_rows)
            .saturating_add(footer_rows)
            .saturating_add(2); // borders

        let popup_area = popup_area(
            area,
            PopupSize {
                min_width: 40,
                max_width: 72,
                desired_height,
                min_height: 8,
            },
        );
        let narrow = popup_area.width < 48;
        let header_rows: usize = if narrow { 0 } else { 2 };
        let footer_rows: usize = 2;
        let inner_height = popup_area.height.saturating_sub(2) as usize;
        let visible_rows = inner_height
            .saturating_sub(header_rows + footer_rows)
            .max(1);

        state.set_field_picker_visible_row_count(rows.len(), visible_rows);

        frame.render_widget(Clear, popup_area);

        let base_style = state.selected_theme.surface_style();
        let block = Block::bordered()
            .title(" Match Fields ")
            .title_alignment(Alignment::Left)
            .border_style(base_style.fg(state.selected_theme.primary_accent_fg))
            .style(base_style);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let mut lines = Vec::new();
        if !narrow {
            lines.push(Line::styled(
                "Choose fields from the selected entry",
                header_style(&state.selected_theme),
            ));
            lines.push(Line::from(""));
        }

        if rows.is_empty() {
            lines.push(Line::styled(
                "Selected entry has no fields",
                header_style(&state.selected_theme),
            ));
            for _ in 1..visible_rows {
                lines.push(Line::from(""));
            }
        } else {
            let selected = state.field_picker_selected_row();
            let start = state.field_picker.scroll_offset;
            let end = start.saturating_add(visible_rows).min(rows.len());
            for (row_index, row) in rows[start..end].iter().enumerate() {
                lines.push(render_row(
                    row,
                    start + row_index == selected,
                    inner.width as usize,
                    base_style,
                    state.selected_theme.selected_style(),
                ));
            }

            for _ in 0..visible_rows.saturating_sub(end.saturating_sub(start)) {
                lines.push(Line::from(""));
            }
        }

        lines.push(Line::from(""));
        let footer_hints: &[(&str, &str)] = if rows.is_empty() {
            &[("esc", "close"), ("ctrl+p", "skip")]
        } else {
            &[
                ("space", "toggle"),
                ("enter", "apply"),
                ("esc", "cancel"),
                ("ctrl+p", "skip"),
            ]
        };
        lines.push(render_footer_hints(footer_hints, &state.selected_theme));

        frame.render_widget(Paragraph::new(lines).style(base_style), inner);
    }
}

pub fn field_picker_rows(state: &TuiState) -> Vec<FieldPickerRow> {
    let Some(selected_entry) = &state.selected_entry else {
        return Vec::new();
    };

    let mut rows = selected_entry
        .entry
        .fields
        .iter()
        .map(|(key, value)| FieldPickerRow {
            key: key.clone(),
            value: compact_value(value),
            selected: state.field_picker.selected_field_keys.contains(key),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows
}

pub fn field_picker_row_count(state: &TuiState) -> usize {
    field_picker_rows(state).len()
}

pub fn toggle_selected_row(state: &mut TuiState) {
    let rows = field_picker_rows(state);
    let Some(row) = rows.get(state.field_picker_selected_row()) else {
        return;
    };

    state.toggle_field_picker_key(&row.key);
}

fn render_row(
    row: &FieldPickerRow,
    selected: bool,
    width: usize,
    base_style: ratatui::style::Style,
    selected_style: ratatui::style::Style,
) -> Line<'static> {
    let pointer = if selected { "> " } else { "  " };
    let marker = if row.selected { "[x]" } else { "[ ]" };
    let label = format!("{pointer}{marker} {} = {}", row.key, row.value);
    let line = fit_to_width(&label, width);

    if selected {
        Line::from(Span::styled(line, selected_style))
    } else {
        Line::from(Span::styled(line, base_style))
    }
}

fn compact_value(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    fit_to_width(&rendered, 48)
}

fn fit_to_width(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    use super::*;
    use crate::{
        config::{search::SearchConfig, tui::TuiConfig},
        event::SelectedEntry,
        log::{LogEntry, LogLevel, Source},
        state::tui_state::TuiState,
    };

    fn selected_entry(fields: HashMap<String, Value>) -> SelectedEntry {
        SelectedEntry {
            entry: Arc::new(LogEntry {
                seq: 1,
                msg: "entry".to_string(),
                ts: Utc::now(),
                level: Some(LogLevel::Info),
                source: Source {
                    producer: "fake".to_string(),
                    id: "src-a".to_string(),
                    display_name: "src-a".to_string(),
                    group: None,
                },
                fields,
            }),
            matches: Vec::new(),
        }
    }

    fn state(fields: HashMap<String, Value>, selected_keys: &[&str]) -> TuiState {
        let mut state = TuiState::new(&TuiConfig::default(), &SearchConfig::default()).unwrap();
        state.selected_entry = Some(selected_entry(fields));
        state.open_field_picker();
        for key in selected_keys {
            state.toggle_field_picker_key(key);
        }
        state
    }

    fn render(mut state: TuiState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let field_picker = FieldPicker::new();
        terminal
            .draw(|frame| field_picker.render(frame, frame.area(), &mut state))
            .expect("draw");
        buffer_to_string(terminal.backend().buffer())
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area();
        let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn builds_sorted_rows_with_compact_json_values() {
        let state = state(
            HashMap::from([
                ("trace".to_string(), json!("abc")),
                ("attempt".to_string(), json!(2)),
                ("ok".to_string(), json!(true)),
            ]),
            &["trace"],
        );

        let rows = field_picker_rows(&state);

        assert_eq!(
            rows,
            vec![
                FieldPickerRow {
                    key: "attempt".to_string(),
                    value: "2".to_string(),
                    selected: false,
                },
                FieldPickerRow {
                    key: "ok".to_string(),
                    value: "true".to_string(),
                    selected: false,
                },
                FieldPickerRow {
                    key: "trace".to_string(),
                    value: "\"abc\"".to_string(),
                    selected: true,
                },
            ]
        );
    }

    #[test]
    fn toggles_selected_row() {
        let mut state = state(
            HashMap::from([
                ("request_id".to_string(), json!("abc")),
                ("status".to_string(), json!(500)),
            ]),
            &[],
        );
        state.field_picker.cursor = 1;

        toggle_selected_row(&mut state);

        assert_eq!(
            state.selected_field_picker_keys(),
            vec!["status".to_string()]
        );
    }

    #[test]
    fn renders_no_fields_message() {
        let rendered = render(state(HashMap::new(), &[]), 80, 24);

        assert!(rendered.contains("Selected entry has no fields"));
        assert!(rendered.contains("esc close"));
        assert!(rendered.contains("ctrl+p skip"));
    }

    #[test]
    fn renders_scrolled_rows() {
        let fields = (0..8)
            .map(|index| (format!("field_{index:02}"), json!(index)))
            .collect::<HashMap<_, _>>();
        let mut state = state(fields, &["field_06"]);
        state.field_picker.scroll_offset = 5;
        state.field_picker.cursor = 1;

        let rendered = render(state, 80, 10);

        assert!(rendered.contains("> [x] field_06 = 6"));
        assert!(rendered.contains("esc cancel  ctrl+p skip"));
    }
}
