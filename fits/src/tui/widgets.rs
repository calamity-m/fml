use ratatui::{Frame, buffer::Buffer, layout::Rect};

use crate::{
    config::tui::TuiConfig,
    event::TuiEvent,
    state::{AppState, tui_state::TuiState},
    tui::layout::Slot,
};

pub mod log_pane;
pub mod query_box;
pub mod scrollable;
pub mod status_bar;

pub trait FmlWidget {
    /// The layout [`Slot`] this widget renders into.
    fn slot(&self) -> Slot;

    /// Renders the widget onto the given frame within the specified area
    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState);

    /// Handle tui event
    fn handle_event(&self, event: TuiEvent, state: &mut TuiState);
}
