# fml TUI Architecture

The TUI layer lives in the `fml-tui` crate. It is a `ratatui` application driven by a `crossterm` event loop. This document covers the design decisions in enough detail to serve as a reusable pattern for future ratatui projects.

## High-level structure

```
src/
├── app.rs          App shell, AppState, Focus FSM, event loop, draw()
├── event.rs        AppEvent enum, crossterm → AppEvent mapping
├── theme.rs        Theme struct, TOML-driven colour system
├── lib.rs          run() entry point, mock data
└── widgets/
    ├── tab_bar.rs
    ├── producer_tree.rs
    ├── log_stream.rs
    ├── query_bar.rs
    ├── command_bar.rs
    └── help.rs
```

---

## Event system (`event.rs`)

### AppEvent

All input is translated from raw `crossterm::event::Event` into the `AppEvent` enum before any widget or application logic touches it. Widgets never import `crossterm` directly.

```rust
pub enum AppEvent {
    Quit, Exit, FocusNext, QueryFocus,
    ScrollUp, ScrollDown, ScrollToTail,
    GreedUp, GreedDown,
    TreeNav(Direction),   // Up / Down / Left / Right
    Char(char),
    Backspace, Enter, Escape,
    Resize(u16, u16),
    // High-level semantic events (emitted by command bar parsing)
    Help, Theme(String), Timestamps, Greed(u8),
    NoOp,
}
```

The `AppEvent` vocabulary is intentionally widget-agnostic. A `TreeNav(Down)` means "move down in a list context" — whether that means scrolling the log stream or moving the cursor in the producer tree is decided by the app shell based on current focus, not by the event itself.

### Two parse modes

The same physical key produces different `AppEvent` values depending on whether a text-input widget is active.

| Mode | Function | When used |
|------|----------|-----------|
| Normal | `AppEvent::parse_event` | Tree, Stream focus |
| Insert | `AppEvent::parse_insert_event` | QueryBar, Command focus |

In **normal mode**, navigation shortcuts take priority: `k` → `TreeNav(Up)`, `q` → `Quit`, `]` → `GreedUp`, etc.

In **insert mode**, all printable characters produce `Char(c)` so the user can type freely. Arrow keys still produce `TreeNav(Left/Right)` so they work as cursor movement inside the text input. Only `Ctrl+c`, `Escape`, `Enter`, `Tab`, and `Backspace` keep their special bindings.

The event loop selects the mode based on current focus before calling either function:

```rust
let app_event = if is_insert_mode(self.state.focus) {
    AppEvent::parse_insert_event(raw)
} else {
    AppEvent::parse_event(raw)
};
```

### Command parsing (`AppEvent::parse_str`)

The command bar accepts a text string typed after `:` and parses it into an `AppEvent` via `AppEvent::parse_str`. This means the command vocabulary is part of the event system — a command bar that produces `AppEvent::Theme("gruvbox")` goes through exactly the same dispatch path as a keybinding that would produce the same event.

```
":theme gruvbox" → AppEvent::parse_str → Ok(AppEvent::Theme("gruvbox"))
                                       ↓
                              app.handle(AppEvent::Theme(...))
```

---

## Application shell (`app.rs`)

### AppState

All mutable application state lives in `AppState`. No state is stored inside widgets; widgets receive a shared reference to the relevant slice of state at render time.

```rust
pub struct AppState {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub focus: Focus,
    pub prev_focus: Focus,     // restored when exiting command mode
    pub theme: Theme,
    pub config: Config,
    pub show_help: bool,
    pub command_bar: CommandBarState,
    pub quit: bool,
}

pub struct TabState {
    pub label: String,
    pub kind: TabKind,         // Main | Freeze(producer) | Correlate { field, value }
    pub tree: ProducerTreeState,
    pub stream: LogStreamState,
    pub query: QueryBarState,
    pub dirty: bool,
}
```

Each tab owns its own widget states independently. All tabs share the same store (not yet wired in Phase 2 — currently uses mock data).

### Focus state machine

```
        Tab          Tab          Escape / Tab
  Tree ──────▶ Stream ──────▶ QueryBar ──────────▶ Tree
    ▲                                    /
    └──────────────────────────────────

  Any (except QueryBar) ──:──▶ Command ──Escape/Enter──▶ prev_focus
```

`Focus` is an enum with four variants: `Tree`, `Stream`, `QueryBar`, `Command`. The app shell checks focus before dispatching events to widgets, and also before selecting the crossterm→AppEvent parse mode.

`Command` mode stores `prev_focus` on entry and restores it on `Escape` or successful `Enter`, so `:theme gruvbox` followed by Enter returns focus to wherever the user was before opening the command line.

### Event loop

