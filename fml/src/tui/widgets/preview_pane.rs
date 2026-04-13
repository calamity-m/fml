use ratatui::widgets::Block;

use crate::tui::{layout::Slot, widgets::FmlWidget};

pub struct PreviewPane {}

impl PreviewPane {
    pub fn new() -> Self {
        PreviewPane {}
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
        state: &mut crate::state::tui_state::TuiState,
    ) {
        // Draw the outer border and title.
        let block = Block::bordered()
            .title(" Preview [ MODE ] ")
            .border_style(
                state
                    .selected_theme
                    .surface_style()
                    .fg(state.selected_theme.border_unfocused_fg),
            )
            .style(state.selected_theme.surface_style());

        let _inner_area = block.inner(area);
        frame.render_widget(block, area);
    }

    fn handle_event(
        &self,
        event: crate::event::TuiEvent,
        state: &mut crate::state::tui_state::TuiState,
        events_bus: &mut crate::state::events_bus::EventBus,
    ) {
        todo!()
    }
}
