#![cfg(feature = "integration")]

mod common;

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;

use fml::{
    app::App,
    config::Config,
    log::{LogEntry, LogLevel, Source},
    state::tui_state::log_pane_state::ScrollMode,
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

fn render_app(app: &mut App<ratatui::backend::TestBackend>) -> String {
    tui::render(&mut app.state, &mut app.terminal);
    buffer_to_string(app.terminal.backend().buffer())
}

#[test]
fn scrollbar_hides_when_retained_content_fits() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.items = entries(1, 5);
    app.state.tui.log_pane.retained_bounds = (1, 5);
    app.state.tui.log_pane.selected_seq = Some(5);

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn scrollbar_renders_at_top_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::History;
    app.state.tui.log_pane.items = entries(1, 20);
    app.state.tui.log_pane.retained_bounds = (1, 20);
    app.state.tui.log_pane.selected_seq = Some(1);

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn scrollbar_renders_in_the_middle_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::History;
    app.state.tui.log_pane.items = entries(1, 20);
    app.state.tui.log_pane.retained_bounds = (1, 20);
    app.state.tui.log_pane.selected_seq = Some(10);

    insta::assert_snapshot!(render_app(&mut app));
}

#[test]
fn scrollbar_renders_at_bottom_of_retained_window() {
    let config = Config::default();
    let mut app = App::with_test_backend(config, 80, 24).expect("app construction");

    app.state.tui.log_pane.mode = ScrollMode::Tail;
    app.state.tui.log_pane.items = entries(1, 20);
    app.state.tui.log_pane.retained_bounds = (1, 20);
    app.state.tui.log_pane.selected_seq = Some(20);

    insta::assert_snapshot!(render_app(&mut app));
}
