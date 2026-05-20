#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fml::{
    app::App,
    config::{
        Config,
        tui::{LogLevelThemeConfig, LogMatchStyle},
    },
    event::{ProducerEvent, Query, QuitEvent, SearchEvent, SearchHit, SearchTarget, TuiEvent},
    log::LogLevel,
    producer, search,
    state::tui_state::log_pane_state::ScrollMode,
};
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
    widgets::ScrollDirection,
};

use common::{buffer_to_string, make_entry, make_entry_with_source_display};

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

fn buffer_to_underlined_snapshot(buf: &Buffer) -> String {
    let area = buf.area();
    let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);

    for y in 0..area.height {
        let mut in_underlined_run = false;
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            let underlined = cell.modifier.contains(Modifier::UNDERLINED);

            if underlined && !in_underlined_run {
                out.push('[');
                in_underlined_run = true;
            } else if !underlined && in_underlined_run {
                out.push(']');
                in_underlined_run = false;
            }

            out.push_str(cell.symbol());
        }

        if in_underlined_run {
            out.push(']');
        }
        out.push('\n');
    }

    out
}

fn scrollbar_thumb_rows(buf: &Buffer) -> Vec<u16> {
    // The scrollbar's exact column depends on the configured
    // `sidebar_width_percent`, so scan the rows within the log pane and
    // report any that contain a thumb glyph. The log pane is always the
    // left-hand block, so any "█" before the info pane belongs to it.
    let area = buf.area();
    (1..area.height.saturating_sub(5))
        .filter(|y| (0..area.width).any(|x| buf[(x, *y)].symbol() == "█"))
        .collect()
}

async fn type_query(tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>, text: &str) {
    tui_tx.send(key(KeyCode::Tab)).expect("focus query box");
    for ch in text.chars() {
        tui_tx
            .send(key(KeyCode::Char(ch)))
            .expect("send query input");
    }
}

fn fuzzy_hit(seq_id: u64) -> SearchHit {
    SearchHit {
        seq_id,
        matches: Vec::new(),
    }
}

fn reducer_app_with_entries(count: u64) -> App<ratatui::backend::TestBackend> {
    let mut app = App::with_test_backend(fuzzy_config(), 80, 24).expect("app construction");
    for seq in 1..=count {
        app.state = producer::handle_producer_event(
            ProducerEvent::StoreEvent(make_entry(&format!("entry {seq}"), "src-a")),
            app.state,
        );
    }
    app.state
        .search
        .client_mut(SearchTarget::LogPane)
        .latest_request_id = 1;
    app.state
        .tui
        .log_pane
        .on_search_started(&Query::Fuzzy("entry".to_string()));
    app.state
        .tui
        .log_pane
        .set_height(10, &mut app.state.tui.log_pane_cursor_row);
    app
}

fn apply_fuzzy_emission(
    mut app: App<ratatui::backend::TestBackend>,
    best_first: &[u64],
) -> App<ratatui::backend::TestBackend> {
    app.state = search::handle_search_event(
        SearchEvent::Result {
            target: SearchTarget::LogPane,
            query: Query::Fuzzy("entry".to_string()),
            results: best_first.iter().copied().map(fuzzy_hit).collect(),
            request_id: 1,
            complete: true,
            progress: None,
        },
        app.state,
    );
    app
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
            .items()
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert!(rendered.contains("1 INFO src-a needle"));
    assert!(rendered.contains("3 INFO src-a needle"));
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(5));
    assert_eq!(app.state.tui.preview_pane.anchor_seq, Some(5));
    assert!(
        app.state
            .tui
            .log_pane
            .fuzzy_matches_for(5)
            .is_some_and(|matches| matches.iter().any(|m| m.key == "msg"))
    );
}

