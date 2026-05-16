# BIGPLAN: Text Selection & Yank (#15)

## Plan Overview

Make it easy to get text out of the fml TUI for sharing or further investigation. Two complementary mechanisms: (a) a toggleable **select mode** that releases crossterm's mouse capture so the host terminal's native click-drag selection (and its built-in copy path) work as users already expect; (b) a **yank shortcut** active only when the log pane is focused that serializes the currently selected `LogEntry` as JSON and writes it to the system clipboard via an **OSC52** escape sequence. The OSC52 path avoids native clipboard libraries (no X11/Wayland deps, works over SSH). "Done" means: pressing the select-mode toggle lets users drag-highlight any visible text and copy via the terminal's own shortcut; pressing `y` with the log pane focused emits an OSC52 yank that supported terminals deliver to the system clipboard; neither path degrades existing rendering, event flow, or perceived input latency. Note that delivery is "best effort by terminal" — see Risks; the status-bar message reflects what fml _did_, not what the clipboard _received_.

## Risks

- **Mouse capture toggle leaks state on panic/exit** — `EnableMouseCapture` is set at `app.rs:212` and `DisableMouseCapture` is wired in the panic hook (`tui.rs:41`) and shutdown (`tui.rs:53`). The plan keeps capture _always on at startup_ and toggles to off in select mode, so the existing unconditional teardown remains correct. Mitigation: do not make startup `EnableMouseCapture` conditional (revised from the earlier draft); only the runtime toggle is new. Issuing `DisableMouseCapture` is documented-safe regardless of current state.
- **OSC52 success is unobservable from inside the process** — there is no in-band reply to an OSC52 write, no `$TERM`-based detection that is reliable, and many common environments silently drop the sequence: macOS Terminal.app (no support at all), GNOME Terminal / VTE before 3.50, Windows Console Host (only ConPTY-wrapped Windows Terminal works, and only on recent builds), tmux without `set -g set-clipboard on`, Zellij when the host terminal does not support OSC52 or its clipboard path is otherwise misconfigured, plain SSH into an old xterm. Mitigation: the status-bar message says **"sent yank (N bytes) — check clipboard"** rather than confirming a copy; the README and help popup list the known-good terminals and call out the unsupported or config-dependent ones by name; no fallback library is added.
- **OSC52 payload caps vary by terminal** — published caps: xterm 8KB (smallest common cap), gnome-terminal/vte 8KB historically, alacritty ~100KB practical, kitty/wezterm/iTerm2 effectively unbounded, tmux 1MB with `set-clipboard on`. Zellij's own copy path uses OSC52 by default and can be configured with `copy_command`, but fml's direct OSC52 yank still ultimately depends on the multiplexer and host terminal forwarding/accepting the sequence. The plan picks the **conservative xterm cap of 8KB** as the warning threshold so users in the worst supported environment get a meaningful signal. Mitigation: warn when the **base64-encoded** payload exceeds 8KB; the message names the cap so users know which environments are at risk. No chunked protocol (not standardized).
- **Multiplexers are the most likely silent-failure environment** — fml's audience overlaps heavily with tmux and Zellij users. Tmux drops OSC52 unless `set -g set-clipboard on` is configured. Zellij uses OSC52 for its own copy path by default and documents `copy_command` as the fallback when the host terminal does not support OSC52, but nested application OSC52 behavior still needs real verification. Documentation alone is a weak mitigation. Mitigation: detect `$TMUX` and `$ZELLIJ` and on the _first_ yank in that session show a one-time status-bar hint using a shared `multiplexer_clipboard_hint_shown` flag. No bare-tmux passthrough wrapper and no Zellij-specific escape wrapper in this plan; only modern/configured multiplexer behavior is first-class.
- **Mouse-capture release loses in-app mouse events** — verified by grep at plan time: no widget consumes `TuiEvent::Mouse` (the variant is dispatched in `tui.rs:83` and ignored everywhere else). The toggle therefore has zero functional regression today. **But** for users who rely on terminal scrollback wheel scrolling, the _current_ default of capture-on already blocks that; the new toggle is the _first_ way to get scrollback back. Mitigation: document this user-visible upside in the README; pin the grep result as a verification task in Deliverable 1 so the assumption is checked at implementation time, not just plan time.
- **Synchronous stdout write inside the event loop can block** — OSC52 is a stdout write, and stdout can in principle block (slow pipe, Ctrl+S pause, tmux backpressure). The same pattern is already used by ratatui's `terminal.draw` and the existing `EnableMouseCapture` call, so this is consistent with current behavior. Mitigation: accept the risk for typical payload sizes (≤ a few KB for a log entry); the 8KB warning naturally caps the worst case; do not move the write off the event-loop thread.
- **Selection state is already defined** — `state.tui.selected_entry: Option<SelectedEntry>` exists at `fml/src/state/tui_state.rs:81`. "No selection" means `None`. The yank task does _not_ introduce a new selection model.

