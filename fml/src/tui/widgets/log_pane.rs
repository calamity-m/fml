use ratatui::widgets::{
    Block, List, ListItem, ListState, ScrollDirection, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};
use tracing::{debug, error};

use crate::{
    event::{Query, SearchEvent, TuiEvent},
    state::{
        events_bus::EventBus,
        tui_state::{
            TuiState,
            log_pane_state::{LogPaneState, ScrollMode},
        },
    },
    tui::{
        keybinds::{self, StaticKeyAction},
        layout::Slot,
        widgets::FmlWidget,
    },
};

pub struct LogPane {}

impl LogPane {
    pub fn new() -> Self {
        LogPane {}
    }

    fn title(&self, state: &LogPaneState) -> String {
        let base = match state.mode {
            ScrollMode::Tail => "TAIL",
            ScrollMode::History => "HISTORY",
            ScrollMode::Search => "SEARCH",
        };

        format!(" FML [{base}] ")
    }

    fn dispatch_search(query: Option<Query>, events_bus: &mut EventBus) {
        let Some(query) = query else {
            return;
        };

        if let Err(err) = events_bus.search_event_tx.try_send(SearchEvent::Search {
            query,
            sources: Vec::new(),
        }) {
            error!("failed to send search event from log pane - {}", err);
        }
    }
}

impl FmlWidget for LogPane {
    fn slot(&self) -> Slot {
        Slot::Main
    }

    fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
        state: &mut TuiState,
    ) {
        // Draw the outer border and title. `inner_area` is the rect inside the
        // border — this is where the list actually goes.
        let block = Block::bordered()
            .title(self.title(&state.log_pane))
            .border_style(self.border_style(&state.focused, &state.selected_theme))
            .style(state.selected_theme.surface_style());

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        // How many rows fit in the pane. Each ListItem is one line, so this is
        // also the number of visible log entries at any time.
        state
            .log_pane
            .set_height(inner_area.height as usize, &mut state.absolute_cursor);

        // Take the trailing `height` entries from the resolved tail window so
        // the most recent log lines sit at the bottom of the pane.
        let items: Vec<ListItem> = state
            .log_pane
            .visible_items()
            .iter()
            .map(|entry| {
                let level = entry
                    .level
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "----".to_string());
                ListItem::new(format!(
                    "{} {} {} {}",
                    entry.seq, level, entry.source.id, entry.msg
                ))
            })
            .collect();
        // The list itself only knows about display styles. It has no concept of
        // scroll position — that comes from ListState below.
        let list = List::new(items)
            .style(state.selected_theme.surface_style())
            // The selected row gets this style applied on top, giving it a
            // highlighted background to show where the cursor is.
            .highlight_style(
                state
                    .selected_theme
                    .surface_style()
                    .bg(state.selected_theme.log_selected_bg),
            );

        // ListState is constructed fresh every frame — it's just a render detail,
        // not persistent state. `with_selected` tells the List which item in the
        // Vec to highlight.
        //
        // `absolute_cursor - window_start` translates from the full display list
        // index to the index within the Vec<ListItem> slice we just built.
        // These must use the same window_start or the highlight lands on the
        // wrong row.
        let mut list_state =
            ListState::default().with_selected(state.log_pane.selected_visible_index());

        frame.render_stateful_widget(list, inner_area, &mut list_state);

        // Render the scrollbar over the right border of the block. We render on
        // `area` (the outer rect including the border), not `inner_area` — this
        // is what makes it sit on top of the border rather than inside the pane.
        //
        // begin/end symbols are None so there are no arrow caps, keeping the
        // corners of the border intact. The track symbol matches the border
        // character so inactive track segments are invisible — only the thumb
        // stands out.
        if let Some(metrics) = state.log_pane.scrollbar_metrics() {
            let mut scrollbar_state = ScrollbarState::new(metrics.content_length)
                .viewport_content_length(metrics.viewport_content_length)
                .position(metrics.position);

            // Inset by 1 row top and bottom so the track sits between the border
            // corners rather than overwriting them.
            let scrollbar_area = ratatui::prelude::Rect {
                y: area.y + 1,
                height: area.height.saturating_sub(2),
                ..area
            };

            debug!(
                "scrollbar metrics - {:?}, absolute_cursor - {}",
                metrics, state.absolute_cursor
            );

            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(Some("▲"))
                    .end_symbol(Some("▼"))
                    .track_symbol(Some("│"))
                    .thumb_symbol("█"),
                scrollbar_area,
                &mut scrollbar_state,
            );
        }
    }

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus) {
        match event {
            TuiEvent::Scroll(scroll) => {
                debug!("received scroll event - {:?}", scroll);

                let query = match scroll {
                    ScrollDirection::Forward => {
                        state.log_pane.scroll_forward(&mut state.absolute_cursor)
                    }
                    ScrollDirection::Backward => {
                        state.log_pane.scroll_backward(&mut state.absolute_cursor)
                    }
                };
                Self::dispatch_search(query, events_bus);
            }
            TuiEvent::ScrollHead => {
                debug!("received scroll head event");
                let query = state.log_pane.jump_head(&mut state.absolute_cursor);
                Self::dispatch_search(query, events_bus);
            }
            TuiEvent::ScrollTail => {
                debug!("received scroll tail event");
                let query = state.log_pane.jump_tail(&mut state.absolute_cursor);
                Self::dispatch_search(query, events_bus);
            }
            TuiEvent::Input(key) => {
                debug!("received input event - {:?}", key);
                let (static_key, custom_key) = keybinds::match_key(&key, &state.focused);

                // First we process static keys as higher relevance

                let result = match static_key {
                    StaticKeyAction::ScrollUp => events_bus
                        .tui_event_tx
                        .send(TuiEvent::Scroll(ScrollDirection::Backward)),
                    StaticKeyAction::ScrollDown => events_bus
                        .tui_event_tx
                        .send(TuiEvent::Scroll(ScrollDirection::Forward)),
                    _ => Ok(()),
                };

                if let Err(err) = result {
                    error!("failed to send tui event for static key match - {}", err)
                }

                let result = match custom_key {
                    keybinds::CustomizedKeyAction::ScrollHead => {
                        events_bus.tui_event_tx.send(TuiEvent::ScrollHead)
                    }
                    keybinds::CustomizedKeyAction::ScrollTail => {
                        events_bus.tui_event_tx.send(TuiEvent::ScrollTail)
                    }
                    _ => Ok(()),
                };

                if let Err(err) = result {
                    error!("failed to send tui event for static key match - {}", err)
                }
            }
            _ => {}
        }
    }
}
