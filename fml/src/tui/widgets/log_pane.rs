use std::sync::Arc;

use ratatui::{
    text::{Line, Span},
    widgets::{
        Block, List, ListItem, ListState, ScrollDirection, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use tracing::{debug, error};

use crate::{
    config::tui::ThemeConfig,
    event::{Match, Query, SearchEvent, SearchTarget, TuiEvent},
    log::LogEntry,
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
        widgets::{FmlWidget, highlight},
    },
};

pub struct LogPane {}

impl Default for LogPane {
    fn default() -> Self {
        Self::new()
    }
}

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
            target: SearchTarget::LogPane,
            query,
            sources: Vec::new(),
        }) {
            error!("failed to send search event from log pane - {}", err);
        }
    }

    fn render_line(
        entry: &Arc<LogEntry>,
        leading_id: String,
        matches: Option<&[Match]>,
        theme: &ThemeConfig,
    ) -> Line<'static> {
        let base_style = theme.surface_style().fg(theme.log_row_fg(entry.level));
        let match_style = theme.surface_style().patch(theme.match_style());
        let level = entry
            .level
            .map(|l| l.to_string())
            .unwrap_or_else(|| "----".to_string());

        let mut spans = Vec::new();
        spans.push(Span::styled(leading_id, base_style));
        spans.push(Span::styled(" ", base_style));
        spans.extend(highlight::styled_field(
            &level,
            matches,
            "level",
            base_style,
            match_style,
        ));
        spans.push(Span::styled(" ", base_style));
        spans.extend(highlight::styled_field(
            &entry.source.display_name,
            matches,
            "source",
            base_style,
            match_style,
        ));
        spans.push(Span::styled(" ", base_style));
        spans.extend(highlight::styled_field(
            &entry.msg,
            matches,
            "msg",
            base_style,
            match_style,
        ));

        Line::from(spans)
    }

    fn dispatch_selected_entry(state: &TuiState, events_bus: &mut EventBus) {
        if let Err(err) = events_bus
            .tui_event_tx
            .send(TuiEvent::NewSelectedEntry(state.log_pane.selected_entry()))
        {
            error!(
                "failed to send selected entry event from log pane - {}",
                err
            );
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
            .set_height(inner_area.height as usize, &mut state.log_pane_cursor_row);

        // Render whatever domain the state resolved for this mode: retained
        // sequence order for tail/history, rank order for search.
        let items: Vec<ListItem> = state
            .log_pane
            .visible_items()
            .iter()
            .enumerate()
            .map(|entry| {
                let (idx, entry) = entry;
                let leading_id = if state.log_pane.mode == ScrollMode::Search {
                    // In search mode the useful coordinate is rank position,
                    // not the underlying log seq, because navigation is over
                    // fuzzy results rather than retained log order.
                    state
                        .log_pane
                        .view_start()
                        .saturating_add(idx)
                        .saturating_add(1)
                        .to_string()
                } else {
                    entry.seq.to_string()
                };
                let matches = (state.log_pane.mode == ScrollMode::Search)
                    .then(|| state.log_pane.fuzzy_matches_for(entry.seq))
                    .flatten();
                ListItem::new(Self::render_line(
                    entry,
                    leading_id,
                    matches,
                    &state.selected_theme,
                ))
            })
            .collect();
        // The list itself only knows about display styles. It has no concept of
        // scroll position — that comes from ListState below.
        let list = List::new(items)
            .style(state.selected_theme.surface_style())
            .highlight_symbol("> ")
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
        // The selected sequence is translated into the Vec<ListItem> slice we
        // just built. These must use the same view window or the highlight
        // lands on the wrong row.
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
                "scrollbar metrics - {:?}, log_pane_cursor_row - {}",
                metrics, state.log_pane_cursor_row
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
                    ScrollDirection::Forward => state
                        .log_pane
                        .scroll_forward(&mut state.log_pane_cursor_row),
                    ScrollDirection::Backward => state
                        .log_pane
                        .scroll_backward(&mut state.log_pane_cursor_row),
                };
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
            }
            TuiEvent::ScrollHead => {
                debug!("received scroll head event");
                let query = state.log_pane.jump_head(&mut state.log_pane_cursor_row);
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
            }
            TuiEvent::ScrollTail => {
                debug!("received scroll tail event");
                let query = state.log_pane.jump_tail(&mut state.log_pane_cursor_row);
                Self::dispatch_search(query, events_bus);
                Self::dispatch_selected_entry(state, events_bus);
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use ratatui::style::{Color, Modifier, Style};

    use super::*;
    use crate::{
        config::tui::{LogMatchStyle, ThemeConfig},
        event::{Match, TuiEvent},
        log::{LogLevel, Source},
        state::{events_bus::EventBus, tui_state::TuiState},
    };

    fn entry(level: LogLevel) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq: 42,
            msg: "error".to_string(),
            ts: Utc::now(),
            level: Some(level),
            source: Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "payments".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    fn sequenced_entry(seq: u64) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq,
            msg: format!("entry {seq}"),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "src-a".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    #[test]
    fn styled_field_splits_non_adjacent_match_indices() {
        let base = Style::default().fg(Color::Red);
        let matched = Style::default().fg(Color::Yellow);
        let matches = [Match {
            key: "msg".to_string(),
            indices: vec![0, 2],
        }];

        let spans = highlight::styled_field("error", Some(&matches), "msg", base, matched);

        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "e");
        assert_eq!(spans[0].style.fg, Some(Color::Yellow));
        assert_eq!(spans[1].content, "r");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[2].content, "r");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
        assert_eq!(spans[3].content, "or");
        assert_eq!(spans[3].style.fg, Some(Color::Red));
    }

    #[test]
    fn render_line_applies_level_color_and_match_override() {
        let theme = ThemeConfig {
            log_match_style: LogMatchStyle::Underline,
            ..ThemeConfig::default()
        };
        let matches = [Match {
            key: "msg".to_string(),
            indices: vec![0],
        }];

        let line = LogPane::render_line(
            &entry(LogLevel::Error),
            "1".to_string(),
            Some(&matches),
            &theme,
        );
        let highlighted = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "e")
            .expect("expected highlighted message span");
        let unhighlighted = line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "rror")
            .expect("expected unhighlighted message span");

        assert_eq!(highlighted.style.fg, None);
        assert!(
            highlighted
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert_eq!(unhighlighted.style.fg, Some(Color::Red));
        assert!(
            !unhighlighted
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn scroll_emits_selected_entry_event() {
        let mut state = TuiState::new(
            &crate::config::tui::TuiConfig::default(),
            &crate::config::search::SearchConfig::default(),
        )
        .expect("tui state");
        let mut events_bus = EventBus::new();
        state.log_pane.set_height(3, &mut state.log_pane_cursor_row);
        state.log_pane.apply_update(
            crate::state::tui_state::log_pane_state::LogPaneUpdate::Tail {
                entries: (1..=5).map(sequenced_entry).collect(),
                retained_bounds: (1, 5),
            },
            &mut state.log_pane_cursor_row,
        );

        LogPane::new().handle_event(
            TuiEvent::Scroll(ScrollDirection::Backward),
            &mut state,
            &mut events_bus,
        );

        match events_bus
            .tui_event_rx
            .try_recv()
            .expect("selected entry event")
        {
            TuiEvent::NewSelectedEntry(Some(selected_entry)) => {
                assert_eq!(selected_entry.entry.seq, 4);
            }
            event => panic!("expected selected entry event, got {event:?}"),
        }
    }
}