## Plan Details

### Critical Files

- `fml/src/app.rs` — leave `EnableMouseCapture` at `app.rs:212` **unconditional** at startup. The toggle operates at runtime only; this preserves the existing teardown invariant.
- `fml/src/tui.rs` — panic hook (`tui.rs:41`) and `kill()` (`tui.rs:53`) unconditionally issue `DisableMouseCapture` (no change). Also home of the crossterm event reader; mouse events flow into `TuiEvent::Mouse` and remain unhandled.
- `fml/src/event.rs` — no new event variants required. Toggle and yank are both produced as `CustomizedKeyAction` values and handled inline in `handle_tui_event`.
- `fml/src/state/tui_state.rs` — `TuiState` gains: `select_mode: bool` (default `false`); `status_message: Option<StatusMessage>` (transient slot used by Deliverable 0); `multiplexer_clipboard_hint_shown: bool` (one-shot flag for tmux/Zellij clipboard hints).
- `fml/src/config/tui.rs` — extend the existing `[tui.keybindings]` surface with `toggle_select_mode` and `yank_selected_entry` defaulting to `["f2"]` and `["y"]`, so help/status labels stay aligned with the config contract already documented for keybindings.
- `fml/src/tui/keybinds.rs` — add `CustomizedKeyAction::ToggleSelectMode` (hint section `HelpSection::Global`, default label `F2`) and `CustomizedKeyAction::YankSelectedEntry` (hint section `HelpSection::LogPane`, default label `y`), wired through the same runtime binding resolution used by existing configurable actions.
- `fml/src/tui.rs` (`handle_tui_event`) — branch on `ToggleSelectMode` to flip `state.tui.select_mode` and execute `EnableMouseCapture`/`DisableMouseCapture`; branch on `YankSelectedEntry` only when `state.tui.focused == Slot::LogPane` and no popup is active.
- `fml/src/log.rs` — `LogEntry` already derives `Serialize` (line 58–73). `serde_json::to_string(&*entry)` is all that's needed.
- New: `fml/src/clipboard.rs` — `pub fn yank_osc52<W: Write>(out: &mut W, payload: &str) -> Result<usize, FmlError>`. Encodes via `base64` (pure-Rust crate, no system deps), writes `\x1b]52;c;<base64>\x1b\\`, returns the encoded byte length. The `impl Write` parameter makes it unit-testable with a `Cursor<Vec<u8>>`.
- `fml/src/tui/widgets/status_bar.rs` — render the transient `status_message` (Deliverable 0) and a small `[SELECT]` indicator when `select_mode == true`.

### Gotchas

