use std::time::Instant;

use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};
use tracing::trace;

use crate::{
    event::TuiEvent,
    state::{events_bus::EventBus, tui_state::TuiState},
    tui::{keybinds, layout::Slot, widgets::FmlWidget},
};

pub struct StatusBar {}

struct StatusBarHint {
    pub title: &'static str,
    pub label: String,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        StatusBar {}
    }

    fn visible_hints(&self) -> Vec<StatusBarHint> {
        vec![
            StatusBarHint {
                title: "Quit",
                label: keybinds::primary_label("Quit")
                    .unwrap_or("ctrl+c / q")
                    .to_string(),
            },
            StatusBarHint {
                title: "Help",
                label: keybinds::primary_label("Help").unwrap_or("?").to_string(),
            },
        ]
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
        let dim_style = if state.selected_theme.status_dim {
            base_style.add_modifier(Modifier::DIM)
        } else {
            base_style
        };
        let key_style = base_style
            .fg(state.selected_theme.primary_accent_fg)
            .add_modifier(Modifier::BOLD);

        let now = Instant::now();
        let transient = state.status_message(now).map(str::to_string);
        let select_mode = state.select_mode;

        let mut spans = Vec::new();

        // A transient message (e.g. "sent yank … — check clipboard") replaces
        // the normal keybind hints for its TTL. Once it expires the hints
        // reappear automatically because status_message() returns None.
        if let Some(msg) = transient {
            spans.push(Span::styled(msg, key_style));
        } else {
            let hints = self.visible_hints();
            for (index, hint) in hints.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::styled(" | ", dim_style));
                }
                spans.push(Span::styled(format!("{} ", hint.title), dim_style));
                spans.push(Span::styled(format!("<{}>", hint.label), key_style));
            }
        }

        // [SELECT] is appended regardless of whether a transient message is
        // shown so the mode indicator is always visible while active.
        if select_mode {
            if !spans.is_empty() {
                spans.push(Span::styled(" | ", dim_style));
            }
            spans.push(Span::styled("[SELECT]", key_style));
        }

        frame.render_widget(Block::default().style(base_style), area);
        frame.render_widget(
            Paragraph::new(Line::from(spans)).block(Block::default().padding(Padding {
                left: 1,
                right: 0,
                top: 0,
                bottom: 0,
            })),
            chunks[0],
        );
        frame.render_widget(
            Paragraph::new(version.to_string())
                .alignment(Alignment::Right)
                .style(base_style)
                .block(Block::default().padding(Padding {
                    left: 0,
                    right: 1,
                    top: 0,
                    bottom: 0,
                })),
            chunks[1],
        );
    }

    fn handle_event(&self, _event: TuiEvent, _state: &mut TuiState, _events_bus: &mut EventBus) {}
}
