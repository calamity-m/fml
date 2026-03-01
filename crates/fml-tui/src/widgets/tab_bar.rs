//! Tab bar widget — renders the strip of open tabs at the top of the screen.

use crate::app::TabState;
use crate::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Tabs, Widget},
};

/// Renders the 1-line strip of open tabs at the top of the screen.
///
/// The active tab is highlighted; a `●` suffix marks tabs with unsaved state.
/// Keybinding hints (`q:quit  ?:help`) are right-aligned in the same row.
pub struct TabBar<'a> {
    tabs: &'a [TabState],
    active: usize,
    _theme: &'a Theme,
}

impl<'a> TabBar<'a> {
    pub fn new(tabs: &'a [TabState], active: usize, theme: &'a Theme) -> Self {
        Self { tabs, active, _theme: theme }
    }
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let labels: Vec<Line> = self
            .tabs
            .iter()
            .map(|tab| {
                let dirty = if tab.dirty { " ●" } else { "" };
                Line::from(format!(" {}{} ", tab.label, dirty))
            })
            .collect();

        Tabs::new(labels)
            .select(self.active)
            .highlight_style(
                Style::default()
                    .bg(ratatui::style::Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .divider("")
            .render(area, buf);

        // Keybinding hints at the right edge
        let hint = " q:quit  ?:help ";
        let hint_x = area.right().saturating_sub(hint.len() as u16);
        buf.set_string(
            hint_x,
            area.y,
            hint,
            Style::default().add_modifier(Modifier::DIM),
        );
    }
}