- **Mouse capture is currently a no-op** — `TuiEvent::Mouse` is dispatched but unhandled (verified by grep: zero `MouseEventKind` matches outside the dispatch site). The toggle therefore has zero functional regression on current features.
- **Startup capture stays on** — do not make startup capture conditional; only the runtime toggle is new. This keeps the existing panic/exit invariant intact (always disable on teardown, regardless of starting state).
- **Selected entry source** — yank reads `state.tui.selected_entry: Option<SelectedEntry>` (defined at `fml/src/state/tui_state.rs:81`); `None` is the no-op case.
- **Keybind choice** — `F2` is the chosen toggle key. Ctrl+\ was considered but is the POSIX SIGQUIT chord; while crossterm raw mode masks it, F2 is unambiguous and matches the function-key convention used elsewhere (e.g., midnight commander, htop).
- **`stdout().execute(...)` from inside the event loop is synchronous** — same pattern as the existing capture call at `app.rs:212`. Safe.
- **Render contention with OSC52 writes** — `render()` runs on the same thread as input handling and also writes to stdout. Writing OSC52 between frames is fine; ratatui's draw call flushes before returning.
- **`y` in query box** — query box uses `tui-textarea` and captures alphanumerics when focused. Because yank is gated on `focused == Slot::LogPane`, typing `y` in the query box continues to insert the character.
- **`Y` (shift-y) stays unbound** — out of scope for this plan; log pane already binds `G` for jump-to-tail and we want symmetry with future "yank N" behaviors before claiming `Y`.
- **Help popup auto-updates** — built from action hints and runtime keybinding labels. New entries appear automatically once added, and their displayed labels must follow `[tui.keybindings]` overrides.
- **`base64` is pure-Rust** — the `base64` crate (`0.22`) has no system requirements. Adding it does not undo the "no native deps" rationale for choosing OSC52 over a clipboard library.
- **Multiplexer detection** — `std::env::var("TMUX").is_ok()` and `std::env::var("ZELLIJ").is_ok()` are sufficient for first-yank hints; both variables are set for nested processes by their respective multiplexers. If both are present, prefer the more specific status message `multiplexer: clipboard may need tmux/Zellij setup`.
- **OSC52 success cannot be confirmed** — fml only knows it _wrote_ the sequence. All user-facing messages should be honest about that ("sent yank — check clipboard").

### Pseudo-code / Sketches

```text
// fml/src/clipboard.rs
pub fn yank_osc52<W: Write>(out: &mut W, payload: &str) -> Result<usize, FmlError> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
    let len = encoded.len();
    write!(out, "\x1b]52;c;{}\x1b\\", encoded)?;
    out.flush()?;
    Ok(len)
}

// fml/src/tui.rs, inside handle_tui_event Input branch
const OSC52_WARN_BYTES: usize = 8 * 1024; // xterm/vte conservative cap

if custom_key == CustomizedKeyAction::ToggleSelectMode {
    state.tui.select_mode = !state.tui.select_mode;
    let _ = if state.tui.select_mode {
        stdout().execute(DisableMouseCapture)
    } else {
        stdout().execute(EnableMouseCapture)
    };
    return state;
}
if custom_key == CustomizedKeyAction::YankSelectedEntry
    && state.tui.focused == Slot::LogPane
    && state.tui.active_popup().is_none()
{
    let Some(selected) = state.tui.selected_entry.as_ref() else {
        return state; // silent no-op; nothing to yank
    };
    let json = serde_json::to_string(&*selected.entry).unwrap_or_default();
    let mut out = stdout().lock();
    let msg = match clipboard::yank_osc52(&mut out, &json) {
        Ok(n) if n > OSC52_WARN_BYTES =>
            format!("sent yank ({n} bytes) — exceeds 8KB; xterm/vte may drop it"),
        Ok(n) => format!("sent yank ({n} bytes) — check clipboard"),
        Err(e) => format!("yank failed: {e}"),
    };
    let now = Instant::now();
    state.tui.set_status_message(msg, now);
    if multiplexer_hint().is_some() && !state.tui.multiplexer_clipboard_hint_shown {
        state.tui.multiplexer_clipboard_hint_shown = true;
        state.tui.queue_status_message(multiplexer_hint().unwrap(), now);
    }
    return state;
}
```

## Deliverables

