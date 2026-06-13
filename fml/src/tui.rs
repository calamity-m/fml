//! Modal TUI: terminal lifecycle, the input reader task, and the key →
//! workspace reducer.
//!
//! Input follows a vim-like grammar (see `docs/MODAL_REDESIGN.md`): a global
//! [`Mode`] plus per-pane follow state, counts and multi-key prefixes, and a
//! single-line prompt for `/` and `:`. Mouse capture is intentionally never
//! enabled so the terminal's native selection/copy keeps working.

use std::io::stdout;

use crossterm::{
    ExecutableCommand as _,
    event::{Event as CrosstermEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use futures_util::{FutureExt as _, StreamExt as _};
use ratatui::{Terminal, backend::Backend, layout::Direction};
use tokio::{sync::mpsc, time::interval};
use tracing::{debug, error, warn};

pub mod pane;
pub mod render;
pub mod workspace;

use crate::{
    clipboard::OSC52_WARN_BYTES,
    config::tui::TuiConfig,
    error::FmlError,
    event::{Query, QuitEvent, TuiEvent},
    state::AppState,
    tui::{
        pane::{Pane, SearchCtx, View},
        workspace::{Mode, Prefix, Workspace},
    },
};

/// Start the TUI input reader task and install panic hooks that restore the
/// terminal before unwinding.
pub fn spawn(config: &TuiConfig, event_tx: mpsc::UnboundedSender<TuiEvent>) {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    tui_loop(config, event_tx);
}

/// Restore the terminal.
pub fn kill() -> Result<(), FmlError> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Spawn the task that forwards crossterm events and render ticks.
fn tui_loop(config: &TuiConfig, event_tx: mpsc::UnboundedSender<TuiEvent>) {
    let frame_rate = config.frame_rate;

    tokio::spawn(async move {
        debug!(frame_rate, "tui event reader task started");
        let mut reader = EventStream::new();
        let mut render_interval = interval(std::time::Duration::from_secs_f64(1.0 / frame_rate));

        loop {
            let event = tokio::select! {
                _ = render_interval.tick() => TuiEvent::Render,
                crossterm_event = reader.next().fuse() => match crossterm_event {
                    Some(Ok(event)) => match event {
                        CrosstermEvent::Key(key) => {
                            // Only act on presses. Terminals that report
                            // release/repeat events (kitty protocol) must not
                            // tear down the reader task here.
                            if key.kind != KeyEventKind::Press {
                                continue;
                            }
                            TuiEvent::Input(key)
                        },
                        CrosstermEvent::Mouse(mouse) => TuiEvent::Mouse(mouse),
                        CrosstermEvent::Resize(x, y) => TuiEvent::Resize(x, y),
                        CrosstermEvent::FocusLost => TuiEvent::FocusLost,
                        CrosstermEvent::FocusGained => TuiEvent::FocusGained,
                        CrosstermEvent::Paste(s) => TuiEvent::Paste(s),
                    }
                    Some(Err(err)) => {
                        warn!(error = %err, "tui event reader received crossterm error");
                        TuiEvent::Error(err.to_string())
                    }
                    None => {
                        debug!("tui event reader stream ended");
                        break;
                    }
                },
            };

            if let Err(err) = event_tx.send(event) {
                error!("error sending event from tui: {:?}", err);
                break;
            }
        }

        debug!("tui event reader task exited");
    });
}

/// Render the current state into `terminal`. Generic over the backend so
/// production runs against a `CrosstermBackend` and tests render into a
/// `TestBackend`.
pub fn render<B: Backend>(state: &mut AppState, terminal: &mut Terminal<B>) {
    let result = terminal.draw(|frame| render::draw(state, frame));
    if let Err(err) = result
        && let Err(err) = state
            .event_bus
            .tui_event_tx
            .send(TuiEvent::Error(err.to_string()))
    {
        error!("failed to send tui_event error after failed render - {err}");
    }
}

/// Apply a single [`TuiEvent`] to the application state.
///
/// Render events are a no-op here — rendering is an output side-effect that
/// the app loop performs directly via [`render`].
pub fn handle_tui_event(event: TuiEvent, mut state: AppState) -> AppState {
    match event {
        TuiEvent::Input(key) => handle_key(&mut state, key),
        TuiEvent::Paste(text) => handle_paste(&mut state, &text),
        TuiEvent::Error(err) => error!("received error event - {err}"),
        TuiEvent::Render
        | TuiEvent::Mouse(_)
        | TuiEvent::Resize(_, _)
        | TuiEvent::FocusGained
        | TuiEvent::FocusLost => {}
    }
    state
}

/// Dispatch the initial searches for every pane (called once at startup).
pub fn dispatch_startup(state: &mut AppState) {
    let (ws, ctx) = split_state(state);
    for tab in &mut ws.tabs {
        for pane in &mut tab.panes {
            dispatch_pane_current(pane, &ctx);
        }
    }
}

/// Re-dispatch panes whose filter resolution depends on the live source
/// list. Called when sources appear or disappear so worker source-id
/// snapshots stay correct.
pub fn redispatch_filtered_panes(state: &mut AppState) {
    let (ws, ctx) = split_state(state);
    for tab in &mut ws.tabs {
        for pane in &mut tab.panes {
            if !pane.filter.is_empty() {
                dispatch_pane_current(pane, &ctx);
            }
        }
    }
}

/// Borrow the workspace mutably alongside a search context built from the
/// other (disjoint) `AppState` fields.
fn split_state(state: &mut AppState) -> (&mut Workspace, SearchCtx<'_>) {
    let bounds = state.store.bounds();
    (
        &mut state.workspace,
        SearchCtx {
            sources: &state.producer.sources,
            tx: &state.event_bus.search_event_tx,
            buffer: state.config.search.tail_size as u64,
            bounds,
        },
    )
}

/// Re-dispatch whatever the pane is currently showing (stream or search).
fn dispatch_pane_current(pane: &mut Pane, ctx: &SearchCtx) {
    match &pane.view {
        View::Results { term, .. } if !term.is_empty() => {
            // Re-rank live while following, frozen at the current high otherwise.
            let until_seq = (!pane.follow).then_some(ctx.bounds.1);
            pane.dispatch(
                Query::Fuzzy {
                    term: term.clone(),
                    until_seq,
                },
                ctx,
            )
        }
        _ => pane.dispatch_stream(ctx),
    }
}

fn quit(state: &mut AppState) {
    if let Err(err) = state.event_bus.quit_tx.try_send(QuitEvent {}) {
        // A second quit while one is queued is harmless.
        warn!("failed to send quit event - {err}");
    }
}

fn handle_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        quit(state);
        return;
    }

    state.workspace.notice = None;

    if state.workspace.help_open {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
        ) {
            state.workspace.help_open = false;
        }
        return;
    }

    if state.workspace.picker.is_some() {
        handle_picker_key(state, key);
        return;
    }

    if state.workspace.focused_pane().detail_open {
        handle_detail_key(state, key);
        return;
    }

    match state.workspace.mode {
        Mode::Normal => handle_normal_key(state, key),
        Mode::Visual {
            anchor_seq,
            anchor_col,
            linewise,
        } => handle_visual_key(state, key, anchor_seq, anchor_col, linewise),
        Mode::Search => handle_search_key(state, key),
        Mode::Command => handle_command_key(state, key),
    }
}

