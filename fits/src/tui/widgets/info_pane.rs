use ratatui::widgets::Block;

use crate::tui::{layout::Slot, widgets::FmlWidget};

pub struct InfoPane {}

impl InfoPane {
    pub fn new() -> Self {
        InfoPane {}
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
        // Draw the outer border and title.
        let block = Block::bordered()
            .title(" Info ")
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
