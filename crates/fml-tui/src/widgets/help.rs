//! Help popup — centred floating overlay listing all keybindings.
//!
//! Toggle with `?`; close with `?` or `Escape`.

use crate::theme::Theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Widget},
};

pub struct HelpPopup<'a> {
    _theme: &'a Theme,
}

impl<'a> HelpPopup<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self { _theme: theme }
    }
}

impl Widget for HelpPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup = centered_rect(80, 20, area);
        Clear.render(popup, buf);

        let block = Block::bordered()
            .title(" fml — keybindings (? to close) ")
            .border_style(Style::default().add_modifier(Modifier::BOLD));

        let inner = block.inner(popup);
        block.render(popup, buf);

        const BINDINGS: &[(&str, &str)] = &[
            ("q  /  Ctrl+c", "Quit / close tab"),
            ("Tab", "Cycle focus: tree → stream → query"),
            ("/", "Focus query bar"),
            ("Escape", "Return focus from query bar"),
            ("↑ k  /  ↓ j", "Navigate tree or scroll list"),
            ("← h  /  → l", "Collapse / expand tree node"),
            ("Space", "Toggle producer selection"),
            ("Enter", "Expand/collapse tree node"),
            ("PageUp  /  Ctrl+u", "Scroll log stream up"),
            ("PageDown / Ctrl+d", "Scroll log stream down"),
            ("G", "Jump to log tail and resume"),
            ("]", "Increase search greed level"),
            ("[", "Decrease search greed level"),
            ("?", "Toggle this help popup"),
        ];

        let lines: Vec<Line> = BINDINGS
            .iter()
            .map(|(key, desc)| {
                Line::from(vec![
                    Span::styled(
                        format!("  {:<22}", key),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(*desc),
                ])
            })
            .collect();

        Paragraph::new(lines).render(inner, buf);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