fn handle_detail_key(state: &mut AppState, key: KeyEvent) {
    let pane = state.workspace.focused_pane_mut();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            pane.detail_scroll = pane.detail_scroll.saturating_add(1)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            pane.detail_scroll = pane.detail_scroll.saturating_sub(1)
        }
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            pane.detail_open = false;
            pane.detail_scroll = 0;
        }
        _ => {}
    }
}

/// Open the source picker, pre-selecting the sources the focused pane's
/// current filter resolves to.
fn open_source_picker(state: &mut AppState) {
    let mut picker = workspace::SourcePicker::default();
    let pane = state.workspace.focused_pane();
    if !pane.filter.is_empty()
        && let Some(ids) = pane.resolve_filter(&state.producer.sources)
    {
        picker.selected = ids.into_iter().collect();
    }
    state.workspace.picker = Some(picker);
}

/// Apply the picker: write the chosen sources as exact `=name` filter
/// patterns on the focused pane and re-dispatch it.
fn apply_source_picker(state: &mut AppState) {
    let Some(picker) = state.workspace.picker.take() else {
        return;
    };
    let rows = picker.rows(&state.producer.sources);
    // Toggled sources win; with nothing toggled, take the highlighted row.
    let chosen: Vec<&crate::log::Source> = if picker.selected.is_empty() {
        rows.get(picker.cursor.min(rows.len().saturating_sub(1)))
            .copied()
            .into_iter()
            .collect()
    } else {
        state
            .producer
            .sources
            .iter()
            .filter(|source| picker.selected.contains(&source.id))
            .collect()
    };
    if chosen.is_empty() {
        return;
    }
    let mut patterns: Vec<String> = chosen
        .iter()
        .map(|source| format!("={}", source.display_name))
        .collect();
    patterns.dedup();
    let count = chosen.len();
    {
        let (ws, ctx) = split_state(state);
        let pane = ws.focused_pane_mut();
        pane.filter = patterns;
        dispatch_pane_current(pane, &ctx);
    }
    state.workspace.notice = Some(format!("filter → {count} sources"));
}

fn handle_picker_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Snapshot the narrowed row ids so the picker can be borrowed mutably.
    let row_ids: Vec<crate::log::SourceId> = {
        let Some(picker) = state.workspace.picker.as_ref() else {
            return;
        };
        picker
            .rows(&state.producer.sources)
            .iter()
            .map(|source| source.id.clone())
            .collect()
    };

    match (key.code, ctrl) {
        (KeyCode::Esc, _) => state.workspace.picker = None,
        (KeyCode::Enter, _) => apply_source_picker(state),
        _ => {
            let Some(picker) = state.workspace.picker.as_mut() else {
                return;
            };
            picker.cursor = picker.cursor.min(row_ids.len().saturating_sub(1));
            match (key.code, ctrl) {
                // fzf idiom: Tab toggles and advances, Shift-Tab backs up.
                (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                    if let Some(id) = row_ids.get(picker.cursor)
                        && !picker.selected.remove(id)
                    {
                        picker.selected.insert(id.clone());
                    }
                    if key.code == KeyCode::Tab {
                        picker.cursor = (picker.cursor + 1).min(row_ids.len().saturating_sub(1));
                    } else {
                        picker.cursor = picker.cursor.saturating_sub(1);
                    }
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), true) | (KeyCode::Char('p'), true) => {
                    picker.cursor = picker.cursor.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), true) | (KeyCode::Char('n'), true) => {
                    picker.cursor = (picker.cursor + 1).min(row_ids.len().saturating_sub(1));
                }
                // Toggle every narrowed row at once.
                (KeyCode::Char('a'), true) => {
                    if row_ids.iter().all(|id| picker.selected.contains(id)) {
                        for id in &row_ids {
                            picker.selected.remove(id);
                        }
                    } else {
                        picker.selected.extend(row_ids.iter().cloned());
                    }
                }
                (KeyCode::Char('u'), true) => {
                    picker.query.reset();
                    picker.cursor = 0;
                }
                (KeyCode::Backspace, _) => {
                    picker.query.backspace();
                    picker.cursor = 0;
                }
                (KeyCode::Char(c), false) => {
                    picker.query.insert(c);
                    picker.cursor = 0;
                }
                _ => {}
            }
        }
    }
}

