use ratatui::{
    layout::{Constraint, Layout},
    style::Style,
    text::Span,
    widgets::{Block, Paragraph},
};
use ratatui_textarea::TextArea;
use tracing::{debug, trace};

use crate::{
    event::{Query, SearchEvent, SearchTarget, TuiEvent},
    state::{events_bus::EventBus, tui_state::TuiState},
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

impl Default for QueryBox {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryBox {
    pub fn new() -> Self {
        QueryBox {}
    }

    fn query_text(state: &TuiState) -> String {
        state
            .query_box_textarea
            .lines()
            .join("\n")
            .trim()
            .to_string()
    }

    fn dispatch_tail(state: &mut TuiState, events_bus: &mut EventBus) {
        // Clearing the box is an explicit mode switch back to live logs, so any
        // delayed fuzzy request must lose even if its timer is about to fire.
        if let Some(handle) = state.query_box_debounce_handle.take() {
            handle.abort();
        }

        if state.query_box_last_dispatched_query.is_empty() {
            return;
        }

        state.query_box_last_dispatched_query.clear();
        if let Err(err) = events_bus.search_event_tx.try_send(SearchEvent::Search {
            target: SearchTarget::LogPane,
            query: Query::Tail,
            sources: Vec::new(),
        }) {
            debug!("failed to dispatch tail search from query box - {}", err);
        }
    }

    fn dispatch_debounced_fuzzy(term: String, state: &mut TuiState, events_bus: &mut EventBus) {
        // Fuzzy scoring can be expensive over large retained buffers. Debouncing
        // at the UI edge keeps typing responsive and avoids starting searches
        // for transient prefixes the user never intended to inspect.
        if state.query_box_last_dispatched_query == term {
            return;
        }

        if let Some(handle) = state.query_box_debounce_handle.take() {
            handle.abort();
        }

        let tx = events_bus.search_event_tx.clone();
        let debounce_ms = state.fuzzy_debounce_ms;
        let dispatched_term = term.clone();
        state.query_box_last_dispatched_query = term;
        state.query_box_debounce_handle = Some(tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;
            if let Err(err) = tx
                .send(SearchEvent::Search {
                    target: SearchTarget::LogPane,
                    query: Query::Fuzzy(dispatched_term),
                    sources: Vec::new(),
                })
                .await
            {
                debug!("failed to dispatch fuzzy search from query box - {}", err);
            }
        }));
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

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus) {
        debug!("handling event for query_box - {:?}", event);

        if let TuiEvent::Input(key) = event {
            let before = Self::query_text(state);
            let changed = state.query_box_textarea.input(key);
            let after = Self::query_text(state);

            // TextArea handles navigation keys too; only content changes
            // should affect search mode.
            if !changed || before == after {
                return;
            }

            if after.is_empty() {
                Self::dispatch_tail(state, events_bus);
            } else {
                Self::dispatch_debounced_fuzzy(after, state, events_bus);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::{
        config::{search::SearchConfig, tui::TuiConfig},
        state::events_bus::EventBus,
    };

    fn key(code: KeyCode) -> TuiEvent {
        TuiEvent::Input(KeyEvent::new(code, KeyModifiers::NONE))
    }

    async fn recv_search(events_bus: &mut EventBus) -> SearchEvent {
        tokio::time::timeout(Duration::from_secs(1), events_bus.search_event_rx.recv())
            .await
            .expect("timed out waiting for search event")
            .expect("search channel closed")
    }

    #[tokio::test]
    async fn debounce_dispatches_only_latest_fuzzy_query() {
        let search_config = SearchConfig {
            fuzzy_debounce_ms: 10,
            ..SearchConfig::default()
        };
        let mut state = TuiState::new(&TuiConfig::default(), &search_config).unwrap();
        let mut events_bus = EventBus::new();
        let widget = QueryBox::new();

        widget.handle_event(key(KeyCode::Char('e')), &mut state, &mut events_bus);
        widget.handle_event(key(KeyCode::Char('r')), &mut state, &mut events_bus);
        widget.handle_event(key(KeyCode::Char('r')), &mut state, &mut events_bus);

        match recv_search(&mut events_bus).await {
            SearchEvent::Search {
                target,
                query: Query::Fuzzy(term),
                sources,
            } => {
                assert_eq!(target, SearchTarget::LogPane);
                assert_eq!(term, "err");
                assert!(sources.is_empty());
            }
            event => panic!("unexpected event: {event:?}"),
        }

        assert!(
            tokio::time::timeout(Duration::from_millis(40), events_bus.search_event_rx.recv())
                .await
                .is_err(),
            "superseded fuzzy query should not dispatch"
        );
    }

    #[tokio::test]
    async fn empty_query_cancels_fuzzy_and_dispatches_tail() {
        let search_config = SearchConfig {
            fuzzy_debounce_ms: 100,
            ..SearchConfig::default()
        };
        let mut state = TuiState::new(&TuiConfig::default(), &search_config).unwrap();
        let mut events_bus = EventBus::new();
        let widget = QueryBox::new();

        widget.handle_event(key(KeyCode::Char('e')), &mut state, &mut events_bus);
        widget.handle_event(key(KeyCode::Backspace), &mut state, &mut events_bus);

        match recv_search(&mut events_bus).await {
            SearchEvent::Search {
                target,
                query: Query::Tail,
                sources,
            } => {
                assert_eq!(target, SearchTarget::LogPane);
                assert!(sources.is_empty());
            }
            event => panic!("unexpected event: {event:?}"),
        }

        assert!(
            tokio::time::timeout(
                Duration::from_millis(140),
                events_bus.search_event_rx.recv()
            )
            .await
            .is_err(),
            "cancelled fuzzy query should not dispatch"
        );
    }
}
