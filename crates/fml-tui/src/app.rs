//! Top-level application state and the main event loop.
//!
//! [`App::run`] sets up the terminal, drives the crossterm event loop, and
//! tears everything down cleanly on exit or panic.

use crate::{
    event::{self, AppEvent},
    theme::Theme,
    widgets::{
        command_bar::{Command, CommandBar, CommandBarState},
        help::HelpPopup,
        log_stream::{LogStream, LogStreamState},
        producer_tree::{ProducerTree, ProducerTreeState, TreeNode},
        query_bar::{QueryBar, QueryBarState},
        tab_bar::TabBar,
    },
};
use crossterm::{
    event::{self as ct_event, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use fml_core::{config::Config, LogEntry};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction as LayoutDir, Layout, Rect},
    Frame, Terminal,
};
use std::{io, time::Duration};

// ---------------------------------------------------------------------------
// Focus + tab types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Stream,
    QueryBar,
    /// Vim-style `:` command line is active.
    Command,
}

pub enum TabKind {
    Main,
    Freeze(String),
    Correlate { field: String, value: String },
}

pub struct TabState {
    pub label: String,
    pub kind: TabKind,
    pub tree: ProducerTreeState,
    pub stream: LogStreamState,
    pub query: QueryBarState,
    pub dirty: bool,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

pub struct AppState {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub focus: Focus,
    /// Focus state before entering command mode, restored on exit.
    pub prev_focus: Focus,
    pub theme: Theme,
    pub config: Config,
    pub show_help: bool,
    pub command_bar: CommandBarState,
    pub quit: bool,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    state: AppState,
}

impl App {
    pub fn new(entries: Vec<LogEntry>, config: Config, theme: Theme) -> Self {
        // Build producer tree from unique producers in the mock data
        let producers: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            entries
                .iter()
                .filter_map(|e| {
                    if seen.insert(e.producer.clone()) {
                        Some(e.producer.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        let children: Vec<TreeNode> = producers
            .into_iter()
            .map(|p| TreeNode::new(p.clone(), p))
            .collect();

        let root = TreeNode::new("__root__", "fml-demo").with_children(children);

        let mut tree = ProducerTreeState::default();
        tree.nodes = vec![root];

        let show_timestamps = config.ui.show_timestamps;
        let mut stream = LogStreamState::new(entries);
        stream.show_timestamps = show_timestamps;

        let main_tab = TabState {
            label: "1:main".to_string(),
            kind: TabKind::Main,
            tree,
            stream,
            query: QueryBarState::default(),
            dirty: false,
        };

        let state = AppState {
            tabs: vec![main_tab],
            active_tab: 0,
            focus: Focus::Tree,
            prev_focus: Focus::Tree,
            theme,
            config,
            show_help: false,
            command_bar: CommandBarState::default(),
            quit: false,
        };

        App { state }
    }

    /// Set up the terminal, run the event loop, and restore the terminal on exit.
    pub fn run(mut self) -> anyhow::Result<()> {
        install_panic_hook();

        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        // Always restore terminal, even if the loop returned an error
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = terminal.show_cursor();

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        loop {
            {
                let s = &self.state;
                terminal.draw(|frame| draw(frame, s))?;
            }

            if self.state.quit {
                break;
            }

            if ct_event::poll(Duration::from_millis(16))? {
                match ct_event::read()? {
                    Event::Key(key)
                        if key.kind == crossterm::event::KeyEventKind::Press =>
                    {
                        let raw = Event::Key(key);
                        // Use insert-mode mapping when a text widget is focused
                        let app_event = if is_insert_mode(self.state.focus) {
                            event::to_app_event_insert(raw)
                        } else {
                            event::to_app_event(raw)
                        };
                        if let Some(ev) = app_event {
                            tracing::debug!(
                                focus = ?self.state.focus,
                                event = ?ev,
                                "key event"
                            );
                            self.handle(ev);
                        }
                    }
                    other => {
                        if let Some(ev) = event::to_app_event(other) {
                            self.handle(ev);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn handle(&mut self, event: AppEvent) {
        let s = &mut self.state;

        // Help popup intercepts all events; only close keys pass through.
        if s.show_help {
            match event {
                AppEvent::Char('?') | AppEvent::Escape | AppEvent::Quit => {
                    tracing::debug!("help popup closed");
                    s.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // Command mode intercepts all events.
        if s.focus == Focus::Command {
            match event {
                AppEvent::Escape => {
                    tracing::debug!("command bar cancelled");
                    s.command_bar.clear();
                    s.focus = s.prev_focus;
                }
                AppEvent::Enter => {
                    let input = s.command_bar.input.clone();
                    match Command::parse(&input) {
                        Ok(cmd) => {
                            tracing::debug!(command = ?cmd, "executing command");
                            s.command_bar.clear();
                            s.focus = s.prev_focus;
                            execute_command(s, cmd);
                        }
                        Err(msg) if msg.is_empty() => {
                            // Empty input — just close
                            s.command_bar.clear();
                            s.focus = s.prev_focus;
                        }
                        Err(msg) => {
                            // Show the error; bar stays open
                            s.command_bar.error = Some(msg);
                        }
                    }
                }
                other => s.command_bar.handle(&other),
            }
            return;
        }

        match event {
            // Toggle help (only when not typing in the query bar)
            AppEvent::Char('?') if s.focus != Focus::QueryBar => {
                tracing::debug!("help popup opened");
                s.show_help = true;
            }

            // Enter command mode with `:` (not from the query bar)
            AppEvent::Char(':') if s.focus != Focus::QueryBar => {
                tracing::debug!(prev_focus = ?s.focus, "entering command mode");
                s.prev_focus = s.focus;
                s.command_bar.clear();
                s.focus = Focus::Command;
            }

            // Quit / close tab
            AppEvent::Quit => {
                if s.active_tab == 0 {
                    tracing::debug!("quit");
                    s.quit = true;
                } else {
                    tracing::debug!(tab = s.active_tab, "closing tab");
                    s.tabs.remove(s.active_tab);
                    s.active_tab = s.active_tab.saturating_sub(1);
                }
            }

            // Return focus from query bar
            AppEvent::Escape => {
                if s.focus == Focus::QueryBar {
                    tracing::debug!("focus: QueryBar -> Tree");
                    s.focus = Focus::Tree;
                }
            }

            // Tab-cycle focus: Tree → Stream → QueryBar → Tree
            AppEvent::FocusNext => {
                let next = match s.focus {
                    Focus::Tree => Focus::Stream,
                    Focus::Stream => Focus::QueryBar,
                    Focus::QueryBar | Focus::Command => Focus::Tree,
                };
                tracing::debug!(from = ?s.focus, to = ?next, "focus cycle");
                s.focus = next;
            }

            // Jump to query bar
            AppEvent::QueryFocus => {
                tracing::debug!("focus -> QueryBar");
                s.focus = Focus::QueryBar;
            }

            // Greed adjustment works regardless of focus
            AppEvent::GreedUp | AppEvent::GreedDown => {
                s.tabs[s.active_tab].query.handle(&event);
            }

            // Terminal resize is handled automatically by ratatui
            AppEvent::Resize(_, _) => {}

            other => dispatch_to_focused(s, other),
        }
    }
}

/// Returns true when the current focus is on a text-input widget, meaning
/// alphabetic keys should produce characters rather than trigger shortcuts.
fn is_insert_mode(focus: Focus) -> bool {
    matches!(focus, Focus::QueryBar | Focus::Command)
}

/// Execute a parsed [`Command`] against the application state.
fn execute_command(s: &mut AppState, cmd: Command) {
    match cmd {
        Command::Quit => {
            if s.active_tab == 0 {
                s.quit = true;
            } else {
                s.tabs.remove(s.active_tab);
                s.active_tab = s.active_tab.saturating_sub(1);
            }
        }
        Command::Help => {
            s.show_help = !s.show_help;
        }
        Command::Theme(name) => {
            s.theme = match name.to_ascii_lowercase().as_str() {
                "gruvbox" | "gruvbox_dark" | "gruvbox-dark" => Theme::load_gruvbox_dark(),
                _ => Theme::load_default(),
            };
        }
        Command::Timestamps => {
            let tab = &mut s.tabs[s.active_tab];
            tab.stream.show_timestamps = !tab.stream.show_timestamps;
        }
        Command::Tail => {
            let tab = &mut s.tabs[s.active_tab];
            tab.stream.handle(&AppEvent::ScrollToTail);
        }
        Command::Greed(n) => {
            s.tabs[s.active_tab].query.greed = n;
        }
    }
}

/// Route an event to the widget that owns the current focus.
fn dispatch_to_focused(s: &mut AppState, event: AppEvent) {
    let tab = &mut s.tabs[s.active_tab];
    match s.focus {
        Focus::Tree => tab.tree.handle(&event),
        Focus::Stream => tab.stream.handle(&event),
        Focus::QueryBar => tab.query.handle(&event),
        Focus::Command => {} // handled before dispatch, should not reach here
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    // Vertical: 1-line tab bar | body | 3-line query bar
    let vert = Layout::default()
        .direction(LayoutDir::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
        ])
        .split(area);

    // Horizontal body split
    let pct = state.config.ui.producer_pane_width_pct;
    let horiz = Layout::default()
        .direction(LayoutDir::Horizontal)
        .constraints([Constraint::Percentage(pct as u16), Constraint::Fill(1)])
        .split(vert[1]);

    let tab = &state.tabs[state.active_tab];

    frame.render_widget(TabBar::new(&state.tabs, state.active_tab, &state.theme), vert[0]);
    frame.render_widget(
        ProducerTree::new(&tab.tree, state.focus == Focus::Tree, &state.theme),
        horiz[0],
    );
    frame.render_widget(
        LogStream::new(&tab.stream, state.focus == Focus::Stream, &state.theme),
        horiz[1],
    );
    frame.render_widget(
        QueryBar::new(&tab.query, state.focus == Focus::QueryBar, &state.theme),
        vert[2],
    );

    if state.show_help {
        frame.render_widget(HelpPopup::new(&state.theme), area);
    }

    // Command bar overlays the bottom row of the screen
    if state.focus == Focus::Command {
        let cmd_area = Rect { y: area.bottom() - 1, height: 1, ..area };
        frame.render_widget(CommandBar::new(&state.command_bar, &state.theme), cmd_area);
        let col = state.command_bar.cursor_col(cmd_area);
        frame.set_cursor_position((col, cmd_area.y));
        return; // cursor is set; skip query-bar cursor below
    }

    // Position the terminal cursor when the query bar is focused
    if state.focus == Focus::QueryBar {
        let qb = QueryBar::new(&tab.query, true, &state.theme);
        let (cx, cy) = qb.cursor_position(vert[2]);
        frame.set_cursor_position((cx, cy));
    }
}

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}