/// Shared cursor motions for NORMAL and VISUAL. Returns true when consumed.
fn handle_motion_key(state: &mut AppState, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Count accumulation: `12j`-style prefixes.
    if let KeyCode::Char(c) = key.code
        && !ctrl
        && c.is_ascii_digit()
        && (c != '0' || state.workspace.pending.count.is_some())
    {
        let count = state.workspace.pending.count.unwrap_or(0);
        state.workspace.pending.count = Some(
            count
                .saturating_mul(10)
                .saturating_add(c as u32 - '0' as u32)
                .min(99_999),
        );
        return true;
    }

    // `g` prefix: gg / gt / gT.
    if state.workspace.pending.prefix == Some(Prefix::G) {
        state.workspace.pending.clear();
        match key.code {
            KeyCode::Char('g') => {
                let (ws, ctx) = split_state(state);
                ws.focused_pane_mut().goto_top(&ctx);
            }
            KeyCode::Char('t') => {
                state.workspace.mode = Mode::Normal;
                state.workspace.next_tab();
            }
            KeyCode::Char('T') => {
                state.workspace.mode = Mode::Normal;
                state.workspace.prev_tab();
            }
            _ => {}
        }
        return true;
    }

    let (ws, ctx) = split_state(state);
    let count = ws.pending.take_count() as i64;
    let pane = ws.focused_pane_mut();
    let half = (pane.page_rows() / 2).max(1) as i64;
    let page = pane.page_rows() as i64;

    match (key.code, ctrl) {
        (KeyCode::Char('j'), false) | (KeyCode::Down, _) => pane.move_cursor(count, &ctx),
        (KeyCode::Char('k'), false) | (KeyCode::Up, _) => pane.move_cursor(-count, &ctx),
        (KeyCode::Char('h'), false) | (KeyCode::Left, _) => pane.move_col(-count, &ctx),
        (KeyCode::Char('l'), false) | (KeyCode::Right, _) => pane.move_col(count, &ctx),
        (KeyCode::Char('0'), false) | (KeyCode::Home, _) => pane.col_home(&ctx),
        (KeyCode::Char('$'), false) | (KeyCode::End, _) => pane.col_end(&ctx),
        (KeyCode::Char('w'), false) => pane.word_forward(&ctx),
        (KeyCode::Char('b'), false) => pane.word_back(&ctx),
        (KeyCode::Char('d'), true) => pane.move_cursor(half, &ctx),
        (KeyCode::Char('u'), true) => pane.move_cursor(-half, &ctx),
        (KeyCode::Char('f'), true) | (KeyCode::PageDown, _) => pane.move_cursor(page, &ctx),
        (KeyCode::Char('b'), true) | (KeyCode::PageUp, _) => pane.move_cursor(-page, &ctx),
        (KeyCode::Char('g'), false) => ws.pending.prefix = Some(Prefix::G),
        (KeyCode::Char('G'), false) => pane.goto_bottom(&ctx),
        (KeyCode::Char('n'), false) => {
            if !pane.jump_hit(true, &ctx) {
                ws.notice = Some("no further hits".to_string());
            }
        }
        (KeyCode::Char('N'), false) => {
            if !pane.jump_hit(false, &ctx) {
                ws.notice = Some("no previous hits".to_string());
            }
        }
        _ => return false,
    }
    true
}

fn handle_normal_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if state.workspace.pending.prefix == Some(Prefix::Window) {
        state.workspace.pending.clear();
        handle_window_key(state, key);
        return;
    }

    if handle_motion_key(state, key) {
        return;
    }

    match (key.code, ctrl) {
        (KeyCode::Char('w'), true) => state.workspace.pending.prefix = Some(Prefix::Window),
        (KeyCode::Char('F'), false) => {
            let (ws, ctx) = split_state(state);
            ws.focused_pane_mut().enter_follow(&ctx);
        }
        (KeyCode::Char('/'), false) => {
            state.workspace.mode = Mode::Search;
            state.workspace.prompt.reset();
            let (ws, ctx) = split_state(state);
            let pane = ws.focused_pane_mut();
            pane.begin_search();
            pane.update_search("", &ctx);
        }
        (KeyCode::Char(':'), false) => {
            state.workspace.mode = Mode::Command;
            state.workspace.prompt.reset();
        }
        (KeyCode::Char(c @ ('v' | 'V')), false) => {
            let pane = state.workspace.focused_pane();
            if let Some(anchor_seq) = pane.cursor_seq {
                state.workspace.mode = Mode::Visual {
                    anchor_seq,
                    anchor_col: pane.effective_col(),
                    linewise: c == 'V',
                };
            }
        }
        (KeyCode::Char('y'), false) => yank_cursor_entry(state),
        (KeyCode::Char('?'), false) => state.workspace.help_open = true,
        (KeyCode::Enter, _) => {
            let (ws, ctx) = split_state(state);
            let pane = ws.focused_pane_mut();
            match &pane.view {
                View::Results { .. } => pane.results_to_stream(&ctx),
                View::Stream { .. } => {
                    if pane.cursor_entry().is_some() {
                        pane.detail_open = true;
                        pane.detail_scroll = 0;
                    }
                }
            }
        }
        (KeyCode::Esc, _) => {
            if !state.workspace.pending.is_empty() {
                state.workspace.pending.clear();
                return;
            }
            let (ws, ctx) = split_state(state);
            let pane = ws.focused_pane_mut();
            match &pane.view {
                View::Results { .. } => pane.results_to_stream(&ctx),
                View::Stream { .. } => pane.clear_search(),
            }
        }
        _ => {}
    }
}

fn handle_window_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('v') => split_pane(state, Direction::Horizontal),
        KeyCode::Char('s') => split_pane(state, Direction::Vertical),
        KeyCode::Char('h') | KeyCode::Left => state.workspace.tab_mut().focus_direction(-1, 0),
        KeyCode::Char('l') | KeyCode::Right => state.workspace.tab_mut().focus_direction(1, 0),
        KeyCode::Char('j') | KeyCode::Down => state.workspace.tab_mut().focus_direction(0, 1),
        KeyCode::Char('k') | KeyCode::Up => state.workspace.tab_mut().focus_direction(0, -1),
        KeyCode::Char('w') => {
            // Cycle focus through the tab's panes in creation order.
            let tab = state.workspace.tab_mut();
            if let Some(pos) = tab.panes.iter().position(|pane| pane.id == tab.focused) {
                tab.focused = tab.panes[(pos + 1) % tab.panes.len()].id;
            }
        }
        KeyCode::Char('q') => close_focused_pane(state),
        KeyCode::Char('o') => {
            let closed = state.workspace.only_focused_pane();
            for id in closed {
                state.search.cancel(id);
            }
        }
        _ => {}
    }
}

