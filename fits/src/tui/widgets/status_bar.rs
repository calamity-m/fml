use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use tracing::trace;

use crate::{
    event::TuiEvent,
    state::tui_state::TuiState,
    tui::{layout::Slot, widgets::FmlWidget},
};

pub struct StatusBar {}

struct StatusBarHint {
    pub title: &'static str,
    pub label: String,
}

impl StatusBar {
    pub fn new() -> Self {
        StatusBar {}
    }

    fn visible_hints(&self) -> Vec<StatusBarHint> {
        let mut visible = Vec::new();

        // Quit
        visible.push(StatusBarHint {
            title: "Quit",
            label: "ctrl+c".to_string(),
        });

        // Help
        visible.push(StatusBarHint {
            title: "Help",
            label: "?".to_string(),
        });

        visible
    }
}

impl FmlWidget for StatusBar {
    fn slot(&self) -> Slot {
        Slot::StatusBar
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut TuiState,
    ) {
        trace!("draw called on StatusBar");

        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        let chunks = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(version.len() as u16 + 1),
        ])
        .split(area);

        let base_style = state.selected_theme.surface_style();
        let key_style = base_style
            .fg(state.selected_theme.primary_accent_fg)
            .add_modifier(Modifier::BOLD);
        let hints = self.visible_hints();
        let mut spans = Vec::new();
        for (index, hint) in hints.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled(" | ", base_style));
            }
            spans.push(Span::styled(format!("{} ", hint.title), base_style));
            spans.push(Span::styled(format!("<{}>", hint.label), key_style));
        }

        frame.render_widget(Block::default().style(base_style), area);
        frame.render_widget(Paragraph::new(Line::from(spans)), chunks[0]);
        frame.render_widget(
            Paragraph::new(format!(" {version}"))
                .alignment(Alignment::Right)
                .style(base_style),
            chunks[1],
        );
    }

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState) {
        todo!()
    }
}
