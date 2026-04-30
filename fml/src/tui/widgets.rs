use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    widgets::BorderType,
};

use crate::{
    config::tui::ThemeConfig,
    event::TuiEvent,
    state::{
        events_bus::EventBus,
        tui_state::{ActivePopup, TuiState},
    },
    tui::layout::Slot,
};

pub mod help;
pub mod highlight;
pub mod info_pane;
pub mod log_pane;
pub mod preview_pane;
pub mod query_box;
pub mod source_selector;
pub mod status_bar;

pub trait FmlWidget {
    /// The layout [`Slot`] this widget renders into.
    fn slot(&self) -> Slot;

    /// Renders the widget onto the given frame within the specified area
    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState);

    /// Handle tui event
    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus);

    /// Returns the border style and type for this widget based on focus state.
    ///
    /// Focused panes get a bold accent-coloured thick border; unfocused panes
    /// get a plain border in the configured unfocused foreground colour.
    fn border(&self, focused: &Slot, theme: &ThemeConfig) -> (Style, BorderType) {
        if *focused == self.slot() {
            (
                theme
                    .surface_style()
                    .fg(theme.primary_accent_fg)
                    .add_modifier(Modifier::BOLD),
                BorderType::Thick,
            )
        } else {
            (
                theme.surface_style().fg(theme.border_unfocused_fg),
                BorderType::Plain,
            )
        }
    }
}

pub trait FmlPopupWidget {
    /// The popup this widget renders.
    fn popup(&self) -> ActivePopup;

    /// Renders the popup over the full terminal area.
    fn render(&self, frame: &mut Frame, area: Rect, state: &mut TuiState);
}
