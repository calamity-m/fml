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
    log::SourceId,
    state::AppState,
    tui::{
        keybinds::{CustomizedKeyAction, StaticKeyAction},
        layout::Slot,
    },
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
        TuiEvent::DispatchLogPaneSearch(ref query) => {
            let mut state = state;
            dispatch_log_pane_search(query.clone(), &mut state);
            state
        }
        TuiEvent::RedispatchLogPaneSearch => {
            let mut state = state;
            if let Some(query) = state.tui.log_pane.active_query.clone() {
                dispatch_log_pane_search(query, &mut state);
            }
            state
        }
        TuiEvent::Error(ref err) => {
            error!("received error event - {}", err);

            state
        }
        TuiEvent::Input(key) => {
            let mut state = state;

            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.tui.info_pane_scroll_offset =
                            state.tui.info_pane_scroll_offset.saturating_sub(1);
                        return state;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        state.tui.info_pane_scroll_offset =
                            state.tui.info_pane_scroll_offset.saturating_add(1);
                        return state;
                    }
                    _ => {}
                }
            }

            let (static_key, custom_key) = keybinds::match_key(&key, &state.tui.focused);

            if static_key == StaticKeyAction::Quit {
                if let Err(err) = state.event_bus.quit_tx.try_send(QuitEvent {}) {
                    // We have failed to send out quit here, which to be frank
                    // is pretty bad. So, what can we do? PANIC PANIC PANIC.
                    panic!("failed to quit - {}", err);
                }
                return state;
            }

            if handle_source_selector_input(&key, custom_key, &mut state) {
                return state;
            }

            if static_key == StaticKeyAction::FocusCycle {
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

fn handle_source_selector_input(
    key: &crossterm::event::KeyEvent,
    custom_key: CustomizedKeyAction,
    state: &mut AppState,
) -> bool {
    if custom_key == CustomizedKeyAction::ToggleSourceSelector {
        state.tui.toggle_source_selector(&state.producer.sources);
        return true;
    }

    if !state.tui.source_selector.open {
        return false;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Enter => state.tui.close_source_selector(),
        KeyCode::Up | KeyCode::Char('k') => {
            let row_count = widgets::source_selector::source_selector_row_count(&state.tui);
            state.tui.source_selector_cursor_up(row_count);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let row_count = widgets::source_selector::source_selector_row_count(&state.tui);
            state.tui.source_selector_cursor_down(row_count);
        }
        KeyCode::Char(' ') => {
            widgets::source_selector::toggle_selected_row(&mut state.tui);
            handle_source_selection_changed(state);
        }
        KeyCode::Char('a') => {
            widgets::source_selector::enable_all_open_sources(&mut state.tui);
            handle_source_selection_changed(state);
        }
        KeyCode::Char('n') => {
            widgets::source_selector::disable_all_open_sources(&mut state.tui);
            handle_source_selection_changed(state);
        }
        _ => {}
    }

    true
}

fn handle_source_selection_changed(state: &mut AppState) {
    if state.tui.source_selector.enabled_source_ids.is_empty() && !state.producer.sources.is_empty()
    {
        apply_no_sources_selected(state);
        return;
    }

    schedule_source_filter_redispatch(state);
}

fn schedule_source_filter_redispatch(state: &mut AppState) {
    if let Some(handle) = state.tui.source_filter_debounce_handle.take() {
        handle.abort();
    }

    let tx = state.event_bus.tui_event_tx.clone();
    let debounce_ms = state.tui.fuzzy_debounce_ms;
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        state.tui.source_filter_debounce_handle = Some(handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
            if let Err(err) = tx.send(TuiEvent::RedispatchLogPaneSearch) {
                debug!("failed to schedule source-filter search redispatch - {err}");
            }
        }));
    } else if let Err(err) = tx.send(TuiEvent::RedispatchLogPaneSearch) {
        debug!("failed to schedule source-filter search redispatch - {err}");
    }
}

fn dispatch_log_pane_search(query: Query, state: &mut AppState) {
    let Some(sources) = filtered_log_pane_sources(state) else {
        apply_no_sources_selected(state);
        return;
    };

    if let Err(err) = state
        .event_bus
        .search_event_tx
        .try_send(SearchEvent::Search {
            target: SearchTarget::LogPane,
            query,
            sources,
        })
    {
        error!("failed to dispatch filtered log pane search - {err}");
    }
}

