use ratatui::{
    style::Style,
    widgets::{
        Block, List, ListItem, ListState, ScrollDirection, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use tracing::{debug, error, trace};

use crate::{
    config::tui::ThemeConfig,
    event::TuiEvent,
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

    pub fn border_style(&self, focused: &Slot, theme: &ThemeConfig) -> Style {
        // When this pane has focus, use the full surface style (brighter border).
        // When unfocused, dim the border so the focused pane stands out.
        if focused == &Slot::Main {
            return theme.surface_style();
        }

        theme.surface_style().fg(theme.border_unfocused_fg)
    }

    fn title(&self, state: &LogPaneState) -> String {
        let base = match state.mode {
            ScrollMode::Tail => "TAIL",
            ScrollMode::History => "HISTORY",
            ScrollMode::Search => "SEARCH",
        };

        format!(" FML [{base}] ")
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
        state.log_pane.height = inner_area.height as usize;

        // Build a window of items slightly larger than the visible area so that
        // fast scrolling doesn't hit a visible edge before the next frame renders.
        // Eventually this slices the real backing buffer; for now it's fake data.
        let window_size = state.log_pane.height + 20;
        let items: Vec<ListItem> = (0..window_size)
            .map(|i| {
                ListItem::new(format!(
                    "[INFO] log line {i:>3} — lorem ipsum dolor sit amet"
                ))
            })
            .collect();
        let item_count = items.len();

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
            ListState::default().with_selected(Some(state.log_pane.absolute_cursor));

        frame.render_stateful_widget(list, inner_area, &mut list_state);

        // Render the scrollbar over the right border of the block. We render on
        // `area` (the outer rect including the border), not `inner_area` — this
        // is what makes it sit on top of the border rather than inside the pane.
        //
        // begin/end symbols are None so there are no arrow caps, keeping the
        // corners of the border intact. The track symbol matches the border
        // character so inactive track segments are invisible — only the thumb
        // stands out.
        // ScrollbarState needs three things to render correctly:
        //   - content_length: total number of items in the full display list
        //   - viewport_content_length: how many rows are visible at once
        //   - position: where the cursor is in the full display list
        //
        // Without viewport_content_length, ratatui can't calculate the thumb
        // size — it doesn't know what fraction of the content is on screen.
        // content_length is a placeholder until the backing buffer exists.
        let mut scrollbar_state = ScrollbarState::new(item_count)
            .viewport_content_length(state.log_pane.height)
            .position(state.log_pane.absolute_cursor);

        // Inset by 1 row top and bottom so the track sits between the border
        // corners rather than overwriting them.
        let scrollbar_area = ratatui::prelude::Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };

        debug!(
            "absolute_cursor - {}, log_pane.height: {}",
            state.log_pane.absolute_cursor, state.log_pane.height
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

    fn handle_event(&self, event: TuiEvent, state: &mut TuiState, events_bus: &mut EventBus) {
        match event {
            TuiEvent::Scroll(scroll) => {
                debug!("received scroll event - {:?}", scroll);

                match scroll {
                    ScrollDirection::Forward => {
                        state.log_pane.absolute_cursor = state
                            .log_pane
                            .absolute_cursor
                            .saturating_add(1)
                            // TODO this needs to be items.len or something in the future
                            .min(state.log_pane.height - 1);

                        // TODO this actually needs to be real
                        // if state.tui.log_pane.absolute_cursor - window-start >= items.len
                        //   window-start += 1
                    }
                    ScrollDirection::Backward => {
                        state.log_pane.absolute_cursor =
                            state.log_pane.absolute_cursor.saturating_sub(1);
                    }
                }
            }
            TuiEvent::Input(key) => {
                debug!("received input event - {:?}", key);
                let (static_key, custom_key) = keybinds::match_key(&key, &state.focused);

                // First we process static keys as higher relevance

                match static_key {
                    StaticKeyAction::ScrollUp => {
                        if let Err(err) = events_bus
                            .tui_event_tx
                            .send(TuiEvent::Scroll(ScrollDirection::Backward))
                        {
                            error!("failed to send scroll up tui event - {}", err);
                        }
                    }
                    StaticKeyAction::ScrollDown => {
                        if let Err(err) = events_bus
                            .tui_event_tx
                            .send(TuiEvent::Scroll(ScrollDirection::Forward))
                        {
                            error!("failed to send scroll down tui event - {}", err);
                        }
                    }
                    _ => {}
                }

                match custom_key {
                    keybinds::CustomizedKeyAction::ToggleHelp => todo!(),
                    keybinds::CustomizedKeyAction::ShowInfo => todo!(),
                    keybinds::CustomizedKeyAction::None => {}
                }
            }
            _ => {}
        }
    }
}