fn split_pane(state: &mut AppState, dir: Direction) {
    state.workspace.split(dir);
    let (ws, ctx) = split_state(state);
    dispatch_pane_current(ws.focused_pane_mut(), &ctx);
}

fn close_focused_pane(state: &mut AppState) {
    let (closed, empty) = state.workspace.close_focused_pane();
    for id in closed {
        state.search.cancel(id);
    }
    if empty {
        quit(state);
    }
}

fn handle_visual_key(
    state: &mut AppState,
    key: KeyEvent,
    anchor_seq: u64,
    anchor_col: usize,
    linewise: bool,
) {
    if handle_motion_key(state, key) {
        return;
    }
    match key.code {
        KeyCode::Char('y') => {
            yank_selection(state, anchor_seq, anchor_col, linewise);
            state.workspace.mode = Mode::Normal;
        }
        // vim semantics: pressing the current kind's key exits; pressing
        // the other kind's key switches without losing the anchor.
        KeyCode::Char('v') => {
            state.workspace.mode = if linewise {
                Mode::Visual {
                    anchor_seq,
                    anchor_col,
                    linewise: false,
                }
            } else {
                Mode::Normal
            };
        }
        KeyCode::Char('V') => {
            state.workspace.mode = if linewise {
                Mode::Normal
            } else {
                Mode::Visual {
                    anchor_seq,
                    anchor_col,
                    linewise: true,
                }
            };
        }
        KeyCode::Esc => state.workspace.mode = Mode::Normal,
        _ => {}
    }
}

fn handle_search_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        (KeyCode::Esc, _) => {
            state.workspace.mode = Mode::Normal;
            state.workspace.prompt.reset();
            let (ws, ctx) = split_state(state);
            ws.focused_pane_mut().abandon_search(&ctx);
        }
        (KeyCode::Enter, _) => {
            state.workspace.mode = Mode::Normal;
            let hits = state.workspace.focused_pane_mut().confirm_search();
            state.workspace.notice = Some(if hits == 0 {
                "no matches".to_string()
            } else {
                format!("{hits} matches — Enter opens in context, n/N jumps")
            });
            state.workspace.prompt.reset();
        }
        (KeyCode::Char('u'), true) => {
            state.workspace.prompt.reset();
            live_search(state);
        }
        (KeyCode::Char(c), false) => {
            state.workspace.prompt.insert(c);
            live_search(state);
        }
        (KeyCode::Backspace, _) => {
            state.workspace.prompt.backspace();
            live_search(state);
        }
        (KeyCode::Left, _) => state.workspace.prompt.left(),
        (KeyCode::Right, _) => state.workspace.prompt.right(),
        (KeyCode::Home, _) => state.workspace.prompt.home(),
        (KeyCode::End, _) => state.workspace.prompt.end(),
        _ => {}
    }
}

fn live_search(state: &mut AppState) {
    let term = state.workspace.prompt.buf.clone();
    let (ws, ctx) = split_state(state);
    ws.focused_pane_mut().update_search(&term, &ctx);
}

fn handle_command_key(state: &mut AppState, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Any key other than Tab/BackTab abandons an in-flight completion.
    if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        state.workspace.completion = None;
    }
    match (key.code, ctrl) {
        (KeyCode::Esc, _) => {
            state.workspace.mode = Mode::Normal;
            state.workspace.prompt.reset();
        }
        (KeyCode::Enter, _) => {
            let line = state.workspace.prompt.buf.clone();
            state.workspace.mode = Mode::Normal;
            state.workspace.prompt.reset();
            execute_command(state, &line);
        }
        (KeyCode::Tab, _) => cycle_completion(state, true),
        (KeyCode::BackTab, _) => cycle_completion(state, false),
        (KeyCode::Char('u'), true) => state.workspace.prompt.reset(),
        (KeyCode::Char(c), false) => state.workspace.prompt.insert(c),
        (KeyCode::Backspace, _) => state.workspace.prompt.backspace(),
        (KeyCode::Left, _) => state.workspace.prompt.left(),
        (KeyCode::Right, _) => state.workspace.prompt.right(),
        (KeyCode::Home, _) => state.workspace.prompt.home(),
        (KeyCode::End, _) => state.workspace.prompt.end(),
        _ => {}
    }
}

/// Command names offered by first-token completion.
const COMMAND_NAMES: &[&str] = &[
    "filter", "sources", "vsplit", "split", "hsplit", "tabnew", "tabclose", "tabnext", "tabprev",
    "tail", "refresh", "clear", "only", "help", "quit", "qa",
];

