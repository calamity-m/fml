use std::collections::{BTreeMap, HashSet};

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

use crate::{
    log::{Source, SourceId},
    state::tui_state::{ActivePopup, TuiState},
    tui::widgets::{FmlPopupWidget, PopupSize, header_style, popup_area, render_footer_hints},
};

const UNGROUPED_LABEL: &str = "(ungrouped)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckboxState {
    Checked,
    Unchecked,
    Mixed,
}

impl CheckboxState {
    fn marker(self) -> &'static str {
        match self {
            CheckboxState::Checked => "[x]",
            CheckboxState::Unchecked => "[ ]",
            CheckboxState::Mixed => "[-]",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceSelectorRowKind {
    Producer { source_ids: Vec<SourceId> },
    Group { source_ids: Vec<SourceId> },
    Source(SourceId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSelectorRow {
    pub depth: usize,
    pub label: String,
    pub kind: SourceSelectorRowKind,
    pub checkbox: CheckboxState,
    pub enabled_count: usize,
    pub total_count: usize,
}

pub struct SourceSelector {}

impl Default for SourceSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceSelector {
    pub fn new() -> Self {
        SourceSelector {}
    }
}

impl FmlPopupWidget for SourceSelector {
    fn popup(&self) -> ActivePopup {
        ActivePopup::SourceSelector
    }

    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState) {
        if state.active_popup() != Some(self.popup()) {
            return;
        }

        let rows = source_selector_rows(state);

        // Estimate width to decide narrow mode for header/footer accounting before
        // we know the final popup_area. Match the shared helper's clamp logic.
        let approx_width = if area.width < 44 {
            area.width.saturating_sub(2).max(40u16.min(area.width))
        } else {
            area.width.saturating_sub(4).clamp(40, 64)
        };
        let narrow = approx_width < 48;
        let header_rows: u16 = if narrow { 0 } else { 2 };
        let footer_rows: u16 = 2; // 1 blank spacer + 1 hint line

        let desired_height = header_rows
            .saturating_add(rows.len() as u16)
            .saturating_add(footer_rows)
            .saturating_add(2); // borders

        let popup_area = popup_area(
            area,
            PopupSize {
                min_width: 40,
                max_width: 64,
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

        state.set_source_selector_visible_row_count(rows.len(), visible_rows);

        frame.render_widget(Clear, popup_area);

        let base_style = state.selected_theme.surface_style();
        let block = Block::bordered()
            .title(" Sources ")
            .title_alignment(Alignment::Left)
            .border_style(base_style.fg(state.selected_theme.primary_accent_fg))
            .style(base_style);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let mut lines = Vec::new();
        if !narrow {
            lines.push(Line::styled(
                "Filter visible logs by source",
                header_style(&state.selected_theme),
            ));
            lines.push(Line::from(""));
        }

        let selected = state.source_selector_selected_row();
        let start = state.source_selector.scroll_offset;
        let end = start.saturating_add(visible_rows).min(rows.len());
        for (row_index, row) in rows[start..end].iter().enumerate() {
            lines.push(render_row(
                row,
                start + row_index == selected,
                inner.width as usize,
                narrow,
                base_style,
                state.selected_theme.selected_style(),
            ));
        }

        for _ in 0..visible_rows.saturating_sub(end.saturating_sub(start)) {
            lines.push(Line::from(""));
        }

        lines.push(Line::from(""));
        let footer_hints: &[(&str, &str)] = if narrow {
            &[("space", "toggle"), ("esc", "close")]
        } else {
            &[
                ("space", "toggle"),
                ("a", "all"),
                ("n", "none"),
                ("esc", "close"),
            ]
        };
        lines.push(render_footer_hints(footer_hints, &state.selected_theme));

        frame.render_widget(Paragraph::new(lines).style(base_style), inner);
    }
}

pub fn source_selector_rows(state: &TuiState) -> Vec<SourceSelectorRow> {
    build_rows(
        &state.source_selector.open_sources,
        &state.source_selector.enabled_source_ids,
    )
}

pub fn source_selector_row_count(state: &TuiState) -> usize {
    source_selector_rows(state).len()
}

pub fn source_ids_in_tree_order(sources: &[Source]) -> Vec<SourceId> {
    build_rows(sources, &HashSet::new())
        .into_iter()
        .filter_map(|row| match row.kind {
            SourceSelectorRowKind::Source(source_id) => Some(source_id),
            SourceSelectorRowKind::Producer { .. } | SourceSelectorRowKind::Group { .. } => None,
        })
        .collect()
}

pub fn toggle_selected_row(state: &mut TuiState) {
    let rows = source_selector_rows(state);
    let Some(row) = rows.get(state.source_selector_selected_row()) else {
        return;
    };

    let ids = match &row.kind {
        SourceSelectorRowKind::Producer { source_ids }
        | SourceSelectorRowKind::Group { source_ids } => source_ids.as_slice(),
        SourceSelectorRowKind::Source(source_id) => std::slice::from_ref(source_id),
    };
    let target_enabled = row.checkbox != CheckboxState::Checked;
    set_source_ids_enabled(state, ids, target_enabled);
}

pub fn enable_all_open_sources(state: &mut TuiState) {
    let source_ids: Vec<SourceId> = state
        .source_selector
        .open_sources
        .iter()
        .map(|source| source.id.clone())
        .collect();
    set_source_ids_enabled(state, &source_ids, true);
}

pub fn disable_all_open_sources(state: &mut TuiState) {
    let source_ids: Vec<SourceId> = state
        .source_selector
        .open_sources
        .iter()
        .map(|source| source.id.clone())
        .collect();
    set_source_ids_enabled(state, &source_ids, false);
}

fn build_rows(sources: &[Source], enabled: &HashSet<SourceId>) -> Vec<SourceSelectorRow> {
    let mut producers: BTreeMap<String, BTreeMap<String, Vec<&Source>>> = BTreeMap::new();
    for source in sources {
        producers
            .entry(source.producer.clone())
            .or_default()
            .entry(
                source
                    .group
                    .clone()
                    .unwrap_or_else(|| UNGROUPED_LABEL.to_string()),
            )
            .or_default()
            .push(source);
    }

    let mut rows = Vec::new();
    for (producer, groups) in producers {
        let producer_ids: Vec<&SourceId> = groups
            .values()
            .flat_map(|sources| sources.iter().map(|source| &source.id))
            .collect();
        let (enabled_count, total_count, checkbox) = aggregate(&producer_ids, enabled);
        rows.push(SourceSelectorRow {
            depth: 0,
            label: producer,
            kind: SourceSelectorRowKind::Producer {
                source_ids: producer_ids.iter().map(|id| (*id).clone()).collect(),
            },
            checkbox,
            enabled_count,
            total_count,
        });

        for (group, mut group_sources) in groups {
            group_sources.sort_by(|a, b| {
                a.display_name
                    .cmp(&b.display_name)
                    .then_with(|| a.id.cmp(&b.id))
            });
            let group_ids: Vec<&SourceId> = group_sources.iter().map(|source| &source.id).collect();
            let (enabled_count, total_count, checkbox) = aggregate(&group_ids, enabled);
            rows.push(SourceSelectorRow {
                depth: 1,
                label: group,
                kind: SourceSelectorRowKind::Group {
                    source_ids: group_ids.iter().map(|id| (*id).clone()).collect(),
                },
                checkbox,
                enabled_count,
                total_count,
            });

            for source in group_sources {
                let checked = enabled.contains(&source.id);
                rows.push(SourceSelectorRow {
                    depth: 2,
                    label: source.display_name.clone(),
                    kind: SourceSelectorRowKind::Source(source.id.clone()),
                    checkbox: if checked {
                        CheckboxState::Checked
                    } else {
                        CheckboxState::Unchecked
                    },
                    enabled_count: usize::from(checked),
                    total_count: 1,
                });
            }
        }
    }

    rows
}

fn aggregate(ids: &[&SourceId], enabled: &HashSet<SourceId>) -> (usize, usize, CheckboxState) {
    let total_count = ids.len();
    let enabled_count = ids
        .iter()
        .filter(|id| enabled.contains((*id).as_str()))
        .count();
    let checkbox = match (enabled_count, total_count) {
        (0, _) => CheckboxState::Unchecked,
        (enabled_count, total_count) if enabled_count == total_count => CheckboxState::Checked,
        _ => CheckboxState::Mixed,
    };

    (enabled_count, total_count, checkbox)
}

fn set_source_ids_enabled(state: &mut TuiState, ids: &[SourceId], enabled: bool) {
    for id in ids {
        if enabled {
            state.source_selector.enabled_source_ids.insert(id.clone());
        } else {
            state.source_selector.enabled_source_ids.remove(id);
        }
    }
}

fn render_row(
    row: &SourceSelectorRow,
    selected: bool,
    width: usize,
    narrow: bool,
    base_style: ratatui::style::Style,
    selected_style: ratatui::style::Style,
) -> Line<'static> {
    let pointer = if selected { "> " } else { "  " };
    let indent = "  ".repeat(row.depth);
    let label = format!("{pointer}{indent}{} {}", row.checkbox.marker(), row.label);
    let line = if narrow || matches!(&row.kind, SourceSelectorRowKind::Source(_)) {
        fit_to_width(&label, width)
    } else {
        let count = format!("{}/{}", row.enabled_count, row.total_count);
        pad_with_right_count(&label, &count, width)
    };

    if selected {
        Line::from(Span::styled(line, selected_style))
    } else {
        Line::from(Span::styled(line, base_style))
    }
}

fn pad_with_right_count(label: &str, count: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let min_gap = 1;
    let label_width = label.chars().count();
    let count_width = count.chars().count();
    if label_width + min_gap + count_width > width {
        return fit_to_width(label, width);
    }

    format!(
        "{label}{:gap$}{count}",
        "",
        gap = width - label_width - count_width
    )
}

fn fit_to_width(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        config::{search::SearchConfig, tui::TuiConfig},
        state::tui_state::TuiState,
    };

    fn source(producer: &str, group: Option<&str>, id: &str, display_name: &str) -> Source {
        Source {
            producer: producer.to_string(),
            id: id.to_string(),
            display_name: display_name.to_string(),
            group: group.map(str::to_string),
        }
    }

    fn state(sources: Vec<Source>, enabled: &[&str]) -> TuiState {
        let mut state = TuiState::new(&TuiConfig::default(), &SearchConfig::default()).unwrap();
        state.open_source_selector(&sources);
        state.source_selector.enabled_source_ids = enabled
            .iter()
            .map(|id| id.to_string())
            .collect::<HashSet<_>>();
        state
    }

    fn render(mut state: TuiState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let source_selector = SourceSelector::new();
        terminal
            .draw(|frame| source_selector.render(frame, frame.area(), &mut state))
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
    fn builds_sorted_tree_rows_with_mixed_aggregates() {
        let sources = vec![
            source("fake", Some("backend"), "src-b", "Service B"),
            source("fake", Some("backend"), "src-a", "Service A"),
            source("fake", Some("frontend"), "src-c", "Service C"),
        ];
        let enabled = HashSet::from(["src-b".to_string(), "src-c".to_string()]);

        let rows = build_rows(&sources, &enabled);

        assert_eq!(rows[0].label, "fake");
        assert_eq!(rows[0].checkbox, CheckboxState::Mixed);
        assert_eq!(rows[1].label, "backend");
        assert_eq!(rows[1].checkbox, CheckboxState::Mixed);
        assert_eq!(rows[2].label, "Service A");
        assert_eq!(rows[2].checkbox, CheckboxState::Unchecked);
        assert_eq!(rows[3].label, "Service B");
        assert_eq!(rows[3].checkbox, CheckboxState::Checked);
        assert_eq!(rows[4].label, "frontend");
    }

    #[test]
    fn renders_all_enabled_popup() {
        let sources = vec![
            source("fake", Some("backend"), "src-a", "Service A"),
            source("fake", Some("backend"), "src-b", "Service B"),
            source("fake", Some("frontend"), "src-c", "Service C"),
        ];

        insta::assert_snapshot!(render(state(sources, &["src-a", "src-b", "src-c"]), 80, 24));
    }

    #[test]
    fn renders_partial_popup() {
        let sources = vec![
            source("fake", Some("backend"), "src-a", "Service A"),
            source("fake", Some("backend"), "src-b", "Service B"),
            source("fake", Some("frontend"), "src-c", "Service C"),
        ];

        insta::assert_snapshot!(render(state(sources, &["src-b", "src-c"]), 80, 24));
    }

    #[test]
    fn renders_multiple_producers_and_ungrouped() {
        let sources = vec![
            source("docker", Some("compose-a"), "api", "api"),
            source("docker", None, "standalone", "standalone"),
            source("file", Some("/var/log"), "syslog", "syslog"),
            source("namespace1", Some("deployments"), "web", "web"),
            source("namespace2", Some("deployments"), "worker", "worker"),
        ];

        insta::assert_snapshot!(render(state(sources, &["api", "syslog", "web"]), 88, 26));
    }

    #[test]
    fn renders_narrow_fallback() {
        let sources = vec![
            source("fake", Some("backend"), "src-a", "Service A"),
            source("fake", Some("backend"), "src-b", "Service B"),
            source("fake", Some("frontend"), "src-c", "Service C"),
        ];

        insta::assert_snapshot!(render(state(sources, &["src-b", "src-c"]), 36, 14));
    }

    #[test]
    fn renders_scrolled_tall_tree() {
        let sources = (0..12)
            .map(|index| {
                source(
                    "fake",
                    Some("many"),
                    &format!("src-{index:02}"),
                    &format!("Service {index:02}"),
                )
            })
            .collect::<Vec<_>>();
        let mut state = state(
            sources,
            &["src-00", "src-01", "src-02", "src-03", "src-04", "src-05"],
        );
        state.source_selector.scroll_offset = 6;
        state.source_selector.cursor = 2;

        insta::assert_snapshot!(render(state, 80, 12));
    }
}
