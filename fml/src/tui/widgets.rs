use ratatui::{Frame, buffer::Buffer, layout::Rect, style::Style};

use crate::{
    config::tui::{ThemeConfig, TuiConfig},
    event::TuiEvent,
    state::{AppState, events_bus::EventBus, tui_state::TuiState},
    tui::layout::Slot,
};

pub mod info_pane;
pub mod log_pane;
pub mod preview_pane;
pub mod query_box;
pub mod status_bar;

pub trait FmlWidget {
    /// The layout [`Slot`] this widget renders into.
    fn slot(&self) -> Slot;

    /// Renders the widget onto the given frame within the specified area
    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState);

    /// Handle tui event
    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus);

    fn border_style(&self, focused: &Slot, theme: &ThemeConfig) -> Style {
        // When this pane has focus, highlight the border with the primary accent color.
        // When unfocused, use the terminal default so the focused pane stands out.
        if *focused == self.slot() {
            return theme.surface_style().fg(theme.primary_accent_fg);
        }

        theme.surface_style().fg(theme.border_unfocused_fg)
    }
}
