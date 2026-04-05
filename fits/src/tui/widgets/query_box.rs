use ratatui::{
    layout::{Constraint, Layout},
    style::Style,
    text::Span,
    widgets::{Block, Paragraph},
};
use ratatui_textarea::TextArea;
use tracing::{info, trace};

use crate::{
    config::tui::{ThemeConfig, TuiConfig},
    event::TuiEvent,
    state::{AppState, tui_state::TuiState},
    tui::{layout::Slot, widgets::FmlWidget},
};

pub fn query_box_textarea<'a>() -> TextArea<'a> {
    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(Style::default());
    textarea.set_placeholder_text("Enter a query...");
    textarea
}

/// A single-line text input for entering search queries.
///
/// Renders a `>` prompt to the left of the input area. The prompt is a separate
/// widget so the [`TextArea`] never contains it — [`query()`](Self::query)
/// returns pure user input with no stripping required.
pub struct QueryBox {}

impl QueryBox {
    pub fn new() -> Self {
        QueryBox {}
    }

    pub fn border_style(&self, focused: &Slot, theme: &ThemeConfig) -> Style {
        if focused == &Slot::QueryBox {
            return theme.surface_style();
        }

        theme.surface_style().fg(theme.border_unfocused_fg)
    }
}

impl FmlWidget for QueryBox {
    fn slot(&self) -> Slot {
        Slot::QueryBox
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut TuiState,
    ) {
        trace!("render called on QueryBox");

        // Outer border
        let inner = Block::bordered()
            .title(" Query ")
            .border_style(self.border_style(&state.focused, &state.selected_theme))
            .style(state.selected_theme.surface_style());
        let inner_area = inner.inner(area);
        frame.render_widget(inner, area);

        // Split inner area: prompt "> " | textarea
        let chunks = Layout::horizontal([
            Constraint::Length(2), // "> "
            Constraint::Fill(1),   // textarea
        ])
        .split(inner_area);

        // Style our textarea
        state
            .query_box_textarea
            .set_style(state.selected_theme.surface_style());

        // Style our "> " prompt
        let prompt = Paragraph::new(Span::styled(
            "> ",
            state
                .selected_theme
                .surface_style()
                .fg(state.selected_theme.secondary_accent_fg),
        ))
        .style(state.selected_theme.surface_style());

        // Render our "> " prompt
        frame.render_widget(prompt, chunks[0]);
        // Render the actual textarea
        frame.render_widget(&state.query_box_textarea, chunks[1]);
    }

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState) {
        todo!()
    }
}
