#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
};

use common::{buffer_to_string, make_entry};

/// Verifies that the log pane continuously tails new entries: a first batch
/// is written, then a second batch arrives and pushes the older entries out
/// of the rendered window. The final snapshot should show only the latest
/// entries with the title in `TAIL` mode.
#[tokio::test]
async fn tail_pushes_old_entries_out_of_window() {
    let config = Config::default();
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        // First batch — these should later be pushed out of view.
        for i in 1..=20u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("first {i}"),
                    "src-a",
                )))
                .await
                .expect("send producer event");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Second batch — newest entries that should occupy the bottom of the
        // visible window when we render.
        for i in 1..=20u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("second {i}"),
                    "src-b",
                )))
                .await
                .expect("send producer event");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    insta::assert_snapshot!(buffer_to_string(app.terminal.backend().buffer()));
}