/// Vim-style `:` completion. The first Tab gathers candidates for the
/// trailing token (command names, or live source names for `:filter`) and
/// applies the first; further Tabs cycle.
fn cycle_completion(state: &mut AppState, forward: bool) {
    if state.workspace.completion.is_none() {
        let buf = state.workspace.prompt.buf.clone();
        // Complete the token after the last separator; ',' supports
        // `:filter a,b<Tab>` and ' ' separates command from args.
        let token_start = buf.rfind([' ', ',']).map(|idx| idx + 1).unwrap_or(0);
        let token = &buf[token_start..];
        let completing_command = token_start == 0;

        let candidates: Vec<String> = if completing_command {
            COMMAND_NAMES
                .iter()
                .filter(|name| name.starts_with(token))
                .map(|name| name.to_string())
                .collect()
        } else if matches!(buf.split_whitespace().next(), Some("filter") | Some("f")) {
            // `=`-prefixed tokens complete to exact-match patterns.
            let (prefix, needle) = match token.strip_prefix('=') {
                Some(rest) => ("=", rest),
                None => ("", token),
            };
            let needle = needle.to_lowercase();
            let mut names: Vec<String> = state
                .producer
                .sources
                .iter()
                .flat_map(|source| {
                    [
                        Some(source.display_name.clone()),
                        source.group.clone(),
                        Some(source.producer.clone()),
                    ]
                })
                .flatten()
                .filter(|name| name.to_lowercase().starts_with(&needle))
                .map(|name| format!("{prefix}{name}"))
                .collect();
            names.sort();
            names.dedup();
            names
        } else {
            Vec::new()
        };

        if candidates.is_empty() {
            return;
        }
        state.workspace.completion = Some(workspace::Completion {
            candidates,
            index: 0,
            token_start,
        });
    } else if let Some(completion) = state.workspace.completion.as_mut() {
        let len = completion.candidates.len();
        completion.index = if forward {
            (completion.index + 1) % len
        } else {
            (completion.index + len - 1) % len
        };
    }

    let Some(completion) = state.workspace.completion.as_ref() else {
        return;
    };
    let candidate = completion.candidates[completion.index].clone();
    let prompt = &mut state.workspace.prompt;
    prompt.buf.truncate(completion.token_start);
    prompt.buf.push_str(&candidate);
    prompt.end();
}

fn execute_command(state: &mut AppState, line: &str) {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();

    match cmd {
        "" => {}
        "q" | "quit" => close_focused_pane(state),
        "qa" | "qall" | "quitall" => quit(state),
        "vs" | "vsplit" => split_pane(state, Direction::Horizontal),
        "sp" | "split" | "hs" | "hsplit" => split_pane(state, Direction::Vertical),
        "only" | "on" => {
            let closed = state.workspace.only_focused_pane();
            for id in closed {
                state.search.cancel(id);
            }
        }
        "tabnew" => {
            state
                .workspace
                .new_tab((!args.is_empty()).then(|| args.join(" ")));
            let (ws, ctx) = split_state(state);
            dispatch_pane_current(ws.focused_pane_mut(), &ctx);
        }
        "tabclose" | "tabc" => {
            let (closed, empty) = state.workspace.close_tab();
            for id in closed {
                state.search.cancel(id);
            }
            if empty {
                quit(state);
            }
        }
        "tabn" | "tabnext" => state.workspace.next_tab(),
        "tabp" | "tabprev" => state.workspace.prev_tab(),
        "filter" | "f" => {
            let patterns: Vec<String> = args
                .join(" ")
                .split(',')
                .map(str::trim)
                .filter(|pat| !pat.is_empty())
                .map(str::to_string)
                .collect();
            let (ws, ctx) = split_state(state);
            let pane = ws.focused_pane_mut();
            pane.filter = patterns;
            dispatch_pane_current(pane, &ctx);
        }
        "tail" => {
            let (ws, ctx) = split_state(state);
            ws.focused_pane_mut().enter_follow(&ctx);
        }
        "refresh" => {
            let (ws, ctx) = split_state(state);
            if !ws.focused_pane_mut().refresh_search(&ctx) {
                ws.notice = Some("nothing to refresh".to_string());
            }
        }
        "clear" | "noh" => {
            let (ws, ctx) = split_state(state);
            let pane = ws.focused_pane_mut();
            pane.clear_search();
            if matches!(pane.view, View::Results { .. }) {
                pane.results_to_stream(&ctx);
            }
        }
        "sources" | "src" | "ls" => open_source_picker(state),
        "help" | "h" => state.workspace.help_open = true,
        unknown => {
            state.workspace.notice = Some(format!("not a command: {unknown}"));
        }
    }
}

fn handle_paste(state: &mut AppState, text: &str) {
    match state.workspace.mode {
        Mode::Search => {
            state.workspace.prompt.insert_str(text);
            live_search(state);
        }
        Mode::Command => state.workspace.prompt.insert_str(text),
        _ => {}
    }
}

fn yank_cursor_entry(state: &mut AppState) {
    let Some(entry) = state.workspace.focused_pane().cursor_entry() else {
        return;
    };
    let payload = serde_json::to_string(&**entry).unwrap_or_default();
    yank(state, &payload, "entry");
}

fn yank_selection(state: &mut AppState, anchor_seq: u64, anchor_col: usize, linewise: bool) {
    let pane = state.workspace.focused_pane();
    let text = if linewise {
        pane.linewise_selection_text(anchor_seq)
    } else {
        pane.charwise_selection_text(anchor_seq, anchor_col)
    };
    if text.is_empty() {
        return;
    }
    let what = if linewise {
        format!("{} lines", text.lines().count())
    } else {
        format!("{} chars", text.chars().count())
    };
    yank(state, &text, &what);
}

fn yank(state: &mut AppState, payload: &str, what: &str) {
    let mut out = stdout().lock();
    let msg = match crate::clipboard::yank_osc52(&mut out, payload) {
        Ok(n) if n > OSC52_WARN_BYTES => {
            format!("yanked {what} ({n} bytes) — exceeds 8KB; xterm/vte may drop it")
        }
        Ok(n) => format!("yanked {what} ({n} bytes)"),
        Err(err) => format!("yank failed: {err}"),
    };
    drop(out);
    state.workspace.notice = Some(msg);

    // Append rather than replace so the byte-count outcome stays visible.
    if let Some(hint) = multiplexer_hint()
        && let Some(notice) = &mut state.workspace.notice
    {
        notice.push_str(" · ");
        notice.push_str(&hint);
    }
}

