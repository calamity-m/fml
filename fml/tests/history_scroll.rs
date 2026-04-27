#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
    state::tui_state::log_pane_state::ScrollMode,
};
use ratatui::widgets::ScrollDirection;

use common::{buffer_to_string, make_entry};

async fn populate(producer_tx: &tokio::sync::mpsc::Sender<ProducerEvent>, start: u64, end: u64) {
    for i in start..=end {
        producer_tx
            .send(ProducerEvent::StoreEvent(make_entry(
                &format!("entry {i}"),
                "src-a",
            )))
            .await
            .expect("send producer event");
    }
}

#[tokio::test]
async fn repeated_up_scroll_reaches_retained_low() {
    let app = App::with_test_backend(Config::default(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx, 1, 100).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");

        for _ in 0..100 {
            tui_tx
                .send(TuiEvent::Scroll(ScrollDirection::Backward))
                .expect("send scroll event");
        }

        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());
    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::History);
    assert!(rendered.contains(" FML [HISTORY] "));
    assert!(rendered.contains("1 INFO src-a entry 1"));
}

#[tokio::test]
async fn home_jumps_to_retained_low() {
    let app = App::with_test_backend(Config::default(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx, 1, 10_000).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tui_tx.send(TuiEvent::ScrollHead).expect("send scroll head");
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());
    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::History);
    assert!(rendered.contains(" FML [HISTORY] "));
    assert!(rendered.contains("1 INFO src-a entry 1"));
}

#[tokio::test]
async fn history_mode_does_not_return_to_tail_when_new_logs_arrive() {
    let app = App::with_test_backend(Config::default(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx, 1, 30).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tui_tx
            .send(TuiEvent::Scroll(ScrollDirection::Backward))
            .expect("send scroll event");
        tokio::time::sleep(Duration::from_millis(250)).await;

        populate(&producer_tx, 31, 50).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());
    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::History);
    assert!(rendered.contains(" FML [HISTORY] "));
}

#[tokio::test]
async fn end_returns_to_tail_and_newest_logs() {
    let app = App::with_test_backend(Config::default(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx, 1, 100).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tui_tx
            .send(TuiEvent::Scroll(ScrollDirection::Backward))
            .expect("send scroll event");
        tokio::time::sleep(Duration::from_millis(250)).await;
        tui_tx.send(TuiEvent::ScrollTail).expect("send scroll tail");
        tokio::time::sleep(Duration::from_millis(350)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());
    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Tail);
    assert!(rendered.contains(" FML [TAIL] "));
    assert!(rendered.contains("100 INFO src-a entry 100"));
}