#[tokio::test]
async fn submitting_fuzzy_query_renders_search_scrollbar_when_results_overflow() {
    let app = App::with_test_backend(fuzzy_config(), 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        for i in 1..=30u64 {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry(
                    &format!("needle {i}"),
                    "src-a",
                )))
                .await
                .expect("send producer event");
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        type_query(&tui_tx, "needle").await;
        tokio::time::sleep(Duration::from_millis(160)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(20)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let buffer = app.terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);

    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Search);
    assert!(rendered.contains(" FML [SEARCH] "));
    assert!(
        app.state.tui.log_pane.scrollbar_metrics().is_some(),
        "overflowing fuzzy results should expose scrollbar metrics"
    );
    assert!(
        !scrollbar_thumb_rows(buffer).is_empty(),
        "overflowing fuzzy results should render a scrollbar thumb"
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
            .items()
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(5));
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
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(6));
    assert!(app.state.tui.log_pane.fuzzy_matches_is_empty());
}

#[tokio::test]
async fn fuzzy_highlighting_snapshot_uses_display_name_and_marks_matches() {
    let mut config = fuzzy_config();
    config.tui.default_theme.log_match_style = LogMatchStyle::Underline;
    config.tui.default_theme.log_level = LogLevelThemeConfig {
        info_fg: Color::Blue,
        ..LogLevelThemeConfig::default()
    };

    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        producer_tx
            .send(ProducerEvent::StoreEvent(make_entry_with_source_display(
                "needle",
                "src-a",
                "Service A",
            )))
            .await
            .expect("send producer event");
        tokio::time::sleep(Duration::from_millis(80)).await;
        type_query(&tui_tx, "Service").await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(20)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    let buffer = app.terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);
    let row = rendered
        .lines()
        .position(|line| line.contains("1 INFO Service A needle"))
        .expect("expected rendered fuzzy result");
    let col = rendered
        .lines()
        .nth(row)
        .expect("expected row")
        .find("Service")
        .expect("expected service column");
    let unhighlighted_col = rendered
        .lines()
        .nth(row)
        .expect("expected row")
        .find("needle")
        .expect("expected needle column");

    assert!(rendered.contains("Service A"));
    assert!(!rendered.contains("src-a needle"));
    assert_ne!(buffer[(col as u16, row as u16)].fg, Color::Blue);
    assert!(
        buffer[(col as u16, row as u16)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "matched character should carry fuzzy highlight style"
    );
    assert!(
        !buffer[(unhighlighted_col as u16, row as u16)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "unmatched rendered field should keep only the level style"
    );
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(1));
    assert_eq!(
        app.state.tui.log_pane.items()[0].level,
        Some(LogLevel::Info)
    );
    insta::assert_snapshot!(buffer_to_underlined_snapshot(buffer));
}

#[test]
fn live_fuzzy_rerank_preserves_selected_entry() {
    let app = reducer_app_with_entries(4);
    let mut app = apply_fuzzy_emission(app, &[3, 2, 1]);
    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(2), &mut app.state.tui.log_pane_cursor_row);
    app.state.tui.log_pane_cursor_row = 1;

    let app = apply_fuzzy_emission(app, &[2, 4, 1]);

    assert_eq!(app.state.tui.log_pane.mode, ScrollMode::Search);
    assert_eq!(
        app.state
            .tui
            .log_pane
            .items()
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 4, 2]
    );
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(2));
    assert_eq!(app.state.tui.log_pane_cursor_row, 2);
}

#[test]
fn live_fuzzy_rerank_falls_back_when_selected_entry_disappears() {
    let app = reducer_app_with_entries(6);
    let mut app = apply_fuzzy_emission(app, &[4, 3, 2, 1]);
    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(2), &mut app.state.tui.log_pane_cursor_row);
    app.state.tui.log_pane_cursor_row = 1;

    let app = apply_fuzzy_emission(app, &[6, 5, 1]);

    assert_eq!(
        app.state
            .tui
            .log_pane
            .items()
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 5, 6]
    );
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(5));
    assert_eq!(app.state.tui.log_pane_cursor_row, 1);
}

#[test]
fn live_fuzzy_rerank_keeps_highest_rank_pinned() {
    let app = reducer_app_with_entries(4);
    let app = apply_fuzzy_emission(app, &[3, 2, 1]);

    let app = apply_fuzzy_emission(app, &[4, 2, 3, 1]);

    assert_eq!(
        app.state
            .tui
            .log_pane
            .items()
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 2, 4]
    );
    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(4));
    assert_eq!(app.state.tui.log_pane_cursor_row, 3);
}