/// Returns a hint string when running inside a known multiplexer whose
/// clipboard forwarding usually needs configuration.
fn multiplexer_hint() -> Option<String> {
    let in_tmux = std::env::var("TMUX").is_ok();
    let in_zellij = std::env::var("ZELLIJ").is_ok();
    match (in_tmux, in_zellij) {
        (true, _) => Some("tmux: `set -g set-clipboard on` enables OSC52".into()),
        (false, true) => Some("zellij: OSC52 needs a capable host terminal".into()),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::*;
    use crate::{
        config::Config,
        event::{PaneId, ProducerEvent, SearchEvent},
        log::{LogLevel, NewLogEntry, Source},
        producer,
    };

    fn input(code: KeyCode) -> TuiEvent {
        TuiEvent::Input(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_input(code: KeyCode) -> TuiEvent {
        TuiEvent::Input(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    fn state_with_entries(count: u64) -> AppState {
        let mut state = AppState::new(Config::default()).expect("app state");
        state = producer::handle_producer_event(
            ProducerEvent::SourceFound(Source {
                producer: "fake".to_string(),
                id: "src-a".to_string(),
                display_name: "Source A".to_string(),
                group: None,
            }),
            state,
        );
        for seq in 1..=count {
            state = producer::handle_producer_event(
                ProducerEvent::StoreEvent(NewLogEntry {
                    msg: format!("entry {seq}"),
                    ts: Utc::now(),
                    level: Some(LogLevel::Info),
                    source: Source {
                        producer: "fake".to_string(),
                        id: "src-a".to_string(),
                        display_name: "Source A".to_string(),
                        group: None,
                    },
                    fields: HashMap::new(),
                }),
                state,
            );
        }
        state
    }

    /// Feed a tail result into the focused pane so motions have entries.
    fn seed_tail(state: &mut AppState, seqs: &[u64]) {
        let mut entries = Vec::new();
        state.store.fetch_requested(seqs, &mut entries).unwrap();
        let bounds = state.store.bounds();
        let pane = state.workspace.focused_pane_mut();
        pane.active_query = Some(Query::Tail);
        pane.apply_result(
            &Query::Tail,
            entries,
            HashMap::new(),
            None,
            None,
            true,
            bounds,
        );
    }

    fn drain_search_events(state: &mut AppState) -> Vec<SearchEvent> {
        let mut events = Vec::new();
        while let Ok(event) = state.event_bus.search_event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn ctrl_c_sends_quit_from_any_mode() {
        let mut state = state_with_entries(3);
        state.workspace.mode = Mode::Command;
        let mut state = handle_tui_event(ctrl_input(KeyCode::Char('c')), state);
        assert!(state.event_bus.quit_rx.try_recv().is_ok());
    }

    #[test]
    fn motion_breaks_follow_into_history_anchor() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);
        assert!(state.workspace.focused_pane().follow);

        let mut state = handle_tui_event(input(KeyCode::Char('k')), state);

        let pane = state.workspace.focused_pane();
        assert!(!pane.follow);
        assert_eq!(pane.cursor_seq, Some(4));
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search {
                query: Query::History {
                    middle_seq_id: 4,
                    ..
                },
                ..
            }
        )));
    }

    #[test]
    fn count_prefix_multiplies_motion() {
        let mut state = state_with_entries(20);
        seed_tail(&mut state, &(1..=20).collect::<Vec<_>>());

        let state = handle_tui_event(input(KeyCode::Char('1')), state);
        let state = handle_tui_event(input(KeyCode::Char('2')), state);
        let state = handle_tui_event(input(KeyCode::Char('k')), state);

        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(8));
    }

    #[test]
    fn gg_jumps_to_oldest_and_g_to_newest() {
        let mut state = state_with_entries(10);
        seed_tail(&mut state, &(1..=10).collect::<Vec<_>>());

        let state = handle_tui_event(input(KeyCode::Char('g')), state);
        let mut state = handle_tui_event(input(KeyCode::Char('g')), state);

        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(1));
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search {
                query: Query::History {
                    middle_seq_id: 1,
                    ..
                },
                ..
            }
        )));

        let state = handle_tui_event(input(KeyCode::Char('G')), state);
        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(10));
    }

    #[test]
    fn refresh_command_notices_when_following() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);
        assert!(state.workspace.focused_pane().follow);

        execute_command(&mut state, "refresh");

        assert_eq!(
            state.workspace.notice.as_deref(),
            Some("nothing to refresh"),
            "refresh on a live pane is a no-op notice"
        );
    }

    #[test]
    fn capital_f_reenters_tail() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);
        let state = handle_tui_event(input(KeyCode::Char('k')), state);
        assert!(!state.workspace.focused_pane().follow);

        let mut state = handle_tui_event(input(KeyCode::Char('F')), state);

        assert!(state.workspace.focused_pane().follow);
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search {
                query: Query::Tail,
                ..
            }
        )));
    }

    #[test]
    fn slash_enters_search_and_typing_dispatches_fuzzy() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);

        let state = handle_tui_event(input(KeyCode::Char('/')), state);
        assert_eq!(state.workspace.mode, Mode::Search);

        let state = handle_tui_event(input(KeyCode::Char('e')), state);
        let mut state = handle_tui_event(input(KeyCode::Char('r')), state);

        let events = drain_search_events(&mut state);
        let terms: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                SearchEvent::Search {
                    query: Query::Fuzzy { term, .. },
                    ..
                } => Some(term.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(terms, vec!["e".to_string(), "er".to_string()]);
    }

    #[test]
    fn search_enter_confirms_hits_and_esc_restores_stream() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);

        // Type a search and inject a fuzzy result as the engine would.
        let mut state = handle_tui_event(input(KeyCode::Char('/')), state);
        state.workspace.prompt.insert('e');
        let term = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: None,
        };
        let mut entries = Vec::new();
        state.store.fetch_requested(&[2, 4], &mut entries).unwrap();
        {
            let pane = state.workspace.focused_pane_mut();
            pane.active_query = Some(term.clone());
            pane.apply_result(&term, entries, HashMap::new(), None, None, true, (1, 5));
        }

        let state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.workspace.mode, Mode::Normal);
        let pane = state.workspace.focused_pane();
        assert_eq!(pane.hits, vec![2, 4]);
        assert!(matches!(pane.view, View::Results { .. }));

        // Esc drops back to a stream centered on the cursor.
        let mut state = handle_tui_event(input(KeyCode::Esc), state);
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search {
                query: Query::History { .. },
                ..
            }
        )));
    }

    #[test]
    fn ctrl_w_v_splits_and_dispatches_clone() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);

        let state = handle_tui_event(ctrl_input(KeyCode::Char('w')), state);
        let mut state = handle_tui_event(input(KeyCode::Char('v')), state);

        assert_eq!(state.workspace.tab().panes.len(), 2);
        let new_id = state.workspace.tab().focused;
        assert_eq!(new_id, PaneId(2));
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search { target, .. } if *target == new_id
        )));
    }

    #[test]
    fn ctrl_w_q_on_last_pane_quits() {
        let state = state_with_entries(1);
        let state = handle_tui_event(ctrl_input(KeyCode::Char('w')), state);
        let mut state = handle_tui_event(input(KeyCode::Char('q')), state);
        assert!(state.event_bus.quit_rx.try_recv().is_ok());
    }

    #[test]
    fn visual_selection_tracks_anchor_and_yields_to_normal_on_esc() {
        let mut state = state_with_entries(5);
        seed_tail(&mut state, &[1, 2, 3, 4, 5]);

        let state = handle_tui_event(input(KeyCode::Char('v')), state);
        let charwise = Mode::Visual {
            anchor_seq: 5,
            anchor_col: 0,
            linewise: false,
        };
        assert_eq!(state.workspace.mode, charwise);

        let state = handle_tui_event(input(KeyCode::Char('k')), state);
        let state = handle_tui_event(input(KeyCode::Char('k')), state);
        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(3));
        assert_eq!(state.workspace.mode, charwise);

        // `V` switches kind in place; `V` again exits.
        let state = handle_tui_event(input(KeyCode::Char('V')), state);
        assert_eq!(
            state.workspace.mode,
            Mode::Visual {
                anchor_seq: 5,
                anchor_col: 0,
                linewise: true,
            }
        );
        let state = handle_tui_event(input(KeyCode::Char('V')), state);
        assert_eq!(state.workspace.mode, Mode::Normal);

        let state = handle_tui_event(input(KeyCode::Char('v')), state);
        let state = handle_tui_event(input(KeyCode::Esc), state);
        assert_eq!(state.workspace.mode, Mode::Normal);
    }

    #[test]
    fn charwise_motions_move_columns_and_yank_extracts_chars() {
        let mut state = state_with_entries(3);
        seed_tail(&mut state, &[1, 2, 3]);

        // `w` from column 0 lands on the level field, `l` steps right.
        let state = handle_tui_event(input(KeyCode::Char('w')), state);
        let state = handle_tui_event(input(KeyCode::Char('l')), state);
        let pane = state.workspace.focused_pane();
        assert!(!pane.follow, "column motion breaks follow");
        assert_eq!(pane.effective_col(), 10);

        // `$` then `0` jump to line ends.
        let state = handle_tui_event(input(KeyCode::Char('$')), state);
        let entry = state.workspace.focused_pane().cursor_entry().unwrap();
        let len = crate::tui::pane::row_text(entry).chars().count();
        assert_eq!(state.workspace.focused_pane().effective_col(), len - 1);
        let state = handle_tui_event(input(KeyCode::Char('0')), state);
        assert_eq!(state.workspace.focused_pane().effective_col(), 0);

        // Charwise selection text: anchor at col 0, move right 4 -> 5 chars.
        let state = handle_tui_event(input(KeyCode::Char('v')), state);
        let state = handle_tui_event(input(KeyCode::Char('4')), state);
        let state = handle_tui_event(input(KeyCode::Char('l')), state);
        match state.workspace.mode {
            Mode::Visual {
                anchor_seq,
                anchor_col,
                linewise,
            } => {
                assert!(!linewise);
                let text = state
                    .workspace
                    .focused_pane()
                    .charwise_selection_text(anchor_seq, anchor_col);
                assert_eq!(text.chars().count(), 5);
            }
            mode => panic!("expected charwise visual, got {mode:?}"),
        }
    }

    #[test]
    fn filter_command_sets_patterns_and_redispatches() {
        let mut state = state_with_entries(3);
        seed_tail(&mut state, &[1, 2, 3]);

        let mut state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "filter src-a".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let mut state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.workspace.mode, Mode::Normal);
        assert_eq!(
            state.workspace.focused_pane().filter,
            vec!["src-a".to_string()]
        );
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search { sources, .. } if sources == &vec!["src-a".to_string()]
        )));
    }

    #[test]
    fn unknown_command_sets_notice() {
        let state = state_with_entries(1);
        let state = handle_tui_event(input(KeyCode::Char(':')), state);
        let state = handle_tui_event(input(KeyCode::Char('x')), state);
        let state = handle_tui_event(input(KeyCode::Enter), state);
        assert_eq!(state.workspace.notice.as_deref(), Some("not a command: x"));
    }

    #[test]
    fn hs_command_stacks_panes() {
        use crate::tui::workspace::Node;

        let mut state = state_with_entries(1);
        state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "hs".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let state = handle_tui_event(input(KeyCode::Enter), state);

        match &state.workspace.tab().tree {
            Node::Split { dir, children } => {
                assert_eq!(*dir, Direction::Vertical);
                assert_eq!(children.len(), 2);
            }
            node => panic!("expected stacked split, got {node:?}"),
        }
    }

    #[test]
    fn tabnew_creates_focused_tailing_tab_and_gt_cycles() {
        let mut state = state_with_entries(1);
        state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "tabnew errors".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let mut state = handle_tui_event(input(KeyCode::Enter), state);

        assert_eq!(state.workspace.tabs.len(), 2);
        assert_eq!(state.workspace.active_tab, 1);
        assert_eq!(state.workspace.tab().name, "errors");
        assert!(state.workspace.focused_pane().follow);
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search {
                query: Query::Tail,
                ..
            }
        )));

        let state = handle_tui_event(input(KeyCode::Char('g')), state);
        let state = handle_tui_event(input(KeyCode::Char('t')), state);
        assert_eq!(state.workspace.active_tab, 0);
    }

    #[test]
    fn enter_on_stream_opens_detail_and_esc_closes() {
        let mut state = state_with_entries(3);
        seed_tail(&mut state, &[1, 2, 3]);

        let state = handle_tui_event(input(KeyCode::Enter), state);
        assert!(state.workspace.focused_pane().detail_open);

        // Keys are swallowed by the overlay except scroll/close.
        let state = handle_tui_event(input(KeyCode::Char('j')), state);
        assert_eq!(state.workspace.focused_pane().detail_scroll, 1);
        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(3));

        let state = handle_tui_event(input(KeyCode::Esc), state);
        assert!(!state.workspace.focused_pane().detail_open);
    }

    #[test]
    fn sources_picker_narrows_toggles_and_applies_exact_filters() {
        let mut state = state_with_entries(1);
        for id in ["demo-2", "demo-10"] {
            state = producer::handle_producer_event(
                ProducerEvent::SourceFound(Source {
                    producer: "fake".to_string(),
                    id: id.to_string(),
                    display_name: id.replace("demo-", "Demo "),
                    group: None,
                }),
                state,
            );
        }
        drain_search_events(&mut state);

        // `:sources` opens the picker.
        let mut state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "sources".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let state = handle_tui_event(input(KeyCode::Enter), state);
        assert!(state.workspace.picker.is_some());

        // Typing narrows; "demo 2" matches only the Demo 2 row.
        let mut state = state;
        for c in "demo 2".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        {
            let picker = state.workspace.picker.as_ref().unwrap();
            let rows = picker.rows(&state.producer.sources);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, "demo-2");
        }

        // Tab toggles it; Enter applies an exact pattern and redispatches.
        let state = handle_tui_event(input(KeyCode::Tab), state);
        let mut state = handle_tui_event(input(KeyCode::Enter), state);

        assert!(state.workspace.picker.is_none());
        assert_eq!(
            state.workspace.focused_pane().filter,
            vec!["=Demo 2".to_string()]
        );
        let events = drain_search_events(&mut state);
        assert!(events.iter().any(|event| matches!(
            event,
            SearchEvent::Search { sources, .. } if sources == &vec!["demo-2".to_string()]
        )));
    }

    #[test]
    fn picker_enter_with_no_toggle_applies_highlighted_row() {
        let mut state = state_with_entries(1);
        drain_search_events(&mut state);
        let mut state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "ls".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let state = handle_tui_event(input(KeyCode::Enter), state);

        let state = handle_tui_event(input(KeyCode::Enter), state);

        assert!(state.workspace.picker.is_none());
        assert_eq!(
            state.workspace.focused_pane().filter,
            vec!["=Source A".to_string()]
        );
    }

    #[test]
    fn exact_filter_pattern_does_not_substring_match() {
        let mut state = state_with_entries(1);
        for (id, name) in [("demo-2", "Demo 2"), ("demo-20", "Demo 20")] {
            state = producer::handle_producer_event(
                ProducerEvent::SourceFound(Source {
                    producer: "fake".to_string(),
                    id: id.to_string(),
                    display_name: name.to_string(),
                    group: None,
                }),
                state,
            );
        }
        let pane = state.workspace.focused_pane_mut();
        pane.filter = vec!["=Demo 2".to_string()];
        assert_eq!(
            pane.resolve_filter(&state.producer.sources),
            Some(vec!["demo-2".to_string()])
        );
        pane.filter = vec!["Demo 2".to_string()];
        assert_eq!(
            pane.resolve_filter(&state.producer.sources),
            Some(vec!["demo-2".to_string(), "demo-20".to_string()])
        );
    }

    #[test]
    fn tab_completes_command_names_and_filter_sources() {
        let mut state = state_with_entries(1);
        state = producer::handle_producer_event(
            ProducerEvent::SourceFound(Source {
                producer: "fake".to_string(),
                id: "api-1".to_string(),
                display_name: "Api One".to_string(),
                group: Some("backend".to_string()),
            }),
            state,
        );
        drain_search_events(&mut state);

        // `:fi<Tab>` completes the command name.
        let mut state = handle_tui_event(input(KeyCode::Char(':')), state);
        for c in "fi".chars() {
            state = handle_tui_event(input(KeyCode::Char(c)), state);
        }
        let mut state = handle_tui_event(input(KeyCode::Tab), state);
        assert_eq!(state.workspace.prompt.buf, "filter");

        // `:filter A<Tab>` completes a source name; Tab cycles candidates.
        state.workspace.prompt.insert(' ');
        state.workspace.completion = None;
        let state = handle_tui_event(input(KeyCode::Char('A')), state);
        let state = handle_tui_event(input(KeyCode::Tab), state);
        assert_eq!(state.workspace.prompt.buf, "filter Api One");

        // A comma starts a new token; `b<Tab>` completes the group name.
        let state = handle_tui_event(input(KeyCode::Char(',')), state);
        let state = handle_tui_event(input(KeyCode::Char('b')), state);
        let state = handle_tui_event(input(KeyCode::Tab), state);
        assert_eq!(state.workspace.prompt.buf, "filter Api One,backend");
    }

    #[test]
    fn help_overlay_swallows_input_until_closed() {
        let mut state = state_with_entries(3);
        seed_tail(&mut state, &[1, 2, 3]);
        let state = handle_tui_event(input(KeyCode::Char('?')), state);
        assert!(state.workspace.help_open);

        let state = handle_tui_event(input(KeyCode::Char('k')), state);
        assert_eq!(state.workspace.focused_pane().cursor_seq, Some(3));

        let state = handle_tui_event(input(KeyCode::Esc), state);
        assert!(!state.workspace.help_open);
    }
}
