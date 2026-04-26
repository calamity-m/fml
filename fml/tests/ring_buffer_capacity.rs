#![cfg(feature = "integration")]

mod common;

use fml::{
    app::App,
    config::Config,
    event::{QuitEvent, TuiEvent},
};

use common::make_entry;

/// Saturate the ring buffer past its declared capacity, then drive one render
/// + quit through the real event loop. The assertion is "did not panic" — at
/// 1M entries the store eviction path and the LogPane render path are both
/// stressed simultaneously.
#[tokio::test(flavor = "multi_thread")]
async fn ring_buffer_at_capacity_does_not_panic() {
    let mut config = Config::default();
    config.store.capacity = 1_000_000;
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    for i in 1..=1_100_000u64 {
        app.state
            .store
            .insert(make_entry(&format!("entry {i}"), "src-a"));
    }

    app.state
        .event_bus
        .tui_event_tx
        .send(TuiEvent::Render)
        .expect("send render event");
    app.state
        .event_bus
        .quit_tx
        .send(QuitEvent {})
        .await
        .expect("send quit");

    let _ = app.run_until_quit().await;
}