### Deliverable 0. Status-bar transient message infrastructure

Add a transient-message slot to `TuiState` and surface it in the status bar widget. This is shared infrastructure used by Deliverables 1 and 3 (`[SELECT]` indicator, "sent yank" message, multiplexer hint). Pinning it as its own deliverable removes the implicit dependency and the design churn of Deliverable 1 inventing a model that Deliverable 3 then retrofits.

Acceptance:

- `TuiState::set_status_message(msg, now)` records the message with the caller-provided timestamp.
- `TuiState::status_message(now)` returns `Some(&str)` only while the message is within a fixed TTL (proposed: 3 seconds); otherwise `None`. TTL is checked at render time, not via a background timer (no extra task). Passing `now` makes the production path and tests use the same API without sleeps or test-only helpers.
- A render-time suppression flag (`TuiConfig::suppress_status_messages`, default `false`) lets snapshot tests render with messages hidden so existing snapshots stay stable.
- The status bar renders the current transient message in a reserved region; absent message renders the existing content unchanged.

- [ ] Add `status_message: Option<(String, Instant)>` and `status_message_ttl: Duration` to `TuiState`.
- [ ] Add `set_status_message(msg, now)` / `status_message(now)` accessors using the TTL.
- [ ] Add `suppress_status_messages: bool` to `TuiConfig` and route through to `status_bar` render.
- [ ] Update `status_bar.rs` to render the message (when present and not suppressed) in a known region.
- [ ] Unit test: `set_status_message(msg, now)` then call `status_message(now + ttl + 1ms)` → returns `None`.
- [ ] Snapshot test: existing status-bar snapshots run with `suppress_status_messages = true` and remain byte-identical.

### Deliverable 1. Toggleable mouse capture (select mode)

Introduce a runtime-toggleable select mode bound to `F2`. On toggle-on, fml issues `DisableMouseCapture`, lets the terminal handle drag-selection and wheel scrollback, and shows a `[SELECT]` indicator in the status bar. On toggle-off, fml re-issues `EnableMouseCapture`. Startup capture remains unconditional so the existing teardown invariant holds.

Acceptance:

- `F2` flips capture state synchronously.
- `[SELECT]` appears in the status bar when in select mode and is gone when not.
- Quit (`Ctrl+C`/`q`), panic, and normal exit always leave the terminal with capture _off_, regardless of mode at exit.
- Verified at implementation time: no widget consumes `TuiEvent::Mouse` (capture-off has no in-app regressions).

- [ ] Add `select_mode: bool` to `TuiState` (default `false`).
- [ ] Add `toggle_select_mode` to `TuiConfig.keybindings` with default `["f2"]`.
- [ ] Add `CustomizedKeyAction::ToggleSelectMode` + action hint (`HelpSection::Global`, default label `F2`) sourced from runtime keybindings.
- [ ] Wire toggle in `handle_tui_event`: flip the bool, execute `EnableMouseCapture`/`DisableMouseCapture`.
- [ ] Render `[SELECT]` indicator in `status_bar.rs` when `select_mode == true`.
- [ ] Re-grep `TuiEvent::Mouse` consumers at implementation time and pin the result in a comment near the toggle (sentinel against silent regressions if a future widget starts consuming mouse events).
- [ ] Test: panic path while `select_mode == true` (`disable_raw_mode` + `DisableMouseCapture` both invoked); confirm `DisableMouseCapture` is a safe no-op when capture is already off.
- [ ] Unit test: toggle action flips `state.tui.select_mode`.
- [ ] Run snapshot suite; confirm zero diffs in the default state (select_mode off, no status message).

### Deliverable 2. Clipboard module (OSC52 writer)

A small, self-contained module that base64-encodes a string and writes the OSC52 sequence to an `impl Write`. Returns the encoded byte length so callers can warn on oversize payloads.

Acceptance:

