use std::{io::stdout, time::Duration};

pub mod keybinds;
pub mod layout;
pub mod widgets;

use crossterm::{
    ExecutableCommand as _,
    event::{
        DisableMouseCapture, Event as CrosstermEvent, EventStream, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use futures_util::{FutureExt as _, StreamExt as _};
use ratatui::{Terminal, backend::Backend};
use tokio::{sync::mpsc, time::interval};
use tracing::{debug, error, trace, warn};

use crate::{
    config::tui::TuiConfig,
    error::FmlError,
    event::{Query, QuitEvent, SearchEvent, SearchTarget, TuiEvent},
    state::AppState,
    tui::{keybinds::StaticKeyAction, layout::Slot},
};

/// Start the TUI
pub fn spawn(config: &TuiConfig, event_tx: mpsc::UnboundedSender<TuiEvent>) {
    // Setup panic hooks
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = stdout().execute(DisableMouseCapture);
        let _ = stdout().execute(LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Create the tokio async task with an infinite loop
    tui_loop(config, event_tx);
}

/// Stop the TUI
pub fn kill() -> Result<(), FmlError> {
    disable_raw_mode()?;
    stdout().execute(DisableMouseCapture)?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Start a tokio async task which handles crossterm events
/// and render timing
fn tui_loop(config: &TuiConfig, event_tx: mpsc::UnboundedSender<TuiEvent>) {
    let frame_rate = config.frame_rate;

    tokio::spawn(async move {
        debug!(frame_rate, "tui event reader task started");
        let mut reader = EventStream::new();
        let mut render_interval = interval(Duration::from_secs_f64(1.0 / frame_rate));

        loop {
            let event = tokio::select! {
                _ = render_interval.tick() => TuiEvent::Render,
                crossterm_event = reader.next().fuse() => match crossterm_event {
                    Some(Ok(event)) => match event {
                        CrosstermEvent::Key(key) => {
                            // If we don't have a key press ignore
                            // the event. We don't want to action on
                            // key up, etc.
                            if key.kind != KeyEventKind::Press {
                                return
                            }

                            TuiEvent::Input(key)
                        },
                        CrosstermEvent::Mouse(mouse) => TuiEvent::Mouse(mouse),
                        CrosstermEvent::Resize(x, y) => TuiEvent::Resize(x, y),
                        CrosstermEvent::FocusLost => TuiEvent::FocusLost,
                        CrosstermEvent::FocusGained => TuiEvent::FocusGained,
                        CrosstermEvent::Paste(s) => TuiEvent::Paste(s),
                    }
                    Some(Err(err)) => {
                        warn!(error = %err, "tui event reader received crossterm error");
                        TuiEvent::Error(err.to_string())
                    }
                    None => {
                        debug!("tui event reader stream ended");
                        break;
                    }
                },
            };

            if let Err(err) = event_tx.send(event) {
                error!("error sending event from tui: {:?}", err);
                break;
            }
        }

        debug!("tui event reader task exited");
    });
}

/// Process a TUI event
///
/// Render events are intentionally a no-op here — rendering is an output
/// side-effect that the app loop performs directly via [`render`] so it can
/// thread the active terminal through. This keeps `AppState` backend-agnostic
/// and lets tests drive the same handlers against a `TestBackend`.
pub fn handle_tui_event(event: TuiEvent, state: AppState) -> AppState {
    let mut new_state = match event {
        TuiEvent::NewSelectedEntry(selected_entry) => {
            let mut state = state;
            state.tui.selected_entry = selected_entry.clone();
            state.tui.info_pane_scroll_offset = 0;
            if let Some(selected_entry) = selected_entry {
                let anchor_seq = selected_entry.entry.seq;
                state.tui.preview_pane.start_surrounding(anchor_seq);
                if let Err(err) = state
                    .event_bus
                    .search_event_tx
                    .try_send(SearchEvent::Search {
                        target: SearchTarget::PreviewPane,
                        query: Query::Surrounding {
                            middle_seq_id: anchor_seq,
                            buffer: state.config.search.tail_size as u64,
                        },
                        sources: vec![selected_entry.entry.source.id.clone()],
                    })
                {
                    error!("failed to dispatch preview surrounding search: {err}");
                }
            } else {
                state.tui.preview_pane.clear();
                if let Err(err) = state
                    .event_bus
                    .search_event_tx
                    .try_send(SearchEvent::Cancel {
                        target: SearchTarget::PreviewPane,
                    })
                {
                    error!("failed to cancel preview search: {err}");
                }
            }
            return state;
        }
        TuiEvent::Render => {
            trace!("received render event");

            state
        }
        TuiEvent::Error(ref err) => {
            error!("received error event - {}", err);

            state
        }
        TuiEvent::Input(key) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        let mut state = state;
                        state.tui.info_pane_scroll_offset =
                            state.tui.info_pane_scroll_offset.saturating_sub(1);
                        return state;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let mut state = state;
                        state.tui.info_pane_scroll_offset =
                            state.tui.info_pane_scroll_offset.saturating_add(1);
                        return state;
                    }
                    _ => {}
                }
            }

            let (static_key, _) = keybinds::match_key(&key, &state.tui.focused);

            match static_key {
                StaticKeyAction::Quit => {
                    if let Err(err) = state.event_bus.quit_tx.try_send(QuitEvent {}) {
                        // We have failed to send out quit here, which to be frank
                        // is pretty bad. So, what can we do? PANIC PANIC PANIC.
                        panic!("failed to quit - {}", err);
                    }
                }
                StaticKeyAction::FocusCycle => {
                    let focusable = Slot::focusable();

                    // Find the index of our current focused slot by iteration
                    if let Some(index) = focusable.iter().position(|s| *s == state.tui.focused) {
                        // Create a new state with the focus incremented by 1
                        let mut focused_state = state;
                        focused_state.tui.focused = focusable[(index + 1) % focusable.len()];

                        // Return our new state
                        return focused_state;
                    }
                }
                _ => {}
            }

            state
        }
        _ => state,
    };

    for widget in new_state.widgets.iter_mut() {
        if new_state.tui.focused == widget.slot() {
            // Propagate the event to the focused
            // widget, not any further
            widget.handle_event(event, &mut new_state.tui, &mut new_state.event_bus);
            break;
        }
    }

    new_state
}

