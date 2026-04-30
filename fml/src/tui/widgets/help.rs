use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

use crate::{
    state::tui_state::{ActivePopup, TuiState},
    tui::{
        keybinds::{self, HelpSection, KeyActionHint},
        layout::Slot,
        widgets::FmlPopupWidget,
    },
};

pub struct Help {}

impl Default for Help {
    fn default() -> Self {
        Self::new()
    }
}

impl Help {
    pub fn new() -> Self {
        Help {}
    }
}

impl FmlPopupWidget for Help {
    fn popup(&self) -> ActivePopup {
        ActivePopup::Help
    }

    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState) {
        if state.active_popup() != Some(self.popup()) {
            return;
        }

        let popup_area = popup_area(area);
        frame.render_widget(Clear, popup_area);

        let base_style = state.selected_theme.surface_style();
        let block = Block::bordered()
            .title(" Help ")
            .title_alignment(Alignment::Left)
            .border_style(base_style.fg(state.selected_theme.primary_accent_fg))
            .style(base_style);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        frame.render_widget(Paragraph::new(help_lines(state)).style(base_style), inner);
    }
}

fn help_lines(state: &TuiState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_section(&mut lines, "Global", HelpSection::Global, state);

    match state.focused {
        Slot::Main => push_section(&mut lines, "Log pane", HelpSection::LogPane, state),
        Slot::QueryBox => push_section(&mut lines, "Query box", HelpSection::QueryBox, state),
        Slot::InfoPane | Slot::PreviewPane | Slot::StatusBar => {}
    }

    if state.source_selector_is_open() {
        push_section(
            &mut lines,
            "Source selector",
            HelpSection::SourceSelector,
            state,
        );
    } else {
        push_section(&mut lines, "Help popup", HelpSection::HelpPopup, state);
    }

    lines
}

fn push_section(
    lines: &mut Vec<Line<'static>>,
    title: &'static str,
    section: HelpSection,
    state: &TuiState,
) {
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::styled(
        title,
        state
            .selected_theme
            .surface_style()
            .fg(state.selected_theme.primary_accent_fg)
            .add_modifier(Modifier::BOLD),
    ));

    for hint in keybinds::hints_for_section(section) {
        lines.push(render_hint(hint, state));
    }
}

fn render_hint(hint: &KeyActionHint, state: &TuiState) -> Line<'static> {
    let base_style = state.selected_theme.surface_style();
    let key_style = base_style
        .fg(state.selected_theme.primary_accent_fg)
        .add_modifier(Modifier::BOLD);
    let dim_style = if state.selected_theme.status_dim {
        base_style.add_modifier(Modifier::DIM)
    } else {
        base_style
    };

    Line::from(vec![
        Span::styled(format!("  {:<14}", hint.label), key_style),
        Span::styled(hint.title, dim_style),
    ])
}

fn popup_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(20, 56);
    let height = area.height.saturating_sub(4).clamp(8, 22);

    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .split(vertical[1]);

    horizontal[1]
}