- `yank_osc52(&mut buf, "hello")` writes exactly `\x1b]52;c;aGVsbG8=\x1b\\` to `buf`, returns `Ok(8)`.
- `yank_osc52(&mut buf, "")` writes `\x1b]52;c;\x1b\\`, returns `Ok(0)`.
- A write error from the underlying `Write` is mapped to `FmlError`.
- No globals: the function takes the writer; callers (production) pass `stdout().lock()`; tests pass a `Cursor<Vec<u8>>`.

- [ ] Add `base64 = "0.22"` to `fml/Cargo.toml`.
- [ ] Create `fml/src/clipboard.rs` with `yank_osc52<W: Write>(...)`.
- [ ] Register module in `fml/src/lib.rs`.
- [ ] Unit test: exact byte sequence for `"hello"` via `Cursor<Vec<u8>>`.
- [ ] Unit test: empty string.
- [ ] Unit test: writer error path returns `FmlError`.

### Deliverable 3. Yank selected entry shortcut

Bind lowercase `y` to serialize `state.tui.selected_entry.entry` as JSON and emit it via `clipboard::yank_osc52`. Gated on log pane focus and no active popup. Surface the result through the Deliverable 0 status-bar slot. On first yank inside a tmux or Zellij session, queue a one-time multiplexer-specific hint.

Acceptance:

- With log pane focused and `selected_entry == Some(_)`, pressing `y` writes the entry JSON via OSC52 and shows **"sent yank (N bytes) — check clipboard"** (or the >8KB warning variant).
- With log pane focused and `selected_entry == None`, `y` is a silent no-op (no message, no clipboard write).
- With query box focused, `y` continues to insert the character.
- With any popup open, `y` is swallowed by popup handlers (no regression).
- Under `$TMUX`, the first yank also shows a one-time hint message about `set -g set-clipboard on`. Subsequent yanks in the same session do not repeat the hint.
- Under `$ZELLIJ`, the first yank also shows a one-time hint message that Zellij clipboard delivery depends on OSC52-capable terminals or `copy_command` for Zellij's own copy path. Subsequent yanks in the same session do not repeat the hint.
- The status-bar message wording reflects "sent," not "copied" — we cannot confirm clipboard receipt.

- [ ] Add `yank_selected_entry` to `TuiConfig.keybindings` with default `["y"]`.
- [ ] Add `CustomizedKeyAction::YankSelectedEntry` + action hint (`HelpSection::LogPane`, default label `y`) sourced from runtime keybindings.
- [ ] Add focus + popup gating in `handle_tui_event` before the existing widget-dispatch loop.
- [ ] Serialize entry with `serde_json::to_string(&*selected.entry)`.
- [ ] Call `clipboard::yank_osc52` with `stdout().lock()`; route result string into `set_status_message`.
- [ ] Implement the 8KB warning branch in the result-matching code; message names the cap.
- [ ] Implement the `$TMUX` / `$ZELLIJ` one-time hint via `multiplexer_clipboard_hint_shown` flag on `TuiState`.
- [ ] Unit test: yank with `selected_entry == None` is a silent no-op (no status message, no write).
- [ ] Unit test: yank with query-box focus does not invoke the yank branch (gated out).
- [ ] Unit test: yank with `selected_entry == Some(_)` produces expected JSON via a test seam (factor the write target so the test can pass a `Vec<u8>`).
- [ ] Unit test: 8KB threshold — payload at exactly 8192 base64 bytes yields the normal message; 8193 yields the warning.
- [ ] Unit test: with env var simulating `TMUX`, second yank in the same `TuiState` does not re-queue the hint.
- [ ] Unit test: with env var simulating `ZELLIJ`, second yank in the same `TuiState` does not re-queue the hint.

### Deliverable 4. Documentation & manual verification matrix

Update the README so users know how to use both features and which terminals are supported. The help popup updates automatically from the action hints and runtime keybinding labels; this deliverable is README + a recorded manual verification matrix.

Acceptance:

