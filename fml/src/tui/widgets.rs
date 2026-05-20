use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
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

pub mod field_picker;
pub mod help;
pub mod highlight;
pub mod info_pane;
pub mod log_pane;
pub mod preview_pane;
pub mod query_box;
pub mod source_selector;
pub mod status_bar;
pub mod wrap;

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

/// Sizing inputs for [`popup_area`].
pub struct PopupSize {
    pub min_width: u16,
    pub max_width: u16,
    /// Content-driven preferred height (including borders). Capped at 80% of `area.height`.
    pub desired_height: u16,
    pub min_height: u16,
}

/// Compute a centered popup [`Rect`] with a consistent geometry across all popups.
///
/// Width clamps to `[min_width, max_width]` with a 4-cell margin on roomy terminals,
/// falling back to a 2-cell margin (down to `min_width`) on narrow terminals. Height
/// shrinks to `desired_height` but never exceeds 80% of the available area, and never
/// drops below `min_height`.
pub fn popup_area(area: Rect, size: PopupSize) -> Rect {
    let width = if area.width < size.min_width.saturating_add(4) {
        area.width.saturating_sub(2).max(1)
    } else {
        area.width
            .saturating_sub(4)
            .clamp(size.min_width, size.max_width)
    };

    let height_cap = ((area.height as f32) * 0.8).round() as u16;
    let height = size
        .desired_height
        .clamp(size.min_height, height_cap.max(size.min_height))
        .min(area.height.saturating_sub(2).max(1));

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

/// Bold accent style used for popup section headers and the help popup's section titles.
pub fn header_style(theme: &ThemeConfig) -> Style {
    theme
        .surface_style()
        .fg(theme.primary_accent_fg)
        .add_modifier(Modifier::BOLD)
}

/// Render an inline single-line footer of `(key, description)` hints, matching the
/// help popup's per-hint styling: bold accent key + dim description, two spaces between
/// pairs.
pub fn render_footer_hints(hints: &[(&str, &str)], theme: &ThemeConfig) -> Line<'static> {
    let base = theme.surface_style();
    let key_style = base
        .fg(theme.primary_accent_fg)
        .add_modifier(Modifier::BOLD);
    let desc_style = if theme.status_dim {
        base.add_modifier(Modifier::DIM)
    } else {
        base
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(hints.len() * 4);
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", base));
        }
        spans.push(Span::styled((*key).to_string(), key_style));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled((*desc).to_string(), desc_style));
    }
    Line::from(spans)
}