```rust
loop {
    terminal.draw(|frame| draw(frame, &state))?;

    if state.quit { break; }

    if ct_event::poll(Duration::from_millis(16))? {
        match ct_event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let raw = Event::Key(key);
                let app_event = if is_insert_mode(state.focus) {
                    AppEvent::parse_insert_event(raw)
                } else {
                    AppEvent::parse_event(raw)
                };
                if let Some(ev) = app_event {
                    self.handle(ev);
                }
            }
            other => { /* resize etc */ }
        }
    }
}
```

Key design decisions:
- **Draw before poll**: the frame is rendered once per loop iteration before waiting for input. This means the first frame appears immediately and the UI is never stale by more than one poll interval.
- **16 ms poll**: gives ~60 fps. Increase to 8 ms for 120 fps if needed for smooth animations.
- **KeyEventKind::Press only**: crossterm on some terminals emits both press and release events. Filtering to Press prevents double-handling.
- **Separate parse modes**: selected before the event reaches `handle()`, so `handle()` never needs to know which physical key was pressed — only the semantic event.

### handle()

`handle()` is the single entry point for all `AppEvent` values. It is a layered match with three priority levels:

```
1. Help popup active?  → intercept (only close keys pass through)
2. Command mode active? → intercept (route to command bar)
3. Normal dispatch      → match on event, then dispatch_to_focused()
```

Global events (quit, focus cycle, greed adjustment, theme switch) are handled at the app level before `dispatch_to_focused`. This means `]`/`[` adjust greed regardless of which pane is focused.

`dispatch_to_focused` routes the remaining events to the widget that owns focus:

```rust
fn dispatch_to_focused(s: &mut AppState, event: AppEvent) {
    match s.focus {
        Focus::Tree    => s.tabs[s.active_tab].tree.handle(&event),
        Focus::Stream  => s.tabs[s.active_tab].stream.handle(&event),
        Focus::QueryBar => s.tabs[s.active_tab].query.handle(&event),
        Focus::Command => {} // already handled
    }
}
```

---

## Widget pattern

Every widget follows the same three-part structure:

### 1. State struct

Owns all mutable data for the widget. Lives in `AppState` (or `TabState`), not inside the widget itself.

```rust
pub struct LogStreamState {
    pub entries: Vec<LogEntry>,
    pub scroll_offset: usize,
    pub cursor: usize,
    pub paused: bool,
    pub buffered_new: usize,
    pub show_timestamps: bool,
    pub last_height: Cell<usize>,  // set during render, read during handle
}
```

### 2. handle(&AppEvent)

Takes an event reference, mutates state, returns nothing (or `Option<AppEvent>` for widgets that re-emit events, like the command bar).

```rust
impl LogStreamState {
    pub fn handle(&mut self, event: &AppEvent) { ... }
}

impl CommandBarState {
    // Returns Some(event) to tell the app shell to close the bar and dispatch
    // the event; None means stay open.
    pub fn handle(&mut self, event: &AppEvent) -> Option<AppEvent> { ... }
}
```

### 3. Widget struct (render-only)

A short-lived struct that borrows the state and theme, constructed just before `render()` is called. It implements `ratatui::widgets::Widget` (or `StatefulWidget` if ratatui manages its own selection state, as with `List`).

```rust
pub struct LogStream<'a> {
    state: &'a LogStreamState,
    focused: bool,
    theme: &'a Theme,
}

impl Widget for LogStream<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) { ... }
}
```

The widget struct is never stored. It is created in `draw()`, rendered immediately, and dropped.

### Why separate state from widget?

- `AppState` can be passed as `&AppState` to `draw()` (immutable borrow for the entire frame) while `handle()` takes `&mut AppState` between frames.
- Widget state persists across frames; the widget struct does not need to.
- Testing state logic (`handle`, selection invariants) does not require a terminal backend.

---

## Rendering pipeline (`draw`)

```rust
fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    // 1. Compute layout rectangles
    let vert = Layout::vertical([Length(1), Fill(1), Length(3)]).split(area);
    let horiz = Layout::horizontal([Percentage(pct), Fill(1)]).split(vert[1]);

    // 2. Render each widget into its rectangle
    frame.render_widget(TabBar::new(...),        vert[0]);
    frame.render_widget(ProducerTree::new(...),  horiz[0]);
    frame.render_widget(LogStream::new(...),     horiz[1]);
    frame.render_widget(QueryBar::new(...),      vert[2]);

    // 3. Overlays (rendered last, on top)
    if state.show_help {
        frame.render_widget(HelpPopup::new(...), area);
    }
    if state.focus == Focus::Command {
        let cmd_area = Rect { y: area.bottom() - 2, height: 2, ..area };
        frame.render_widget(CommandBar::new(...), vert[2]);
        frame.set_cursor_position((col, cmd_area.y));
        return; // cursor already set; skip QueryBar cursor below
    }
    if state.focus == Focus::QueryBar {
        frame.set_cursor_position(qb.cursor_position(vert[2]));
    }
}
```