- README has a new **"Selecting & copying text"** section explaining:
  - `F2` toggles select mode for drag-highlight and terminal wheel scrollback.
  - `y` (log pane focused) sends the selected entry as JSON via OSC52.
  - **Tmux users need `set -g set-clipboard on`** (explicit acceptance criterion).
  - **Zellij users should verify their host terminal supports OSC52; for Zellij's own copy path, `copy_command` is the documented fallback when OSC52 is unsupported** (explicit acceptance criterion).
  - Known-working terminals: alacritty, wezterm, kitty, iTerm2, recent xterm, Windows Terminal (with ConPTY).
  - Known-broken terminals: macOS Terminal.app, GNOME Terminal / VTE before 3.50, Windows Console Host, tmux without the setting above.
- A manual verification matrix is recorded in the PR description (not in-repo) covering the cells below.

Manual verification matrix (one PR pass; record outcome per cell):

| Terminal                                      | Select-mode drag-copy | `y` yank (paste to external app) | Multiplexer note shown when applicable |
| --------------------------------------------- | --------------------- | -------------------------------- | -------------------------------------- |
| alacritty                                     | ✓                     | ✓                                | n/a                                    |
| wezterm                                       | ✓                     | ✓                                | n/a                                    |
| kitty                                         | ✓                     | ✓                                | n/a                                    |
| inside tmux on alacritty (no `set-clipboard`) | ✓                     | ✗ silently dropped + hint shown  | ✓                                      |
| inside tmux on alacritty (`set-clipboard on`) | ✓                     | ✓                                | ✓ on first only                        |
| inside Zellij on alacritty                    | ✓                     | verify                           | ✓ on first only                        |
| inside Zellij on wezterm                      | ✓                     | verify                           | ✓ on first only                        |
| Terminal.app                                  | ✓                     | ✗ (documented as unsupported)    | n/a                                    |

- [ ] Add README section per acceptance.
- [ ] Verify the help popup renders both actions in their respective sections.
- [ ] Run the manual matrix and paste outcomes into the PR.

## Issues

- **2026-05-17 — agent:codex** — Review follow-up merged: treat Zellij as a first-class multiplexer customer alongside tmux. The plan now uses `multiplexer_clipboard_hint_shown`, requires `$TMUX` and `$ZELLIJ` first-yank hints, adds Zellij README acceptance and verification matrix rows, keeps keybinding additions aligned with `[tui.keybindings]`, and makes transient status-message TTL tests deterministic by passing `Instant` into the production accessors. This also resolves the earlier deferred configurable yank-keybind follow-up for this feature slice.
- **2026-05-16 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 20 findings; 19 merged. Most significant changes: (1) added Deliverable 0 for the status-bar transient-message infrastructure that D1 and D3 both depend on; (2) lowered the OSC52 warning threshold from 100KB to a conservative 8KB matching xterm/VTE; (3) added explicit `$TMUX` detection with a one-time first-yank hint instead of doc-only mitigation; (4) status-bar wording revised from "yanked" to "sent yank — check clipboard" to be honest about unobservability; (5) keybind decision committed to `F2`; (6) expanded Deliverable 4 into a documented manual verification matrix across supported and unsupported terminals; (7) pinned existing-snapshot stability behind a `TuiConfig::suppress_status_messages` flag. One reviewer finding deferred: adding a legacy `\ePtmux;...\e\\` passthrough wrapper — out of scope; only modern tmux with `set-clipboard on` is supported.
- **2026-05-16 — agent:claude** — Plan drafted from issue #15 with pre-draft questions resolved by the user: mouse capture is toggled (not removed), yank covers selected entry as raw/JSON only, clipboard backend is OSC52, keybinding is vim-style `y` gated on log-pane focus. Open follow-ups deferred to future work, not this plan: (1) yanking a specific field value via a picker; (2) yanking the info-pane or preview-pane contents; (3) `Y` for whole-buffer or multi-entry yank; (4) configurable yank keybind via a `[keybinds]` config table; (5) legacy tmux passthrough wrapper.
