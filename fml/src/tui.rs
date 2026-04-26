use std::{io::stdout, time::Duration};

pub mod keybindings;
pub mod keybinds;
pub mod layout;
pub mod widgets;

use crossterm::{
    ExecutableCommand as _,
    event::{DisableMouseCapture, Event as CrosstermEvent, EventStream, KeyEventKind},
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use futures_util::{FutureExt as _, StreamExt as _};
use ratatui::{Terminal, backend::Backend};
use tokio::{sync::mpsc, time::interval};
use tracing::{debug, error, trace, warn};

use crate::{
    config::tui::TuiConfig,
    error::FmlError,
    event::{QuitEvent, TuiEvent},
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
        TuiEvent::Render => {
            trace!("received render event");

            state
        }
        TuiEvent::Error(err) => {
            error!("received error event - {}", err);

            todo!()
        }
        TuiEvent::Input(key) => {
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

    if let Err(err) = result {
        if let Err(err) = state
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
}
