#![cfg(feature = "integration")]

mod common;

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;

use fml::{
    app::App,
    config::Config,
    log::{LogEntry, LogLevel, Source},
    state::tui_state::log_pane_state::{LogPaneUpdate, ScrollMode},
    tui,
};

use common::buffer_to_string;

fn entry(seq: u64) -> Arc<LogEntry> {
    Arc::new(LogEntry {
        seq,
        msg: format!("entry {seq}"),
        ts: Utc::now(),
        level: Some(LogLevel::Info),
        source: Source {
            id: "src-a".to_string(),
            display_name: "src-a".to_string(),
            group: None,
        },
        fields: HashMap::new(),
    })
}

fn entries(start: u64, end: u64) -> Vec<Arc<LogEntry>> {
    (start..=end).map(entry).collect()
}

fn entries_from(seqs: &[u64]) -> Vec<Arc<LogEntry>> {
    seqs.iter().copied().map(entry).collect()
}

fn render_app(app: &mut App<ratatui::backend::TestBackend>) -> String {
    tui::render(&mut app.state, &mut app.terminal);
    buffer_to_string(app.terminal.backend().buffer())
}

fn log_pane_scrollbar_thumb_rows(app: &mut App<ratatui::backend::TestBackend>) -> Vec<u16> {
    tui::render(&mut app.state, &mut app.terminal);
    let buffer = app.terminal.backend().buffer();
    let area = buffer.area();
    let scrollbar_x = 55;

    (1..area.height.saturating_sub(5))
        .filter(|y| buffer[(scrollbar_x, *y)].symbol() == "█")
        .collect()
}

#[test]
fn scrollbar_hides_when_retained_content_fits() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries: entries(1, 5),
            retained_bounds: (1, 5),
        },
        &mut app.state.tui.absolute_cursor,
    );

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn search_scrollbar_hides_when_fuzzy_results_fit() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Search;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Fuzzy {
            best_first_entries: entries_from(&(1..=5).rev().collect::<Vec<_>>()),
            retained_bounds: (1, 5),
            matches_by_seq: HashMap::new(),
        },
        &mut app.state.tui.absolute_cursor,
    );

    assert!(log_pane_scrollbar_thumb_rows(&mut app).is_empty());
}

#[test]
fn search_scrollbar_renders_at_first_middle_and_last_result() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Search;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Fuzzy {
            best_first_entries: entries_from(&(1..=30).rev().collect::<Vec<_>>()),
            retained_bounds: (1, 30),
            matches_by_seq: HashMap::new(),
        },
        &mut app.state.tui.absolute_cursor,
    );

    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(1), &mut app.state.tui.absolute_cursor);
    let first = log_pane_scrollbar_thumb_rows(&mut app);

    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(15), &mut app.state.tui.absolute_cursor);
    let middle = log_pane_scrollbar_thumb_rows(&mut app);

    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(30), &mut app.state.tui.absolute_cursor);
    let last = log_pane_scrollbar_thumb_rows(&mut app);

    assert!(!first.is_empty());
    assert!(!middle.is_empty());
    assert!(!last.is_empty());
    assert!(first.first() < middle.first());
    assert!(middle.first() < last.first());
}

#[test]
fn search_scrollbar_follows_sticky_cursor_after_fuzzy_rerank() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");
    let mut cursor = 0;
    app.state.tui.log_pane.set_height(10, &mut cursor);

    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Fuzzy {
            best_first_entries: entries_from(&(1..=30).rev().collect::<Vec<_>>()),
            retained_bounds: (1, 30),
            matches_by_seq: HashMap::new(),
        },
        &mut cursor,
    );
    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(24), &mut cursor);
    cursor = 6;

    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Fuzzy {
            best_first_entries: entries_from(
                &std::iter::once(31)
                    .chain(std::iter::once(24))
                    .chain((1..=30).rev().filter(|seq| *seq != 24))
                    .collect::<Vec<_>>(),
            ),
            retained_bounds: (1, 31),
            matches_by_seq: HashMap::new(),
        },
        &mut cursor,
    );

    assert_eq!(app.state.tui.log_pane.selected_seq(), Some(24));
    let rendered = render_app(&mut app);

    assert!(rendered.contains(" FML [SEARCH] "));
    assert!(rendered.contains("> 30 INFO src-a entry 24"));
    assert_eq!(
        app.state
            .tui
            .log_pane
            .scrollbar_metrics()
            .map(|metrics| metrics.position),
        Some(29)
    );
    assert!(!log_pane_scrollbar_thumb_rows(&mut app).is_empty());
}

#[test]
fn scrollbar_renders_at_top_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::History;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::History {
            entries: entries(1, 20),
            retained_bounds: (1, 20),
        },
        &mut app.state.tui.absolute_cursor,
    );
    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(1), &mut app.state.tui.absolute_cursor);

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn scrollbar_renders_in_the_middle_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::History;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::History {
            entries: entries(1, 20),
            retained_bounds: (1, 20),
        },
        &mut app.state.tui.absolute_cursor,
    );
    app.state
        .tui
        .log_pane
        .set_selected_seq(Some(10), &mut app.state.tui.absolute_cursor);

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn scrollbar_renders_at_bottom_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries: entries(1, 20),
            retained_bounds: (1, 20),
        },
        &mut app.state.tui.absolute_cursor,
    );

    insta::assert_snapshot!(render_app(&mut app));
}
