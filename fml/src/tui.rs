use std::{io::stdout, time::Duration};

pub mod keybinds;
pub mod layout;
pub mod widgets;

use crossterm::{
    ExecutableCommand as _,
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, EventStream, KeyCode,
        KeyEventKind, KeyModifiers,
    },
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use futures_util::{FutureExt as _, StreamExt as _};
use ratatui::{Terminal, backend::Backend};
use tokio::{sync::mpsc, time::interval};
use tracing::{debug, error, trace, warn};

use crate::{
    clipboard::OSC52_WARN_BYTES,
    config::tui::TuiConfig,
    error::FmlError,
    event::{Query, QuitEvent, SearchEvent, SearchTarget, TuiEvent},
    log::SourceId,
    state::{
        AppState,
        tui_state::{ActivePopup, preview_pane_state::PreviewModeCycle},
    },
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
            let prev_seq = state.tui.selected_entry.as_ref().map(|e| e.entry.seq);
            let next_seq = selected_entry.as_ref().map(|e| e.entry.seq);
            state.tui.selected_entry = selected_entry.clone();
            if prev_seq != next_seq {
                state.tui.info_pane_scroll_offset = 0;
            }
            if state.tui.field_picker_is_open() {
                state.tui.prune_field_picker_selection_to_selected_entry();
            }
            state.tui.preview_pane.selected_entry_changed(
                selected_entry.as_ref(),
                state.config.search.tail_size as u64,
                &state.event_bus.search_event_tx,
            );
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
            let (static_key, custom_key) = keybinds::match_key(&key, &state.tui.focused);

            if static_key == StaticKeyAction::Quit {
                if let Err(err) = state.event_bus.quit_tx.try_send(QuitEvent {}) {
                    // We have failed to send out quit here, which to be frank
                    // is pretty bad. So, what can we do? PANIC PANIC PANIC.
                    panic!("failed to quit - {}", err);
                }
                return state;
            }

            if custom_key == CustomizedKeyAction::TogglePreviewMode {
                handle_preview_mode_toggle(&mut state);
                return state;
            }

            if custom_key == CustomizedKeyAction::ToggleSelectMode {
                state.tui.select_mode = !state.tui.select_mode;
                // No widget currently consumes TuiEvent::Mouse, so releasing
                // capture has no in-app functional regression today. The toggle
                // is the first way users can get terminal wheel scrollback.
                let mut out = stdout();
                if state.tui.select_mode {
                    let _ = out.execute(DisableMouseCapture);
                } else {
                    let _ = out.execute(EnableMouseCapture);
                }
                return state;
            }

            if custom_key == CustomizedKeyAction::YankSelectedEntry
                && state.tui.focused == Slot::Main
                && state.tui.active_popup().is_none()
            {
                handle_yank_selected_entry(&mut state);
                return state;
            }

            if key.code == KeyCode::Esc && state.tui.field_picker_is_open() {
                state.tui.preview_pane.cancel_field_selection();
                state.tui.close_field_picker();
                return state;
            }

            if key.code == KeyCode::Esc && state.tui.active_popup().is_some() {
                state.tui.close_popup();
                return state;
            }

            if handle_popup_input(&key, custom_key, &mut state) {
                return state;
            }

            if custom_key == CustomizedKeyAction::ToggleHelp {
                state.tui.toggle_help();
                return state;
            }

            if custom_key == CustomizedKeyAction::ToggleSourceSelector {
                state.tui.toggle_source_selector(&state.producer.sources);
                return state;
            }

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

            // `/` focuses the query box without inserting into query text.
            if key.code == KeyCode::Char('/') && state.tui.focused == Slot::Main {
                state.tui.focused = Slot::QueryBox;
                return state;
            }

            // `Enter` while the query box is focused returns to log navigation
            // without passing the key to the textarea.
            if key.code == KeyCode::Enter && state.tui.focused == Slot::QueryBox {
                state.tui.focused = Slot::Main;
                return state;
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

fn handle_popup_input(
    key: &crossterm::event::KeyEvent,
    custom_key: CustomizedKeyAction,
    state: &mut AppState,
) -> bool {
    match state.tui.active_popup() {
        Some(ActivePopup::FieldPicker) => handle_field_picker_input(key, custom_key, state),
        Some(ActivePopup::Help) => handle_help_popup_input(custom_key, state),
        Some(ActivePopup::SourceSelector) => handle_source_selector_input(key, custom_key, state),
        None => false,
    }
}

fn handle_field_picker_input(
    key: &crossterm::event::KeyEvent,
    custom_key: CustomizedKeyAction,
    state: &mut AppState,
) -> bool {
    if custom_key == CustomizedKeyAction::ToggleHelp {
        state.tui.preview_pane.cancel_field_selection();
        state.tui.toggle_help();
        return true;
    }
    if custom_key == CustomizedKeyAction::ToggleSourceSelector {
        state.tui.preview_pane.cancel_field_selection();
        state.tui.open_source_selector(&state.producer.sources);
        return true;
    }

    match key.code {
        KeyCode::Enter => {
            let selected_keys = state.tui.selected_field_picker_keys();
            if selected_keys.is_empty() {
                return true;
            }
            if let Some(selected_entry) = state.tui.selected_entry.clone() {
                state.tui.preview_pane.apply_field_selection(
                    &selected_entry,
                    &selected_keys,
                    state.config.search.tail_size as u64,
                    &state.event_bus.search_event_tx,
                );
                state.tui.close_field_picker();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let row_count = widgets::field_picker::field_picker_row_count(&state.tui);
            state.tui.field_picker_cursor_up(row_count);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let row_count = widgets::field_picker::field_picker_row_count(&state.tui);
            state.tui.field_picker_cursor_down(row_count);
        }
        KeyCode::Char(' ') => {
            widgets::field_picker::toggle_selected_row(&mut state.tui);
        }
        _ => {}
    }

    true
}

fn handle_help_popup_input(custom_key: CustomizedKeyAction, state: &mut AppState) -> bool {
    match custom_key {
        CustomizedKeyAction::ToggleHelp => state.tui.close_popup(),
        CustomizedKeyAction::ToggleSourceSelector => {
            state.tui.open_source_selector(&state.producer.sources);
        }
        _ => {}
    }
    true
}

fn handle_source_selector_input(
    key: &crossterm::event::KeyEvent,
    custom_key: CustomizedKeyAction,
    state: &mut AppState,
) -> bool {
    if custom_key == CustomizedKeyAction::ToggleSourceSelector {
        state.tui.close_source_selector();
        return true;
    }
    if custom_key == CustomizedKeyAction::ToggleHelp {
        state.tui.toggle_help();
        return true;
    }

    match key.code {
        KeyCode::Enter => state.tui.close_source_selector(),
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

fn handle_preview_mode_toggle(state: &mut AppState) {
    if state.tui.field_picker_is_open() {
        state.tui.preview_pane.skip_field_selection_cycle();
        state.tui.close_field_picker();
        if let Some(selected_entry) = state.tui.selected_entry.as_ref() {
            state.tui.preview_pane.selected_entry_changed(
                Some(selected_entry),
                state.config.search.tail_size as u64,
                &state.event_bus.search_event_tx,
            );
        }
        return;
    } else if state.tui.active_popup().is_some() {
        state.tui.close_popup();
    }

    let result = state.tui.preview_pane.cycle_mode(
        state.tui.selected_entry.as_ref(),
        state.config.search.tail_size as u64,
        &state.event_bus.search_event_tx,
    );
    if result == PreviewModeCycle::NeedsFieldSelection {
        state.tui.open_field_picker();
    }
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

fn handle_yank_selected_entry(state: &mut AppState) {
    let Some(selected) = state.tui.selected_entry.as_ref() else {
        return; // silent no-op when nothing is selected
    };
    let json = serde_json::to_string(&*selected.entry).unwrap_or_default();
    let mut out = stdout().lock();
    let msg = match crate::clipboard::yank_osc52(&mut out, &json) {
        Ok(n) if n > OSC52_WARN_BYTES => {
            format!("sent yank ({n} bytes) — exceeds 8KB; xterm/vte may drop it")
        }
        Ok(n) => format!("sent yank ({n} bytes) — check clipboard"),
        Err(e) => format!("yank failed: {e}"),
    };
    drop(out);
    state.tui.set_status_message(msg);

    if let Some(hint) = multiplexer_hint() {
        if !state.tui.multiplexer_clipboard_hint_shown {
            state.tui.multiplexer_clipboard_hint_shown = true;
            state.tui.queue_status_message(hint);
        }
    }
}

/// Returns a one-time hint string when running inside a known multiplexer,
/// or `None` when no multiplexer is detected.
fn multiplexer_hint() -> Option<String> {
    let in_tmux = std::env::var("TMUX").is_ok();
    let in_zellij = std::env::var("ZELLIJ").is_ok();
    match (in_tmux, in_zellij) {
        (true, true) | (true, false) => {
            Some("tmux: run `set -g set-clipboard on` to enable OSC52 yank".into())
        }
        (false, true) => {
            Some("zellij: OSC52 yank needs an OSC52-capable host terminal or copy_command".into())
        }
        (false, false) => None,
    }
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
        for widget in state.popup_widgets.iter() {
            widget.render(frame, frame.area(), &mut state.tui);
        }
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
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::widgets::ScrollDirection;
    use serde_json::json;

    use super::*;
    use crate::{
        config::Config,
        event::{FieldPredicate, SelectedEntry, TuiEvent},
        log::{LogEntry, LogLevel, Source},
        state::tui_state::preview_pane_state::{PreviewMode, PreviewStatus},
    };

    fn entry(seq: u64) -> Arc<LogEntry> {
        entry_with_fields(seq, HashMap::new())
    }

    fn entry_with_fields(seq: u64, fields: HashMap<String, serde_json::Value>) -> Arc<LogEntry> {
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
            fields,
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
        assert_eq!(state.tui.preview_pane.status, PreviewStatus::NoSelection);
        assert_eq!(state.tui.preview_pane.anchor_seq, None);
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query:
                    Query::Surrounding {
                        middle_seq_id,
                        buffer,
                    },
                sources,
            } => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(middle_seq_id, 7);
                assert_eq!(buffer, state.config.search.tail_size as u64);
                assert_eq!(sources, vec!["src-a".to_string()]);
            }
            event => panic!("expected preview surrounding search, got {event:?}"),
        }
    }

    #[test]
    fn new_selected_entry_preserves_ready_preview_until_result_applies() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.preview_pane.start_surrounding(3);
        state
            .tui
            .preview_pane
            .apply_surrounding(3, vec![entry(2), entry(3), entry(4)]);

        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry(7),
                matches: Vec::new(),
            })),
            state,
        );

        assert_eq!(state.tui.preview_pane.status, PreviewStatus::Ready);
        assert_eq!(state.tui.preview_pane.anchor_seq, Some(3));
        assert_eq!(
            state
                .tui
                .preview_pane
                .items()
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query:
                    Query::Surrounding {
                        middle_seq_id,
                        buffer,
                    },
                sources,
            } => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(middle_seq_id, 7);
                assert_eq!(buffer, state.config.search.tail_size as u64);
                assert_eq!(sources, vec!["src-a".to_string()]);
            }
            event => panic!("expected preview surrounding search, got {event:?}"),
        }
    }

    #[test]
    fn stale_preview_result_is_ignored_while_new_anchor_is_pending() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry(7),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );

        state
            .tui
            .preview_pane
            .apply_surrounding(6, vec![entry(5), entry(6), entry(7)]);

        assert_eq!(state.tui.preview_pane.status, PreviewStatus::NoSelection);
        assert_eq!(state.tui.preview_pane.anchor_seq, None);
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
    fn new_selected_entry_same_seq_preserves_info_pane_scroll() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(7),
            matches: Vec::new(),
        });
        state.tui.info_pane_scroll_offset = 3;

        let state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry(7),
                matches: Vec::new(),
            })),
            state,
        );

        assert_eq!(state.tui.info_pane_scroll_offset, 3);
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

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::SourceSelector));
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.source_selector.cursor, 0);
        assert_eq!(state.tui.source_selector.open_sources.len(), 2);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.focused, Slot::QueryBox);
    }

    #[test]
    fn question_mark_opens_and_closes_help_without_changing_focus() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;

        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::Help));
        assert_eq!(state.tui.focused, Slot::QueryBox);

        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.focused, Slot::QueryBox);
    }

    #[test]
    fn opening_help_replaces_source_selector() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a")];
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::Help));
    }

    #[test]
    fn opening_source_selector_replaces_help() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.producer.sources = vec![source("src-a")];
        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::SourceSelector));
        assert_eq!(state.tui.source_selector.open_sources.len(), 1);
    }

    #[test]
    fn escape_closes_active_popup() {
        let state = AppState::new(Config::default()).expect("app state");
        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        let state = handle_tui_event(input(KeyCode::Esc), state);

        assert_eq!(state.tui.active_popup(), None);
    }

    #[test]
    fn help_swallows_tab_and_text_input() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        let state = handle_tui_event(input(KeyCode::Char('?')), state);

        let state = handle_tui_event(input(KeyCode::Tab), state);
        let state = handle_tui_event(input(KeyCode::Char('x')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::Help));
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.query_box_textarea.lines().join("\n").trim(), "");
    }

    #[test]
    fn ctrl_c_still_quits_when_help_is_open() {
        let state = AppState::new(Config::default()).expect("app state");
        let mut state = handle_tui_event(input(KeyCode::Char('?')), state);

        state = handle_tui_event(ctrl_input(KeyCode::Char('c')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::Help));
        assert!(state.event_bus.quit_rx.try_recv().is_ok());
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

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::SourceSelector));
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.query_box_textarea.lines().join("\n").trim(), "");
    }

    #[test]
    fn source_selector_escape_and_enter_close_popup() {
        let state = AppState::new(Config::default()).expect("app state");
        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        let state = handle_tui_event(input(KeyCode::Esc), state);

        assert_eq!(state.tui.active_popup(), None);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);
        let state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.tui.active_popup(), None);
    }

    #[test]
    fn ctrl_c_still_quits_when_source_selector_is_open() {
        let state = AppState::new(Config::default()).expect("app state");
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        state = handle_tui_event(ctrl_input(KeyCode::Char('c')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::SourceSelector));
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

    #[test]
    fn field_picker_enter_applies_selected_fields_and_closes() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry_with_fields(
                    7,
                    HashMap::from([
                        ("status".to_string(), json!(500)),
                        ("trace".to_string(), json!("abc")),
                    ]),
                ),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );
        let _ = recv_search_event(&mut state);
        state.tui.preview_pane.mode = PreviewMode::Expanded;
        state.tui.preview_pane.open_field_selection();
        state.tui.open_field_picker();
        state.tui.toggle_field_picker_key("trace");

        let mut state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(
            state.tui.preview_pane.mode,
            PreviewMode::FieldMatched {
                predicates: vec![FieldPredicate {
                    key: "trace".to_string(),
                    value: json!("abc"),
                }],
            }
        );
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query:
                    Query::FieldMatched {
                        anchor_seq_id,
                        predicates,
                        ..
                    },
                sources,
            } => {
                assert_eq!(target, SearchTarget::PreviewPane);
                assert_eq!(anchor_seq_id, 7);
                assert_eq!(
                    predicates,
                    vec![FieldPredicate {
                        key: "trace".to_string(),
                        value: json!("abc"),
                    }]
                );
                assert!(sources.is_empty());
            }
            event => panic!("expected field-matched preview search, got {event:?}"),
        }
    }

    #[test]
    fn field_picker_escape_cancels_without_losing_previous_mode() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.preview_pane.mode = PreviewMode::Expanded;
        state.tui.preview_pane.open_field_selection();
        state.tui.open_field_picker();

        let state = handle_tui_event(input(KeyCode::Esc), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Expanded);
    }

    #[test]
    fn field_picker_enter_with_no_selection_keeps_previous_mode() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry_with_fields(7, HashMap::from([("trace".to_string(), json!("abc"))])),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );
        let _ = recv_search_event(&mut state);
        state.tui.preview_pane.mode = PreviewMode::Expanded;
        state.tui.preview_pane.open_field_selection();
        state.tui.open_field_picker();

        let mut state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::FieldPicker));
        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Expanded);
        assert!(state.event_bus.search_event_rx.try_recv().is_err());
    }

    #[test]
    fn selected_entry_change_rebuilds_open_field_picker_selection() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry_with_fields(
                    7,
                    HashMap::from([
                        ("status".to_string(), json!(500)),
                        ("trace".to_string(), json!("abc")),
                    ]),
                ),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );
        let _ = recv_search_event(&mut state);
        state.tui.preview_pane.open_field_selection();
        state.tui.open_field_picker();
        state.tui.toggle_field_picker_key("status");
        state.tui.toggle_field_picker_key("trace");

        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry_with_fields(
                    8,
                    HashMap::from([
                        ("trace".to_string(), json!("abc")),
                        ("user".to_string(), json!("calam")),
                    ]),
                ),
                matches: Vec::new(),
            })),
            state,
        );

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::FieldPicker));
        assert_eq!(
            state.tui.selected_field_picker_keys(),
            vec!["trace".to_string()]
        );
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                query:
                    Query::Surrounding {
                        middle_seq_id: 8, ..
                    },
                ..
            } => {}
            event => panic!("expected surrounding preview redispatch, got {event:?}"),
        }
    }

    #[test]
    fn ctrl_p_cycles_preview_modes_and_opens_field_picker() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry_with_fields(7, HashMap::from([("trace".to_string(), json!("abc"))])),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );
        let _ = recv_search_event(&mut state);

        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('p')), state);

        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Expanded);
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query:
                    Query::Surrounding {
                        middle_seq_id: 7, ..
                    },
                ..
            } => assert_eq!(target, SearchTarget::PreviewPane),
            event => panic!("expected expanded surrounding preview search, got {event:?}"),
        }

        state = handle_tui_event(ctrl_input(KeyCode::Char('p')), state);

        assert_eq!(state.tui.active_popup(), Some(ActivePopup::FieldPicker));
        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Expanded);

        state = handle_tui_event(ctrl_input(KeyCode::Char('p')), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Surrounding);
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                target,
                query:
                    Query::Surrounding {
                        middle_seq_id: 7, ..
                    },
                ..
            } => assert_eq!(target, SearchTarget::PreviewPane),
            event => panic!("expected surrounding preview search after picker skip, got {event:?}"),
        }
    }

    #[test]
    fn toggle_select_mode_flips_state() {
        let state = AppState::new(Config::default()).expect("app state");
        assert!(!state.tui.select_mode);

        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            state,
        );
        assert!(state.tui.select_mode);

        let state = handle_tui_event(
            TuiEvent::Input(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            state,
        );
        assert!(!state.tui.select_mode);
    }

    #[test]
    fn yank_with_no_selection_is_silent_noop() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        assert!(state.tui.selected_entry.is_none());

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        assert!(state.tui.status_message.is_none());
    }

    #[tokio::test]
    async fn yank_with_query_box_focused_does_not_trigger() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(1),
            matches: Vec::new(),
        });

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        assert!(state.tui.status_message.is_none());
    }

    #[test]
    fn yank_with_selection_sets_status_message() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(42),
            matches: Vec::new(),
        });

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        let msg = state.tui.status_message.as_ref().map(|(m, _)| m.as_str());
        assert!(
            msg.is_some_and(|m| m.contains("sent yank") && m.contains("bytes")),
            "expected 'sent yank ... bytes' message, got {msg:?}"
        );
    }

    #[test]
    fn yank_with_popup_open_does_not_trigger() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(1),
            matches: Vec::new(),
        });
        state
            .tui
            .open_popup(crate::state::tui_state::ActivePopup::Help);

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        assert!(state.tui.status_message.is_none());
    }

    #[test]
    fn multiplexer_hint_shown_flag_prevents_repeat() {
        // Simulate multiplexer detection by pre-setting the shown flag. When the
        // flag is already true, queue_status_message is never called regardless
        // of $TMUX / $ZELLIJ env vars.
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: entry(1),
            matches: Vec::new(),
        });
        state.tui.multiplexer_clipboard_hint_shown = true;

        let state = handle_tui_event(input(KeyCode::Char('y')), state);
        assert!(state.tui.status_message_pending.is_none());
    }

    #[test]
    fn yank_status_message_normal_when_payload_is_small() {
        // Small entry → well below the 8KB base64 threshold
        let small_entry = entry_with_fields(
            99,
            std::collections::HashMap::from([("k".to_string(), json!("v"))]),
        );
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: small_entry,
            matches: Vec::new(),
        });

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        let msg = state
            .tui
            .status_message
            .as_ref()
            .map(|(m, _)| m.as_str())
            .unwrap_or("");
        assert!(
            !msg.contains("exceeds 8KB"),
            "expected normal message for small payload, got: {msg}"
        );
    }

    #[test]
    fn yank_status_message_warns_when_payload_exceeds_8kb_threshold() {
        // Large pad field → well above the 8KB base64 threshold for any overhead
        let large_entry = entry_with_fields(
            99,
            std::collections::HashMap::from([(
                "pad".to_string(),
                serde_json::Value::String("x".repeat(100_000)),
            )]),
        );
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::Main;
        state.tui.selected_entry = Some(SelectedEntry {
            entry: large_entry,
            matches: Vec::new(),
        });

        let state = handle_tui_event(input(KeyCode::Char('y')), state);

        let msg = state
            .tui
            .status_message
            .as_ref()
            .map(|(m, _)| m.as_str())
            .unwrap_or("");
        assert!(
            msg.contains("exceeds 8KB"),
            "expected 8KB warning message, got: {msg}"
        );
    }

    #[test]
    fn ctrl_p_closes_existing_popup_then_switches_preview_mode() {
        let mut state = handle_tui_event(
            TuiEvent::NewSelectedEntry(Some(SelectedEntry {
                entry: entry(7),
                matches: Vec::new(),
            })),
            AppState::new(Config::default()).expect("app state"),
        );
        let _ = recv_search_event(&mut state);
        state.producer.sources = vec![source("src-a")];
        state = handle_tui_event(ctrl_input(KeyCode::Char('s')), state);

        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('p')), state);

        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.preview_pane.mode, PreviewMode::Expanded);
        match recv_search_event(&mut state) {
            SearchEvent::Search {
                query:
                    Query::Surrounding {
                        middle_seq_id: 7, ..
                    },
                ..
            } => {}
            event => panic!("expected preview mode redispatch, got {event:?}"),
        }
    }

    #[test]
    fn slash_focuses_query_box_and_does_not_insert_slash() {
        let state = AppState::new(Config::default()).expect("app state");
        assert_eq!(state.tui.focused, Slot::Main);

        let state = handle_tui_event(input(KeyCode::Char('/')), state);

        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.query_box_textarea.lines().join("").trim(), "");
    }

    #[tokio::test]
    async fn slash_when_query_box_already_focused_inserts_into_query() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;

        let state = handle_tui_event(input(KeyCode::Char('/')), state);

        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(state.tui.query_box_textarea.lines().join(""), "/");
    }

    #[test]
    fn enter_returns_focus_from_query_box_to_main_and_preserves_query() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        // Pre-type a query into the textarea.
        state.tui.query_box_textarea.insert_str("error");

        let state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.tui.focused, Slot::Main);
        assert_eq!(
            state.tui.query_box_textarea.lines().join("").trim(),
            "error"
        );
    }

    #[test]
    fn tab_does_not_cycle_focus() {
        let state = AppState::new(Config::default()).expect("app state");
        assert_eq!(state.tui.focused, Slot::Main);

        let state = handle_tui_event(input(KeyCode::Tab), state);

        assert_eq!(state.tui.focused, Slot::Main);
    }

    #[test]
    fn after_slash_enter_j_dispatches_scroll_to_log_pane() {
        let state = AppState::new(Config::default()).expect("app state");

        let state = handle_tui_event(input(KeyCode::Char('/')), state);
        assert_eq!(state.tui.focused, Slot::QueryBox);

        let state = handle_tui_event(input(KeyCode::Enter), state);
        assert_eq!(state.tui.focused, Slot::Main);

        let mut state = handle_tui_event(input(KeyCode::Char('j')), state);

        assert!(matches!(
            state.event_bus.tui_event_rx.try_recv(),
            Ok(TuiEvent::Scroll(ratatui::widgets::ScrollDirection::Forward))
        ));
    }

    #[test]
    fn log_pane_action_keys_do_not_mutate_query_text() {
        // j, k, g, G, w, y should not insert into the query box while focused is Main.
        let state = AppState::new(Config::default()).expect("app state");
        assert_eq!(state.tui.focused, Slot::Main);

        let state = handle_tui_event(input(KeyCode::Char('j')), state);
        let state = handle_tui_event(input(KeyCode::Char('k')), state);
        let state = handle_tui_event(input(KeyCode::Char('g')), state);
        let state = handle_tui_event(input(KeyCode::Char('G')), state);
        let state = handle_tui_event(input(KeyCode::Char('w')), state);

        assert_eq!(state.tui.query_box_textarea.lines().join("").trim(), "");
    }

    #[test]
    fn esc_while_query_box_focused_does_not_clear_query() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;
        state.tui.query_box_textarea.insert_str("error");

        let state = handle_tui_event(input(KeyCode::Esc), state);

        // Esc should not return focus (that's Enter) and should not clear query.
        assert_eq!(state.tui.focused, Slot::QueryBox);
        assert_eq!(
            state.tui.query_box_textarea.lines().join("").trim(),
            "error"
        );
    }

    #[test]
    fn query_box_focus_preserved_across_help_popup_open_close() {
        let mut state = AppState::new(Config::default()).expect("app state");
        state.tui.focused = Slot::QueryBox;

        let state = handle_tui_event(input(KeyCode::Char('?')), state);
        assert_eq!(state.tui.active_popup(), Some(ActivePopup::Help));
        assert_eq!(state.tui.focused, Slot::QueryBox);

        let state = handle_tui_event(input(KeyCode::Char('?')), state);
        assert_eq!(state.tui.active_popup(), None);
        assert_eq!(state.tui.focused, Slot::QueryBox);
    }

    #[test]
    fn startup_focus_is_main() {
        let state = AppState::new(Config::default()).expect("app state");
        assert_eq!(state.tui.focused, Slot::Main);
    }
}
