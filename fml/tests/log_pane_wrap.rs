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

fn long_entry(seq: u64) -> Arc<LogEntry> {
    Arc::new(LogEntry {
        seq,
        msg: format!(
            "entry {seq} alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi"
        ),
        ts: Utc::now(),
        level: Some(LogLevel::Info),
        source: Source {
            producer: "fake".to_string(),
            id: "src-a".to_string(),
            display_name: "src-a".to_string(),
            group: None,
        },
        fields: HashMap::new(),
    })
}

fn short_entry(seq: u64) -> Arc<LogEntry> {
    Arc::new(LogEntry {
        seq,
        msg: format!("short {seq}"),
        ts: Utc::now(),
        level: Some(LogLevel::Info),
        source: Source {
            producer: "fake".to_string(),
            id: "src-a".to_string(),
            display_name: "src-a".to_string(),
            group: None,
        },
        fields: HashMap::new(),
    })
}

fn render_app(app: &mut App<ratatui::backend::TestBackend>) -> String {
    tui::render(&mut app.state, &mut app.terminal);
    buffer_to_string(app.terminal.backend().buffer())
}

#[test]
fn wrap_on_snapshot_long_msg_renders_with_hanging_indent() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    let mut cursor = 0;
    app.state.tui.log_pane.set_line_wrap(true, &mut cursor);
    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries: vec![long_entry(1)],
            retained_bounds: (1, 1),
        },
        &mut app.state.tui.log_pane_cursor_row,
    );

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn long_msg_renders_on_one_line_when_wrap_off() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries: vec![long_entry(1)],
            retained_bounds: (1, 1),
        },
        &mut app.state.tui.log_pane_cursor_row,
    );

    let buffer = render_app(&mut app);
    // The pane is 80 cells wide (minus borders/info-pane). The long msg has
    // many words; with wrap off, only the first line should contain content.
    // Confirm that "kappa" (a later word) does NOT appear anywhere — it's
    // clipped past the right edge.
    assert!(
        !buffer.contains("kappa"),
        "expected late words to be clipped in truncated mode:\n{buffer}"
    );
}

#[test]
fn long_msg_wraps_onto_continuation_lines_when_wrap_on() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    let mut cursor = 0;
    app.state.tui.log_pane.set_line_wrap(true, &mut cursor);
    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries: vec![long_entry(1)],
            retained_bounds: (1, 1),
        },
        &mut app.state.tui.log_pane_cursor_row,
    );

    let buffer = render_app(&mut app);
    // In wrapped mode the late words must appear somewhere in the rendered
    // output — wrap continuation lines carry the rest of the msg.
    assert!(
        buffer.contains("kappa"),
        "expected wrapped continuation to include later words:\n{buffer}"
    );
}

#[test]
fn toggle_round_trip_preserves_tail_selection() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    let entries: Vec<Arc<LogEntry>> = (1..=5)
        .map(|s| {
            if s == 5 {
                long_entry(s)
            } else {
                short_entry(s)
            }
        })
        .collect();
    app.state.tui.log_pane.apply_update(
        LogPaneUpdate::Tail {
            entries,
            retained_bounds: (1, 5),
        },
        &mut app.state.tui.log_pane_cursor_row,
    );

    let _ = render_app(&mut app);
    let initial = app.state.tui.log_pane.selected_seq();
    assert!(initial.is_some(), "tail mode should select an entry");

    // Toggle wrap on, render, then back off. Tail-mode selection must stay
    // pointed at the latest entry across both toggles.
    let mut cursor = app.state.tui.log_pane_cursor_row;
    app.state.tui.log_pane.set_line_wrap(true, &mut cursor);
    app.state.tui.log_pane_cursor_row = cursor;
    let _ = render_app(&mut app);
    assert_eq!(
        app.state.tui.log_pane.selected_seq(),
        initial,
        "wrap-on toggle must preserve tail selection"
    );

    let mut cursor = app.state.tui.log_pane_cursor_row;
    app.state.tui.log_pane.set_line_wrap(false, &mut cursor);
    app.state.tui.log_pane_cursor_row = cursor;
    let _ = render_app(&mut app);
    assert_eq!(
        app.state.tui.log_pane.selected_seq(),
        initial,
        "wrap-off toggle must preserve tail selection"
    );
}