fn filtered_log_pane_sources(state: &AppState) -> Option<Vec<SourceId>> {
    let live_sources = &state.producer.sources;
    if live_sources.is_empty() {
        return Some(Vec::new());
    }

    let enabled = &state.tui.source_selector.enabled_source_ids;
    if enabled.is_empty() {
        return None;
    }

    if live_sources
        .iter()
        .all(|source| enabled.contains(&source.id))
    {
        return Some(Vec::new());
    }

    let source_ids: Vec<SourceId> =
        widgets::source_selector::source_ids_in_tree_order(live_sources)
            .into_iter()
            .filter(|source_id| enabled.contains(source_id))
            .collect();

    (!source_ids.is_empty()).then_some(source_ids)
}

fn apply_no_sources_selected(state: &mut AppState) {
    if let Some(handle) = state.tui.source_filter_debounce_handle.take() {
        handle.abort();
    }

    if let Err(err) = state
        .event_bus
        .search_event_tx
        .try_send(SearchEvent::Cancel {
            target: SearchTarget::LogPane,
        })
    {
        error!("failed to cancel log pane search after disabling all sources - {err}");
    }

    state
        .tui
        .log_pane
        .show_no_sources_selected(&mut state.tui.log_pane_cursor_row);
    if let Err(err) = state
        .event_bus
        .tui_event_tx
        .send(TuiEvent::NewSelectedEntry(None))
    {
        error!("failed to clear selected entry after disabling all sources - {err}");
    }
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
        widgets::source_selector::render_source_selector(frame, frame.area(), &mut state.tui);
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

    fn source(id: &str) -> Source {
        source_with_group(id, None)
    }

    fn source_with_group(id: &str, group: Option<&str>) -> Source {
        Source {
            producer: "fake".to_string(),
            id: id.to_string(),
            display_name: format!("Source {id}"),
            group: group.map(str::to_string),
        }
    }

    fn input(code: KeyCode) -> TuiEvent {
        TuiEvent::Input(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_input(code: KeyCode) -> TuiEvent {
        TuiEvent::Input(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    fn recv_search_event(state: &mut AppState) -> SearchEvent {
        state
            .event_bus
            .search_event_rx
            .try_recv()
            .expect("search event")
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

    #[test]
    fn ctrl_s_opens_and_closes_source_selector_without_changing_focus() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        state.producer.sources = vec![source("src-a"), source("src-b")];

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        assert!(state.tui.source_selector.open);
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.source_selector.cursor, 0);
        assert_eq!(state.tui.source_selector.open_sources.len(), 2);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        assert!(!state.tui.source_selector.open);
        assert_eq!(state.tui.focused, Slot::QueryBox);
    }

    #[test]
    fn source_selector_navigation_updates_cursor_and_swallows_scroll() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        state = handle_tui_event(input(KeyCode::Down), state);

        assert_eq!(state.tui.source_selector.cursor, 1);
        assert!(state.event_bus.tui_event_rx.try_recv().is_err());

        state = handle_tui_event(input(KeyCode::Up), state);

        assert_eq!(state.tui.source_selector.cursor, 0);
        assert!(state.event_bus.tui_event_rx.try_recv().is_err());
    }

    #[test]
    fn source_selector_navigation_scrolls_when_visible_window_is_full() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        state.tui.set_source_selector_visible_row_count(4, 2);

        state = handle_tui_event(input(KeyCode::Down), state);
        state = handle_tui_event(input(KeyCode::Down), state);
        state = handle_tui_event(input(KeyCode::Down), state);

        assert_eq!(state.tui.source_selector.cursor, 1);
        assert_eq!(state.tui.source_selector.scroll_offset, 2);

        state = handle_tui_event(input(KeyCode::Up), state);
        state = handle_tui_event(input(KeyCode::Up), state);

        assert_eq!(state.tui.source_selector.cursor, 0);
        assert_eq!(state.tui.source_selector.scroll_offset, 1);
    }

    #[test]
    fn source_selector_swallows_non_local_keys_and_preserves_focus() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        let state = handle_tui_event(input(KeyCode::Tab), state);
        let state = handle_tui_event(input(KeyCode::Char('x')), state);

        assert!(state.tui.source_selector.open);
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.query_box_textarea.lines().join("\n").trim(), "");
    }

    #[test]
    fn source_selector_escape_and_enter_close_popup() {
        let state = AppState::new(Config::default()).expect("app state");
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        let state = handle_tui_event(input(KeyCode::Esc), state);

        assert!(!state.tui.source_selector.open);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        let state = handle_tui_event(input(KeyCode::Enter), state);

        assert!(!state.tui.source_selector.open);
    }

    #[test]
    fn ctrl_c_still_quits_when_source_selector_is_open() {
        let state = AppState::new(Config::default()).expect("app state");
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        state = handle_tui_event(ctrl_input(KeyCode::Char('c')), state);

        assert!(state.tui.source_selector.open);
        assert!(state.event_bus.quit_rx.try_recv().is_ok());
    }

    #[test]
    fn dispatch_log_pane_search_uses_wildcard_before_live_sources_exist() {
        let state = AppState::new(Config::default()).expect("app state");

        let mut state = handle_tui_event(TuiEvent::DispatchLogPaneSearch(Query::Tail), state);

        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query: Query::Tail,
                sources,
            } => {
                assert_eq!(target, SearchTarget::LogPane);
                assert!(sources.is_empty());
            }
            event => panic!("expected wildcard tail search, got {event:?}"),
        }
    }

    #[test]
    fn dispatch_log_pane_search_uses_wildcard_when_enabled_covers_live_sources() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        state.tui.enable_source_id("src-a".to_string());
        state.tui.enable_source_id("src-b".to_string());
        state.tui.enable_source_id("src-c".to_string());

        let mut state = handle_tui_event(TuiEvent::DispatchLogPaneSearch(Query::Tail), state);

        match recv_search_event(&mut state) {
            SearchEvent::Search { sources, .. } => assert!(sources.is_empty()),
            event => panic!("expected wildcard search, got {event:?}"),
        }
    }

    #[test]
    fn dispatch_log_pane_tail_search_uses_filtered_source_ids() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![
            source_with_group("src-b", Some("backend")),
            source_with_group("src-a", Some("backend")),
            source_with_group("src-c", Some("frontend")),
        ];
        state.tui.enable_source_id("src-b".to_string());
        state.tui.enable_source_id("src-c".to_string());

        let mut state = handle_tui_event(TuiEvent::DispatchLogPaneSearch(Query::Tail), state);

        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query: Query::Tail,
                sources,
            } => {
                assert_eq!(target, SearchTarget::LogPane);
                assert_eq!(sources, vec!["src-b".to_string(), "src-c".to_string()]);
            }
            event => panic!("expected filtered tail search, got {event:?}"),
        }
    }

    #[test]
    fn dispatch_log_pane_history_search_uses_filtered_source_ids() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        state.tui.enable_source_id("src-b".to_string());

        let mut state = handle_tui_event(
            TuiEvent::DispatchLogPaneSearch(Query::History {
                middle_seq_id: 42,
                buffer: 10,
            }),
            state,
        );

        match recv_search_event(&mut state) {
            SearchEvent::Search {
                query:
                    Query::History {
                        middle_seq_id,
                        buffer,
                    },
                sources,
                ..
            } => {
                assert_eq!(middle_seq_id, 42);
                assert_eq!(buffer, 10);
                assert_eq!(sources, vec!["src-b".to_string()]);
            }
            event => panic!("expected filtered history search, got {event:?}"),
        }
    }

    #[test]
    fn redispatch_active_fuzzy_query_uses_filtered_source_ids() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        state.tui.enable_source_id("src-b".to_string());
        state.tui.log_pane.active_query = Some(Query::Fuzzy("error".to_string()));

        let mut state = handle_tui_event(TuiEvent::RedispatchLogPaneSearch, state);

        match recv_search_event(&mut state) {
            SearchEvent::Search {
                query: Query::Fuzzy(term),
                sources,
                ..
            } => {
                assert_eq!(term, "error");
                assert_eq!(sources, vec!["src-b".to_string()]);
            }
            event => panic!("expected filtered fuzzy search, got {event:?}"),
        }
    }

    #[test]
    fn dispatch_log_pane_search_with_no_enabled_sources_shows_empty_state() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];

        let mut state = handle_tui_event(TuiEvent::DispatchLogPaneSearch(Query::Tail), state);

        assert_eq!(
            state.tui.log_pane.empty_message(),
            Some("No sources selected")
        );
        assert!(matches!(
            recv_search_event(&mut state),
            SearchEvent::Cancel {
                target: SearchTarget::LogPane
            }
        ));
        assert!(matches!(
            state.event_bus.tui_event_rx.try_recv(),
            Ok(TuiEvent::NewSelectedEntry(None))
        ));
    }

    #[test]
    fn new_source_arrival_while_filtered_stays_explicit_not_wildcard() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b"), source("src-c")];
        state.tui.enable_source_id("src-b".to_string());
        state.tui.enable_source_id("src-c".to_string());

        let mut state = handle_tui_event(TuiEvent::DispatchLogPaneSearch(Query::Tail), state);

        match recv_search_event(&mut state) {
            SearchEvent::Search { sources, .. } => {
                assert_eq!(sources, vec!["src-b".to_string(), "src-c".to_string()]);
            }
            event => panic!("expected explicit source list, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn source_filter_redispatch_is_debounced() {
        let mut config = Config::default();
        config.search.fuzzy_debounce_ms = 10;
        let mut state = AppState::new(config).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        state.tui.enable_source_id("src-a".to_string());
        state.tui.enable_source_id("src-b".to_string());
        state.tui.log_pane.active_query = Some(Query::Tail);

        state.tui.source_selector.enabled_source_ids.remove("src-a");
        handle_source_selection_changed(&mut state);
        state.tui.enable_source_id("src-a".to_string());
        handle_source_selection_changed(&mut state);

        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(matches!(
            state.event_bus.tui_event_rx.try_recv(),
            Ok(TuiEvent::RedispatchLogPaneSearch)
        ));
        assert!(state.event_bus.tui_event_rx.try_recv().is_err());
    }

    #[test]
    fn source_selector_space_toggles_source_row() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![
            source_with_group("src-a", Some("backend")),
            source_with_group("src-b", Some("backend")),
        ];
        state.tui.enable_source_id("src-a".to_string());
        state.tui.enable_source_id("src-b".to_string());
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        state.tui.source_selector.cursor = 2;

        state = handle_tui_event(input(KeyCode::Char(' ')), state);

        assert!(
            !state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-a")
        );
        assert!(
            state
                .tui
                .source_selector
                .enabled_source_ids
                .contains("src-b")
        );
        let rows = widgets::source_selector::source_selector_rows(&state.tui);
        assert_eq!(
            rows[0].checkbox,
            widgets::source_selector::CheckboxState::Mixed
        );
        assert_eq!(
            rows[1].checkbox,
            widgets::source_selector::CheckboxState::Mixed
        );
    }

    #[test]
    fn source_selector_space_toggles_group_row() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![
            source_with_group("src-a", Some("backend")),
            source_with_group("src-b", Some("backend")),
            source_with_group("src-c", Some("frontend")),
        ];
        state.tui.enable_source_id("src-b".to_string());
        state.tui.enable_source_id("src-c".to_string());
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        state.tui.source_selector.cursor = 1;

        state = handle_tui_event(input(KeyCode::Char(' ')), state);

        assert!(
            ["src-a", "src-b", "src-c"].iter().all(|id| state
                .tui
                .source_selector
                .enabled_source_ids
                .contains(*id))
        );
    }

    #[test]
    fn source_selector_space_toggles_producer_row_to_all_disabled() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![
            source_with_group("src-a", Some("backend")),
            source_with_group("src-b", Some("backend")),
        ];
        state.tui.enable_source_id("src-a".to_string());
        state.tui.enable_source_id("src-b".to_string());
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        let mut state = handle_tui_event(input(KeyCode::Char(' ')), state);

        assert!(state.tui.source_selector.enabled_source_ids.is_empty());
        assert_eq!(
            state.tui.log_pane.empty_message(),
            Some("No sources selected")
        );
        assert!(matches!(
            state.event_bus.search_event_rx.try_recv(),
            Ok(SearchEvent::Cancel {
                target: SearchTarget::LogPane
            })
        ));
        assert!(matches!(
            state.event_bus.tui_event_rx.try_recv(),
            Ok(TuiEvent::NewSelectedEntry(None))
        ));
    }

    #[test]
    fn source_selector_a_and_n_enable_or_disable_all_open_sources() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a"), source("src-b")];
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        let state = handle_tui_event(input(KeyCode::Char('a')), state);
        assert!(
            ["src-a", "src-b"].iter().all(|id| state
                .tui
                .source_selector
                .enabled_source_ids
                .contains(*id))
        );

        let mut state = handle_tui_event(input(KeyCode::Char('n')), state);
        assert!(state.tui.source_selector.enabled_source_ids.is_empty());
        assert_eq!(
            state.tui.log_pane.empty_message(),
            Some("No sources selected")
        );
        assert!(matches!(
            state.event_bus.search_event_rx.try_recv(),
            Ok(SearchEvent::Cancel {
                target: SearchTarget::LogPane
            })
        ));
    }
}