Key conventions:
- **Layout first, render second**: all `Rect` values are computed upfront, then widgets render into them. Never compute layout inside a widget.
- **Overlays last**: widgets that float above the layout (help popup, command bar) are rendered after the base layer so they paint over it.
- **Cursor position is set by draw()**, not by the widget. The widget exposes a `cursor_position(area) -> (u16, u16)` helper; `draw()` calls it and hands the result to `frame.set_cursor_position()`. This keeps terminal cursor management in one place.
- **`Clear` before overlays**: floating widgets call `Clear.render(area, buf)` before drawing their own content to erase whatever was behind them.

### Scrollbar alignment

When adding a scrollbar to a widget that has a `Block` border, render the scrollbar inside the `inner` area (after `block.inner(area)`), not on the outer `area`. Rendering on the outer area causes a height mismatch: the scrollbar track is 2 rows taller than the visible content, so the thumb never reaches the bottom.

```rust
let inner = block.inner(area);
block.render(area, buf);

// Reserve 1 column for the scrollbar within inner
let text_area = Rect { width: inner.width.saturating_sub(1), ..inner };
let sb_area   = Rect { x: inner.right().saturating_sub(1), width: 1, ..inner };

Paragraph::new(lines).render(text_area, buf);
StatefulWidget::render(scrollbar, sb_area, buf, &mut sb_state);
```

---

## Command bar pattern

The command bar is a vim-style `:` input that overlays the bottom of the screen. Its handle() method does not return `()` — it returns `Option<AppEvent>`. The returned event is dispatched back into `app.handle()`, so commands go through the exact same code path as keybindings:

```
User types ":tail" + Enter
    → CommandBarState::handle(Enter)
    → AppEvent::parse_str("tail") → Ok(AppEvent::ScrollToTail)
    → returns Some(AppEvent::ScrollToTail)
    → app.handle(AppEvent::ScrollToTail)   ← same path as pressing 'G'
```

`None` means "stay open, keep editing". `Some(AppEvent::NoOp)` means "close the bar, but do nothing else" (used for Escape and empty Enter).

This pattern (widget re-emits AppEvent) works well for any modal input that needs to trigger application-level side effects.

---

## Theme system (`theme.rs`)

Themes are TOML files embedded in the binary via `include_str!`. The `Theme` struct holds pre-resolved `ratatui::style::Style` values — no allocation at render time.

```
themes/
├── default.toml       Nord-inspired palette
└── gruvbox_dark.toml  Gruvbox Dark palette
```

```toml
[levels]
trace   = { fg = "#928374" }
error   = { fg = "#fb4934", bold = true }

[borders]
focused      = { fg = "#83a598" }
command_bar  = { fg = "#fabd2f" }
unfocused    = { fg = "#928374" }

[producers]
palette = ["#83a598", "#b8bb26", "#d3869b", ...]
```

Producer colour is assigned by hashing the producer name with a stable (djb2-style) hash and taking `hash % palette.len()`. This makes colour assignment deterministic across restarts regardless of the order producers appear.

Themes can be switched at runtime via `:theme <name>` — `AppState.theme` is replaced with a newly loaded `Theme` struct.

---

## Debug logging

When `--debug` is passed, `tracing-subscriber` is initialised to write structured logs to `/tmp/fml-debug.log`. The TUI writes to the file; a second terminal can run `tail -f /tmp/fml-debug.log` alongside.

Instrumented sites:
- `app.rs` — every key event (`focus`, `event`), focus transitions, quit, command execution
- `log_stream.rs` — cursor position and scroll offset on every movement
- `producer_tree.rs` — cursor and node id on every navigation/selection
- `query_bar.rs` — query text and greed level on every change
- `command_bar.rs` — command parsed, event emitted

All log calls use `tracing::debug!` with structured fields so they are filterable by target: `RUST_LOG=fml_tui::app=debug` shows only app-level events.

---

## Terminal lifecycle

```rust
// Setup
enable_raw_mode()?;
execute!(stdout(), EnterAlternateScreen)?;
let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

// Panic hook — restores terminal even on panic
std::panic::set_hook(Box::new(move |info| {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
    original_hook(info);
}));

// Teardown (in the normal exit path)
disable_raw_mode()?;
execute!(stdout(), LeaveAlternateScreen)?;
terminal.show_cursor()?;
```

The panic hook is installed before `enable_raw_mode` so that any panic during setup is also handled. The hook calls the original hook after restoring the terminal so the panic message is still printed to the now-restored terminal.
