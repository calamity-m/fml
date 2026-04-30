#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
};

use common::{buffer_to_string, make_entry};

/// Drives the real event loop end-to-end: routes producer events into the
/// store, lets the tail search emit a result back into TUI state, then issues
/// a render against `TestBackend` and snapshots the resulting buffer. The
/// snapshot exercises the wired-up tail path.
#[tokio::test]
async fn renders_full_tui() {
    let config = Config::default();
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    // The driver task only owns Send-able senders, so the AppState (which
    // holds non-Send `dyn FmlWidget` boxes) can stay on the main task.
    tokio::spawn(async move {
        for i in 1..=5u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("entry {i}"),
                    "src-a",
                )))
                .await
                .expect("send producer event");
        }

        // Give the tail worker time to tick (default 150ms poll interval) and
        // the resulting `SearchEvent::Result` time to flow back into TUI state.
        tokio::time::sleep(Duration::from_millis(400)).await;

        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    insta::assert_snapshot!(buffer_to_string(app.terminal.backend().buffer()));
}

#[tokio::test]
async fn renders_help_popup() {
    let config = Config::default();
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        for i in 1..=3u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("entry {i}"),
                    "src-a",
                )))
                .await
                .expect("send producer event");
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
        tui_tx
            .send(TuiEvent::Input(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('?'),
                crossterm::event::KeyModifiers::NONE,
            )))
            .expect("send help input");
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    insta::assert_snapshot!(buffer_to_string(app.terminal.backend().buffer()));
}
