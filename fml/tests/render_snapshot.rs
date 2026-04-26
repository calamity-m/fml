#![cfg(feature = "integration")]

mod common;

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
};

use common::{buffer_to_string, make_entry};

/// Drives the real event loop end-to-end: routes producer events into the
/// store, then issues a render against `TestBackend` and snapshots the
/// resulting buffer. The current LogPane shows placeholder content (per
/// README TODO #3); this snapshot will update once the pane is wired to
/// the store, which is exactly the regression signal we want.
#[tokio::test]
async fn renders_full_tui() {
    let config = Config::default();
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    for i in 1..=5u64 {
        producer_tx
            .send(ProducerEvent::StoreEvent(make_entry(
                &format!("entry {i}"),
                "src-a",
            )))
            .await
            .expect("send producer event");
    }
    tui_tx.send(TuiEvent::Render).expect("send render event");
    quit_tx.send(QuitEvent {}).await.expect("send quit");

    let app = app.run_until_quit().await;
    insta::assert_snapshot!(buffer_to_string(app.terminal.backend().buffer()));
}