// Render the current state into `terminal`. Generic over the backend so
// production runs against a `CrosstermBackend` and tests render into a
// `TestBackend` without further plumbing.
pub fn render<B: Backend>(state: &mut AppState, terminal: &mut Terminal<B>) {
    let result = terminal.draw(|frame| {
        let areas = layout::build_layout(frame.area(), state.config.tui.sidebar_width_percent);

        for widget in state.widgets.iter_mut() {
            if let Some(&area) = areas.get(&widget.slot()) {
                widget.render(frame, area, &mut state.tui);
            }
        }
        // Reassign the areas to our state to cache them for next render
        state.tui.areas = areas;
    });

    if let Err(err) = result
        && let Err(err) = state
            .event_bus
            .tui_event_tx
            .send(TuiEvent::Error(err.to_string()))
    {
        // If we failed to send even the error that we errored, we
        // can't really do anything but either panic or log and try
        // again in the render event.
        error!(
            "failed to send tui_event error after failed render - {}",
            err
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::widgets::ScrollDirection;

    use super::*;
    use crate::{
        config::Config,
        event::{SelectedEntry, TuiEvent},
        log::{LogEntry, LogLevel, Source},
        state::tui_state::preview_pane_state::PreviewStatus,
    };

    fn entry(seq: u64) -> Arc<LogEntry> {
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
    fn new_selected_entry_event_updates_tui_state() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.info_pane_scroll_offset = 4;
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry(7),
                matches: Vec::new(),
            })),
            state,
        );

        assert_eq!(
            state
                .tui
                .selected_entry
                .as_ref()
                .map(|selected| selected.entry.seq),
            Some(7)
        );
        assert_eq!(state.tui.info_pane_scroll_offset, 0);
        assert_eq!(state.tui.preview_pane.status, PreviewStatus::Loading);
        assert_eq!(state.tui.preview_pane.anchor_seq, Some(7));
        match state.event_bus.search_event_rx.try_recv() {
            Ok(SearchEvent::Search {
                target,
                query:
                    Query::Surrounding {
                        middle_seq_id,
                        buffer,
                    },
                sources,
            }) => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(middle_seq_id, 7);
                assert_eq!(buffer, state.config.search.tail_size as u64);
                assert_eq!(sources, vec!["src-a".to_string()]);
            }
            event => panic!("expected preview surrounding search, got {event:?}"),
        }
    }

    #[test]
    fn new_selected_entry_event_clears_tui_state() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(7),
            matches: Vec::new(),
        });
        state.tui.info_pane_scroll_offset = 4;

        let mut state = handle_tui_event(TuiEvent::NewSelectedEntry(None), state);

        assert!(state.tui.selected_entry.is_none());
        assert_eq!(state.tui.info_pane_scroll_offset, 0);
        assert_eq!(state.tui.preview_pane.status, PreviewStatus::NoSelection);
        assert_eq!(state.tui.preview_pane.anchor_seq, None);
        assert!(matches!(
            state.event_bus.search_event_rx.try_recv(),
            Ok(SearchEvent::Cancel {
                target: SearchTarget::PreviewPane
            })
        ));
    }

    #[test]
    fn ctrl_down_scrolls_info_pane_globally() {
        let state = AppState::new(Config::default()).expect("app state");
        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
            state,
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 1);
    }

    #[test]
    fn ctrl_j_scrolls_info_pane_globally() {
        let state = AppState::new(Config::default()).expect("app state");
        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            state,
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 1);
    }

    #[test]
    fn ctrl_up_scrolls_info_pane_and_saturates_at_zero() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.info_pane_scroll_offset = 2;

        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            state,
        );
        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            state,
        );
        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
            state,
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 0);
    }

    #[test]
    fn ctrl_k_scrolls_info_pane_and_saturates_at_zero() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.info_pane_scroll_offset = 1;

        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            state,
        );
        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            state,
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 0);
    }

    #[test]
    fn regular_down_still_dispatches_to_focused_widget() {
        let mut state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            AppState::new(Config::default()).expect("app state"),
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 0);
        assert!(matches!(
            state.event_bus.tui_event_rx.try_recv(),
            Ok(TuiEvent::Scroll(ScrollDirection::Forward))
        ));
    }
}
