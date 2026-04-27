#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
    state::tui_state::log_pane_state::ScrollMode,
};
use ratatui::widgets::ScrollDirection;

use common::{buffer_to_string, make_entry};

async fn populate(producer_tx: &tokio::sync::mpsc::Sender<ProducerEvent>) {
    for msg in [
        "needle",
        "noise",
        "needle",
        "more noise",
        "needle",
        "unrelated",
    ] {
        producer_tx
            .send(ProducerEvent::StoreEvent(make_entry(msg, "src-a")))
            .await
            .expect("send producer event");
    }
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Input(KeyEvent::new(code, KeyModifiers::NONE))
}

fn fuzzy_config() -> Config {
    let mut config = Config::default();
    config.search.fuzzy_debounce_ms = 10;
    config.search.fuzzy_tick_rate_ms = 10;
    config.search.fuzzy_max_typos = Some(0);
    config.search.tail_poll_interval_ms = 10;
    config.search.history_poll_interval_ms = 10;
    config
}

async fn type_query(tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>, text: &str) {
    tui_tx.send(key(KeyCode::Tab)).expect("focus query box");
    for ch in text.chars() {
        tui_tx
            .send(key(KeyCode::Char(ch)))
            .expect("send query input");
    }
}

#[tokio::test]
async fn submitting_fuzzy_query_renders_ranked_matches() {
    let app = App::with_test_backend(fuzzy_config(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        type_query(&tui_tx, "needle").await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(20)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());

    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Search);
    assert!(rendered.contains(" FML [SEARCH] "));
    assert_eq!(
        app.state
            .tui
            .log_pane
            .items
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert!(rendered.contains("1 INFO src-a needle"));
    assert!(rendered.contains("3 INFO src-a needle"));
    assert!(!rendered.contains("5 INFO src-a needle"));
    assert_eq!(app.state.tui.log_pane.selected_seq, Some(5));
    assert!(
        app.state
            .tui
            .log_pane
            .fuzzy_matches
            .get(&5)
            .is_some_and(|matches| matches.iter().any(|m| m.key == "msg"))
    );
}

#[tokio::test]
async fn fuzzy_result_navigation_clamps_to_rank_boundaries() {
    let app = App::with_test_backend(fuzzy_config(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        type_query(&tui_tx, "needle").await;
        tokio::time::sleep(Duration::from_millis(120)).await;

        tui_tx.send(TuiEvent::ScrollHead).expect("send scroll head");
        tui_tx
            .send(TuiEvent::Scroll(ScrollDirection::Backward))
            .expect("send scroll up");
        tui_tx.send(TuiEvent::ScrollTail).expect("send scroll tail");
        tui_tx
            .send(TuiEvent::Scroll(ScrollDirection::Forward))
            .expect("send scroll down");

        tokio::time::sleep(Duration::from_millis(40)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(20)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;

    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Search);
    assert_eq!(
        app.state
            .tui
            .log_pane
            .items
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert_eq!(app.state.tui.log_pane.selected_seq, Some(5));
}

#[tokio::test]
async fn clearing_fuzzy_query_returns_to_tail_mode() {
    let app = App::with_test_backend(fuzzy_config(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        populate(&producer_tx).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        type_query(&tui_tx, "needle").await;
        tokio::time::sleep(Duration::from_millis(120)).await;

        for _ in 0.."needle".len() {
            tui_tx
                .send(key(KeyCode::Backspace))
                .expect("send query backspace");
        }

        tokio::time::sleep(Duration::from_millis(80)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(20)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let rendered = buffer_to_string(app.terminal.backend().buffer());

    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Tail);
    assert!(rendered.contains(" FML [TAIL] "));
    assert_eq!(app.state.tui.log_pane.selected_seq, Some(6));
    assert!(app.state.tui.log_pane.fuzzy_matches.is_empty());
}
