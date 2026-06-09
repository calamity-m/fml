#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
    log::Source,
};

use common::{buffer_to_string, make_entry};

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Input(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(code: KeyCode) -> TuiEvent {
    TuiEvent::Input(KeyEvent::new(code, KeyModifiers::CONTROL))
}

/// End-to-end through the production event loop: entries stream in, the
/// startup pane tails them, a vertical split clones the view, and both panes
/// render the latest entries with the TAIL badge in the status line.
#[tokio::test]
async fn tail_streams_into_split_panes() {
    let config = Config::default();
    let app = App::with_test_backend(config, 100, 30).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        producer_tx
            .send(ProducerEvent::SourceFound(Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "src-a".to_string(),
                group: None,
            }))
            .await
            .expect("send source");
        for i in 1..=40u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("tail entry {i}"),
                    "src-a",
                )))
                .await
                .expect("send entry");
        }
        // Let the tail worker pick up the entries, then split.
        tokio::time::sleep(Duration::from_millis(300)).await;
        tui_tx.send(ctrl(KeyCode::Char('w'))).expect("send ctrl-w");
        tui_tx.send(key(KeyCode::Char('v'))).expect("send v");
        tokio::time::sleep(Duration::from_millis(300)).await;
        tui_tx.send(TuiEvent::Render).expect("send render");
        tokio::time::sleep(Duration::from_millis(100)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;

    assert_eq!(app.state.workspace.tab().panes.len(), 2);
    for pane in &app.state.workspace.tab().panes {
        assert!(pane.follow, "both panes should still be tailing");
        let entries = pane.view.entries();
        assert_eq!(
            entries.last().map(|entry| entry.msg.as_str()),
            Some("tail entry 40"),
            "pane should hold the newest entry"
        );
    }

    let rendered = buffer_to_string(app.terminal.backend().buffer());
    assert!(rendered.contains("TAIL"), "status shows TAIL:\n{rendered}");
    assert!(
        rendered.contains("tail entry 40"),
        "newest entry rendered:\n{rendered}"
    );
}

/// Searching narrows the pane to fuzzy matches; confirming records hits.
#[tokio::test]
async fn search_narrows_pane_to_matches() {
    let config = Config::default();
    let app = App::with_test_backend(config, 100, 30).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        for i in 1..=20u64 {
            let msg = if i % 5 == 0 {
                format!("payment failed {i}")
            } else {
                format!("ok request {i}")
            };
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(&msg, "src-a")))
                .await
                .expect("send entry");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        // `/payment` then Enter to confirm.
        tui_tx.send(key(KeyCode::Char('/'))).expect("send slash");
        for c in "payment".chars() {
            tui_tx.send(key(KeyCode::Char(c))).expect("send char");
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        tui_tx.send(key(KeyCode::Enter)).expect("send enter");
        tokio::time::sleep(Duration::from_millis(100)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;

    let pane = app.state.workspace.focused_pane();
    assert_eq!(
        pane.hits,
        vec![5, 10, 15, 20],
        "confirmed hits are the payment entries"
    );
    let entries = pane.view.entries();
    assert!(!entries.is_empty());
    assert!(
        entries
            .iter()
            .all(|entry| entry.msg.starts_with("payment failed")),
        "results view holds only matches"
    );
}
