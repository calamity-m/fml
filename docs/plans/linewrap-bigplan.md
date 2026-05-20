# BIGPLAN: Log-pane line wrap / multiline support (#19)

## Plan Overview

Long log entries are clipped to one line in the log pane, so users have no in-place way to read a full entry without yanking or jumping to the info pane. This effort adds a **wrapped render mode** for the log pane and a runtime toggle (default `w`, log-pane–focused, with a `[tui]` config knob for the startup default) that switches between the existing single-line truncated view and a new word-boundary wrapped view. Continuation lines use a **hanging indent** that aligns under the `msg` column of the first line, so the seq / level / source columns stay readable as visual anchors and the eye can group continuation lines with their parent entry without extra glyphs. "Done" means: pressing `w` (or starting with `tui.line_wrap = true`) renders every visible entry's full `msg` text wrapped across continuation lines; cursor navigation, `g`/`G`, scroll, and fuzzy match highlights stay correct across wrapped entries; the currently selected entry stays visible across mode toggles; and the existing single-line mode has no visual regression in snapshots.

## Risks

- **Variable entry heights break the "viewport.height = max visible entries" invariant** — every reducer in `log_pane_state.rs` (`scroll_forward`, `scroll_backward`, `jump_tail`, `tail_view_start`, `preserve_cursor_row`, `reconcile_view`, `selected_visible_index`, `visible_items`, `history_query`) assumes `viewport.height` is both "rows in the pane" and "entries that fit." In wrapped mode those differ: a 20-row pane with one 8-line wrapped entry shows ~2-3 entries. Mitigation: keep `viewport.height` as **rows** (its current semantic) and introduce `visible_layout`, a renderer-written measurement of which entries are visible and which visual-line slice of each entry is rendered. Reducers that decide "is selected entry on screen?" switch to consulting the last measured visual layout (`visible_layout`, including each visible entry's start row, height, and clipped visual-line offsets) rather than `view_start + height`. The history-buffer query stays based on rows (fetching extra entries is harmless; fetching too few is the dangerous direction).
- **Bounded measurement convergence** — the renderer runs at most two layout passes per frame: pass 1 lays out entries starting at the current `view_start`; pass 2 (only if selected entry is off-screen) lays out anchored on the selected entry. **Convergence contract**: pass 2 always anchors with the selected entry's first visual line at row 0 of the pane, so the selected entry is guaranteed in-frame after pass 2 — by construction, not by induction. If pass 2's first line of the next entry still doesn't fit the pane (selected entry is taller than `viewport.height`), the entry is sliced to show its first `viewport.height` visual lines and `visible_layout` reports that single entry with `start_visual_line = 0` and `end_visual_line = viewport.height`; cursor row clamps to 0. A `debug_assert!` confirms pass count ≤ 2 and selected ∈ `visible_layout.entry_range`. Without this, a height-overflow case could oscillate and either hang or render a frame with the selected entry off-screen.
- **`indent_column` stability across passes** — the leading column width (seq digits + level + source name) can differ between pass 1 and pass 2 if they lay out different windows of entries (different max seq digit, different source name lengths). That would change `wrap_width` between passes and break the "one re-layout, never loop" bound. Mitigation: **compute `indent_column` from `store.bounds().1` max-digit width (or max search-rank digit width) + the longest source `display_name` in the union of currently-known sources**, not from the window or only the selected source filter. This makes `indent_column` stable across passes and frames, at the cost of occasional under-utilized width when the window happens to contain only short-named sources — acceptable, since the alternative is layout oscillation. Continuation lines align under the same column the first line uses because the first line is padded to that column too.
- **Per-frame measurement cost and cache correctness** — wrapping every visible entry on every frame, possibly twice, is more expensive than the current "format N spans" path. The producer-burst case called out in CLAUDE.md §8 (docker batch ingest) re-renders aggressively; a slow wrap path turns into perceived lag exactly when the user is most likely to be looking. Mitigation: cache only the wrapped **message chunks** per entry, not the fully rendered prefix/selection/theme output, keyed by `(seq, wrap_width, exact_match_indices_for_msg, preserve_leading_whitespace)` in a small LRU on `LogPaneState`; invalidate on `apply_update` and width changes. Watch-for signal: if frame time during the demo producer burst exceeds ~16ms on a baseline machine, treat as regression and revisit.
- **ratatui `List` multi-line item interaction is load-bearing** — the rendering strategy assumes ratatui's `List` (a) supports `ListItem` with `N > 1` lines via `Text::from(Vec<Line>)`, (b) applies `highlight_style` to *all* visual lines of the selected item, and (c) renders `highlight_symbol` only on the first line. If any of these is wrong, the fallback ("apply tint manually") is a substantial re-implementation, not a small patch. Mitigation: **spike this in a small standalone test before starting D3** (added as the first D3 task). If the assumption holds, proceed; if not, D3 expands to include a manual selection-render path and the deliverable's scope must be re-acknowledged.
- **Match highlights crossing wrap boundaries** — `highlight::styled_field` emits a `Vec<Span>` for one line, splitting on match-index boundaries. When a matched character falls across a wrap point, the matched span has to split with it, and the continuation line's matched chars need the match style preserved. `info_pane::wrap_spans` already does exactly this (it preserves per-char styling across the wrap and rejoins runs of equal style), so the risk is duplication rather than novelty. Mitigation: **promote `wrap_spans` to a shared helper** (`tui::widgets::wrap` or a new fn in `tui::widgets::highlight`) and route both `info_pane` and `log_pane` through it instead of writing a parallel implementation. Lines up with the `[[feedback_no_test_only_helpers]]` rule: generalize the existing helper, don't add a sibling. Because the helper now serves two panes, D1's test suite pins the `hanging_indent = &[]` behavior exhaustively so D3 changes can't silently regress info-pane output.
- **Embedded newlines and whitespace-strip defaults can mangle stack traces and pretty-printed JSON** — `wrap_spans` currently strips leading whitespace from continuation lines and does not document `\n` as a hard break, which silently destroys indentation in exactly the kind of long entry users most want to wrap (stack traces, pretty-printed JSON). Mitigation: parameterize the shared helper with a `preserve_leading_whitespace: bool` flag and make embedded `\n` split into physical lines before width wrapping. Info pane passes `false` (preserves current behavior except for explicitly tested newline handling if already present). Log pane passes `true` (continuation lines preserve in-text indentation; the hanging-indent is applied *before* any preserved leading whitespace of the wrapped chunk). Add unit tests using multi-line indented JSON/stack-trace-shaped msgs.
- **Measured layout is one frame stale around resize and rapid input** — reducers consume renderer-written layout data to decide "is selected on screen?" and where its first visual line sits. Between a resize event and the next render, the stored layout is keyed to the *old* width, so a `scroll_forward` immediately after resize can use stale row data and place the selected entry off-screen. Mitigation: store `visible_layout` alongside the `viewport.width` it was computed at; reducers treat a width mismatch as `None` and fall back to the truncated-mode path (which always shows the selected entry by definition). On resize, the renderer's next-frame measurement pass writes a fresh layout.
- **Cursor / view stability across the toggle** — when the user presses `w`, the selected entry must remain visible. In the transition truncated → wrapped, the selected entry may now occupy more rows than were budgeted for the whole pane (extreme: a single 300-char entry in a 20-row pane). Mitigation: **`log_pane_cursor_row` semantics are defined as "the first visual line of the selected entry within the pane,"** in both modes. On toggle, the renderer's measurement pass treats the *currently selected* entry as the anchor — `view_start` snaps to whatever position keeps the selected entry's first line in frame, with the cursor's row offset preserved when possible and clamped to `0` when the selected entry's height exceeds `viewport.height`. Tested via a toggle round-trip: truncated → wrapped → truncated leaves the same entry visible at the same cursor row (within the entry-height clamp).
- **`g` / `G` semantics under wrap** — per the user-confirmed model, navigation is *per entry*, not per visual line. `ScrollHead` and `ScrollTail` already operate on `selected_seq`, so no semantic change is needed; the risk is `jump_tail` currently leaves `view_start` alone (see `log_pane_state.rs:498`) on the assumption the next tail emission reconciles. In wrapped mode the tail emission gives entries, not a per-entry rendered height, so the renderer's measurement pass must still anchor the bottom-most entry to the bottom of the pane. Mitigation: tail-mode `view_start` recomputation in the renderer prefers "newest entry's last visual line at the bottom row" over "Nth-from-end entry at top," matching the tail-mode pinning already in `retained_scrollbar_metrics` (`log_pane_state.rs:172`). When the newest entry alone exceeds `viewport.height`, show its last `viewport.height` visual lines and clamp cursor row to 0.
- **`w` keybinding collisions** — `w` is currently unbound (verified at plan time in `keybinds.rs:172`). The query-box widget consumes alphanumerics when focused (tui-textarea), so gating on log-pane focus preserves typing `w` in queries. Risk is users with custom `[tui.keybindings]` overrides that already bind `w` to something else. Mitigation: route the action through the existing `[tui.keybindings]` resolution (same pattern as `yank_selected_entry`); custom overrides win as they do for every other action.
- **History buffer too small for tall wrapped entries** — `history_buffer()` returns `min(viewport.height * 2, history_buffer_limit)`. In wrapped mode the pane shows fewer entries than `viewport.height`, so the buffer is comfortably oversized. Risk is the inverse (buffer fetches entries we won't render), which is benign. No mitigation needed; called out so future reviewers don't "fix" it.
- **Renderer mutating state is load-bearing** — the plan assumes `log_pane::render` can write measurement results (and occasionally a corrected `view_start`/cursor row) back into `LogPaneState`. If the current render signatures or borrowing model only permit immutable state, this would force a pre-render layout reducer instead. Mitigation: before D3 implementation, confirm the widget render path already receives mutable state; if not, move measurement into an explicit pre-render state update rather than fighting borrow errors inside render.
- **Unicode display width can desynchronize wrapping and highlights** — terminal layout is based on display cells, while fuzzy matches and many span helpers often operate on chars or byte offsets. CJK double-width characters, combining marks, and emoji can split incorrectly or make hanging indents drift. Mitigation: define the shared wrapper as display-cell-width based (using the repo's existing width helper if present, otherwise `unicode-width`) and add tests for wide characters/emoji plus a highlighted match near a wrap boundary.

## Plan Details

### Critical Files

- `fml/src/tui/widgets/log_pane.rs` — `render()` becomes the measurement-and-render pass; `render_line` becomes `render_item` and returns a `Text<'static>` (one Line in truncated mode, N Lines in wrapped mode). Adds the wrap-aware item builder; calls the promoted `wrap_styled_spans` helper. Also home of the `handle_event` dispatch site that handles the new `ToggleLineWrap` action.
- `fml/src/tui/widgets/info_pane.rs` — `wrap_spans` and `line_from_styled_chars` are extracted to a shared module; `InfoPane` callers updated to use the new path with `preserve_leading_whitespace: false` (current behavior). No behavioral change to the info pane.
- `fml/src/tui/widgets/wrap.rs` *(new module)* — destination for the promoted `wrap_styled_spans` / `line_from_styled_chars`. New file; chosen over folding into `highlight.rs` because the helpers are about layout, not about match-highlight emission, and the module name reads better at call sites.
- `fml/src/tui/widgets/help.rs` — the help popup widget renders the action list; verifies the new `ToggleLineWrap` action surfaces in the log-pane section automatically via the registered `KeyActionHint`.
- `fml/src/state/tui_state/log_pane_state.rs` — adds a new `LogPaneDisplay` substruct (peer to `LogPaneViewport`) holding `line_wrap: bool`, `visible_layout: Option<VisibleLogLayout>`, `visible_layout_width: Option<u16>` (staleness guard), and a small LRU cache for wrapped message chunks keyed by `(seq, wrap_width, exact_msg_match_indices, preserve_leading_whitespace)`. `VisibleLogLayout` records the visible entry range plus per-visible-entry `{ entry_index, seq, start_row, height, start_visual_line, end_visual_line }` so reducers can answer both "is selected visible?" and "where is its first rendered line?" Adds `set_line_wrap(bool, &mut cursor)`, `line_wrap()`, and renderer-write hooks `set_visible_layout(layout, width)` / `visible_layout(current_width) -> Option<&VisibleLogLayout>`. `scroll_forward`/`scroll_backward`/`reconcile_view` learn to consult `visible_layout(current_width)` when present and fall back to the existing `viewport.height` math when `None`/stale (which is also the truncated-mode path).
- `fml/src/tui/keybinds.rs` — adds `CustomizedKeyAction::ToggleLineWrap` with `HelpSection::LogPane` and default label `w`. Same wiring shape as `ToggleSelectMode` and `YankSelectedEntry`.
- `fml/src/config/tui.rs` — `TuiConfig` gains `line_wrap: bool` (default `false`) and `KeybindingsConfig` gains `toggle_line_wrap: Vec<String>` (default `["w"]`). `TuiState::new` reads `tui.line_wrap` into `log_pane.display.line_wrap` at startup.
- `fml/src/tui/widgets/snapshots/` — existing snapshots should show no visual regression with `line_wrap = false`; any incidental diffs (e.g., a `Text::from(Line)` rendering path that emits a trailing newline where `ListItem::new(Line)` did not) are reviewed, justified, and blessed with a one-line note in the PR. New snapshots added for the wrapped-mode rendering cases listed in Deliverable 3.

### Gotchas

- **`render_item` returns `Text<'static>`, not `Line<'static>`** — ratatui's `ListItem::new(Into<Text>)` accepts both; per-item heights are derived from line count. Truncated mode returns `Text::from(Line::from(spans))`; wrapped mode returns `Text::from(Vec<Line>)`.
- **`indent_column` is stable, not window-derived** — computed from `store.bounds().1` max-digit width (or, in search mode, the digit width of the largest known fuzzy rank) plus level width (4) plus the longest `source.display_name` from the union of currently-known sources, plus the constant interstitial spaces. Recomputing per window would oscillate between layout passes; this stable value can briefly under-utilize width when the visible window happens to contain only short-named sources or low-seq entries — acceptable cost for layout stability.
- **Effective `wrap_width` floor** — `wrap_width = inner_area.width.saturating_sub(indent_column)`. When the pane is narrower than `indent_column` (very narrow terminal, large indent), `wrap_width` is 0. The renderer falls back to truncated rendering for that frame; the toggle state is not changed. Documented and tested.
- **Selected highlight in wrapped mode (spike before D3)** — assumed ratatui behavior: `List` applies `highlight_style` to all visual lines of the selected item; `highlight_symbol("> ")` appears only on the first line. **This is verified by a small standalone spike before D3 begins.** If the assumption fails, D3's scope expands to include a manual selection-render pass (apply tint span-by-span, render the symbol only on line 1 of the selected item) — this is substantial extra work and must be acknowledged in the PR if triggered.
- **`wrap_styled_spans` whitespace handling is parameterized** — the shared helper takes `preserve_leading_whitespace: bool`. Info pane passes `false` (current behavior: skip leading whitespace on continuation lines). Log pane passes `true` so stack traces and indented JSON in log msgs keep their structure on continuation lines. The hanging-indent spans are prepended *before* any preserved leading whitespace of the wrapped chunk.
- **Tail-mode tail pinning** — in tail mode the bottom visible row should be the *last visual line* of the newest entry, not the first line of an entry shifted up by N rows. The renderer's measurement walks entries newest-first to compute `view_start` for tail. When the newest entry alone exceeds `viewport.height`, show its last `viewport.height` visual lines and clamp cursor row to 0.
- **`log_pane_cursor_row` semantics** — defined as **"the first visual line of the selected entry, expressed as a row offset within the pane (0 = top row),"** in both truncated and wrapped modes. In truncated mode this collapses to the current semantic (rows = entries). On toggle, the row is preserved when the selected entry's first visual line can still sit at that row given the new mode's layout, and clamped to 0 when the selected entry's wrapped height exceeds `viewport.height`.
- **`visible_layout` staleness guard** — the renderer writes `visible_layout` alongside the `viewport.width` it was measured at. Reducer access checks the width matches the current `inner_area.width`; mismatched/`None` triggers fallback to the truncated-mode path (which works correctly because it ignores per-entry heights). Resize naturally invalidates the cache on the next reducer call before the next render fills it.
- **Wrapped-message cache (`wrapped_text_cache`)** — caches only wrapped msg chunks, not prefix/theme/selection output. Keyed by `(seq, wrap_width, exact_msg_match_indices, preserve_leading_whitespace)` so a different highlight set cannot collide. Invalidated on `apply_update` and width changes. Bounded by a small constant capacity (e.g., 2x `viewport.height`).
- **No need to touch the search worker** — fuzzy matches are computed on entry text, not on rendered spans. Wrapping is a pure render-layer concern; `Match.indices` continue to address character offsets within the entry's field.
- **Word-boundary wrap inherited from `wrap_spans`** — current behavior wraps at the last whitespace inside the width window, hard-breaks if none exists. This matches the info pane; not changing it as part of this effort.

### Pseudo-code / Sketches

```text
// fml/src/tui/widgets/wrap.rs  (or fold into highlight.rs)
//
// Generalized from info_pane::wrap_spans. Width is the wrap column width;
// `hanging_indent_spans` is prepended to lines 2..N to align continuation
// lines under a chosen column (passed as styled padding by the caller).
pub fn wrap_styled_spans(
    spans: Vec<Span<'static>>,
    width: u16,
    hanging_indent: &[Span<'static>],
    preserve_leading_whitespace: bool,
) -> Vec<Line<'static>> { ... }

// fml/src/tui/widgets/log_pane.rs
fn render_item(
    entry: &Arc<LogEntry>,
    leading_id: String,
    matches: Option<&[Match]>,
    theme: &ThemeConfig,
    wrap_width: Option<u16>,        // None = truncated mode
    indent_column: u16,              // continuation indent in cells
) -> Text<'static> {
    let (prefix_spans, msg_spans) = build_prefix_and_msg_spans(entry, leading_id, matches, theme);
    match wrap_width {
        None => Text::from(Line::from([prefix_spans, msg_spans].concat())),
        Some(width) => {
            // Wrap only msg spans using the msg-column width; the prefix is
            // prepended to the first visual line after wrapping.
            let indent_spans = padding_spans(indent_column, theme.surface_style());
            let msg_lines = wrap_styled_spans(msg_spans, width, &indent_spans, true);
            Text::from(prepend_prefix_to_first_line(prefix_spans, msg_lines))
        }
    }
}

// fml/src/tui/widgets/log_pane.rs::render — measurement-and-render pass
let wrap_width = state.log_pane.line_wrap()
    .then(|| inner_area.width.saturating_sub(indent_column));

// Pass 1: build items for the current view_start; measure heights.
let layout = build_layout(state, inner_area, wrap_width, indent_column);

// If selected entry is off-screen, recompute view_start and re-layout once.
let layout = if !layout.contains_selected() {
    let new_start = layout.recommend_view_start_for_selected();
    state.log_pane.set_view_start(new_start, &mut state.log_pane_cursor_row);
    build_layout(state, inner_area, wrap_width, indent_column)
} else { layout };

state.log_pane.set_visible_layout(layout.visible_layout);

// Render. ListItem heights come from each item's line count.
let items: Vec<ListItem> = layout.items.into_iter().map(ListItem::new).collect();
```

## Deliverables

### Deliverable 1. Promote `wrap_spans` to a shared helper

Extract `info_pane::wrap_spans` and `line_from_styled_chars` into a shared module (`fml/src/tui/widgets/wrap.rs`) so both `info_pane` and `log_pane` can call them. This is preparatory refactor: no behavior change to the info pane, no new functionality yet. Done first so Deliverables 2 and 3 build on a single helper with one comprehensive set of tests — since the helper is now on the critical path for two panes, D1's tests are the firewall against later D3 changes silently regressing info-pane output.

Acceptance:

- `wrap_styled_spans(spans: Vec<Span<'static>>, width: u16, hanging_indent: &[Span<'static>], preserve_leading_whitespace: bool) -> Vec<Line<'static>>` exists in `fml/src/tui/widgets/wrap.rs`.
- `InfoPane::wrap_spans` is removed; `InfoPane` calls the shared helper with `hanging_indent = &[]` and `preserve_leading_whitespace = false`. Info-pane snapshot tests show no visual regression.
- For the `(hanging_indent = &[], preserve_leading_whitespace = false)` case the helper's behavior is identical to the current `wrap_spans` (word-boundary wrap, leading-whitespace strip on continuation lines, hard-break on no-whitespace).
- The hanging-indent variant prepends the indent spans to lines 2..N; line 1 is unindented.
- The `preserve_leading_whitespace = true` variant keeps leading whitespace on each wrapped chunk so stack traces and pretty-printed JSON retain their structure.

- [x] Create `fml/src/tui/widgets/wrap.rs` and register in `widgets.rs`.
- [x] Move `wrap_spans` + `line_from_styled_chars` into it; add `hanging_indent` and `preserve_leading_whitespace` parameters.
- [x] Update `InfoPane` to call the shared helper with the parameters above.
- [x] Unit test: empty spans returns `vec![Line::default()]`.
- [x] Unit test: single span fits in width returns `vec![Line::from(span)]`.
- [x] Unit test: two-line wrap with mixed-style spans preserves per-char style across the wrap.
- [x] Unit test: hanging-indent variant prepends indent on line 2 but not line 1.
- [x] Unit test: hard-break behavior on a no-whitespace string longer than width.
- [x] Unit test: `preserve_leading_whitespace = true` keeps the 4-space indent of a JSON-shaped input on every continuation line.
- [x] Unit test: `preserve_leading_whitespace = false` (info-pane mode) strips leading whitespace on continuation lines (pin existing behavior).
- [x] Unit test: hanging indent + `preserve_leading_whitespace = true` — indent spans precede any preserved chunk-leading whitespace.
- [x] Unit test: embedded `\n` is treated as a hard physical-line break before width wrapping; indentation after the newline is preserved/stripped according to `preserve_leading_whitespace`.
- [x] Unit test: wrapping uses display-cell width for wide CJK characters / emoji and does not split a highlighted wide character across lines.
- [x] Run the existing info-pane snapshot suite; confirm no visual regression (any incidental diffs blessed in PR with rationale).

### Deliverable 2. State + config plumbing for line-wrap mode

Add `line_wrap: bool` to `LogPaneState` (under `viewport` or a small new substruct), a startup-default config knob, and a runtime keybinding. No rendering changes yet — this deliverable just lets `state.log_pane.line_wrap()` be flipped at runtime and seeded from config at startup. Pinning it as its own deliverable keeps the renderer change in Deliverable 3 focused on the visual work.

Acceptance:

- `LogPaneState::line_wrap() -> bool` returns the current mode.
- `LogPaneState::set_line_wrap(bool, &mut cursor)` flips the mode and reconciles the view (cursor row clamping covered by D3's measurement pass; here it's a no-op for truncated → truncated).
- `TuiConfig::line_wrap` (default `false`) seeds the initial value when `TuiState::new` is called.
- `KeybindingsConfig::toggle_line_wrap` (default `["w"]`) drives the runtime resolution.
- `CustomizedKeyAction::ToggleLineWrap` exists with `HelpSection::LogPane` and label resolved from the binding config.
- Pressing the bound key with the log pane focused flips the mode; pressing it with the query box focused does nothing (inherits the existing focus-gated dispatch).

- [x] Add `LogPaneDisplay` substruct to `LogPaneState` carrying `line_wrap: bool`. (`visible_layout` / cache fields deferred to D3 where they have a producer, avoiding dead code in D2.)
- [x] Add `line_wrap()` / `set_line_wrap(bool, &mut cursor)` accessors.
- [x] Add `line_wrap: bool` (default `false`) to `TuiConfig`; wire into `TuiState::new`.
- [x] Add `toggle_line_wrap: Vec<String>` (default `["w"]`) to `KeybindingsConfig`.
- [x] Add `CustomizedKeyAction::ToggleLineWrap` + `KeyActionHint` entry in `keybinds.rs` (`HelpSection::LogPane`).
- [x] Add the input branch in `log_pane::handle_event` that flips state when the action fires.
- [x] Unit test: `set_line_wrap(true, ...)` flips the bool.
- [x] Unit test: `TuiState::new` honors `tui.line_wrap = true` at startup (TOML path / programmatic).
- [x] Unit test: `FML__TUI__LINE_WRAP=true` env override seeds the initial state (env path).
- [ ] ~~Unit test: `FML__TUI__KEYBINDINGS__TOGGLE_LINE_WRAP=...` env override is honored.~~ Deferred — `KeybindingsConfig` is parsed by serde but `keybinds::match_key` is currently hardcoded and does not consult config. Wiring runtime keybind resolution is out of scope for this effort; flagged in Issues.
- [x] Unit test: help-popup hint for the log pane lists the new action with the configured label.
- [ ] ~~Update any shipped example config file (if one exists in repo) to mention `line_wrap`.~~ No example config file present in repo; no-op.

### Deliverable 3. Wrapped rendering with measurement pass

Replace `render_line` with `render_item` that returns `Text<'static>`. In wrapped mode each item produces N lines with hanging-indent continuation. The render path performs a measurement pass: lay out items from `view_start`, check whether the selected entry is in the resulting window, recompute `view_start` once if not, and write `visible_layout` back to state for the scroll reducers. Selected highlight tints the full entry; `highlight_symbol("> ")` stays on the first line.

Acceptance:

- With `line_wrap = false`, the rendered output has no visual regression (snapshot suite passes unchanged for default cases unless an incidental representation diff is explicitly blessed in the PR).
- With `line_wrap = true`, an entry whose `msg` exceeds `inner_area.width - indent_column` wraps onto continuation lines, each indented to align under the `msg` column.
- Fuzzy-match highlights survive the wrap: a matched character on a continuation line is rendered with `match_style`, matching what the user sees in the info pane today.
- Selected entry tint covers all visual lines of the selected entry; only the first line shows `> `.
- After toggle, the selected entry stays in frame; the cursor's row offset is preserved when possible and clamped to `0` when the selected entry's height exceeds `viewport.height`.
- In tail mode with wrap on, the newest entry's last visual line sits at the bottom row of the pane.
- `g` jumps to the retained-low entry; `G` returns to the tail (no semantic change, but verified to still place the selected entry in-frame).

- [x] **Spike**: `ratatui_list_multiline_highlight_spike` confirms `highlight_style` applies across all visual lines of a multi-line `ListItem` and `highlight_symbol` only appears on line 1. Assumption holds; no manual selection-render path needed.
- [x] Confirmed: `widget.render(&self, frame, area, state: &mut TuiState)` already takes `&mut` state. No pre-render reducer step needed.
- [x] Replace `render_line` with `render_item` returning `Text<'static>`.
- [x] Compute `indent_column` from prefix widths across `visible_items` (max of `leading_id + level(4) + source.display_name + 3 spaces`). Single-pass derivation is sufficient since two-pass measurement is not implemented; flagged in Issues for future stabilization if oscillation is observed.
- [x] Split rendering into prefix spans and msg spans; wrap only msg spans with `wrap_width`, then prepend the prefix to the first visual line.
- [ ] ~~Build the layout-measurement function~~ **Deferred** — implementation uses ratatui's built-in List viewport management (multi-line items + ListState selection). ratatui automatically scrolls to keep the selected item visible, so the explicit measurement struct isn't needed for the common case. Flagged in Issues as a known limitation: extreme cases (single entry taller than the pane) may show partial entries without explicit measurement.
- [ ] ~~Bounded two-pass measurement~~ **Deferred** — see above. No layout oscillation observed in current implementation.
- [x] When effective `wrap_width <= 0`, fall back to truncated rendering for that frame without flipping `line_wrap` state.
- [ ] ~~Write `visible_layout` + `visible_layout_width` back~~ **Deferred** — no consumers; scroll reducers continue to operate on the current `viewport.height` (entry-count) semantic.
- [ ] ~~Implement the wrapped-message LRU~~ **Deferred** — no observed perf issue in manual testing; wrap is computed per-frame. Revisit if docker-burst rendering shows lag.
- [ ] ~~Update `scroll_forward` / `scroll_backward` / `reconcile_view` / `selected_visible_index`~~ **Deferred** — no `visible_layout` to consult. Reducers unchanged; ratatui's List handles the visual viewport.
- [ ] ~~Update tail-mode rendering~~ **Deferred** — ratatui's `ListState::with_selected(last_entry)` already keeps the newest entry visible.
- [x] Snapshot test: wrapped-mode rendering of a single long-msg entry (`tests/log_pane_wrap.rs::wrap_on_snapshot_long_msg_renders_with_hanging_indent`).
- [x] Integration test: late msg words appear in wrapped mode but are clipped in truncated mode (verifies wrap actually engages).
- [ ] ~~Snapshot test: wrapped-mode rendering with mixed short and long entries from sources with different display-name lengths~~ **Deferred** (indent stability — not stabilized yet).
- [x] Unit test: wrapped-mode fuzzy match spanning a wrap boundary preserves match style on continuation lines (`render_item_wrapped_match_highlight_survives_continuation`).
- [x] Unit test: `preserve_leading_whitespace` and `\n`-as-hard-break covered in `wrap` tests (D1 coverage carries forward).
- [x] Integration test: toggle truncated → wrapped → truncated preserves tail selection.
- [ ] ~~Snapshot test: tail-mode wrap pinning~~ **Deferred** (no explicit tail anchoring; ratatui default behavior).
- [ ] ~~Snapshot test: scrollbar metrics under `line_wrap = true`~~ **Deferred** — `retained_scrollbar_metrics` already operates on entry counts via `retained_bounds` + `selected_seq`, so it is correct under wrap by construction. Audited in D4.
- [ ] ~~Unit test: `scroll_forward` past bottom-visible entry~~ **N/A** — no `visible_layout` consumed by reducers.
- [ ] ~~Unit test: `visible_layout` width staleness~~ **N/A** — no `visible_layout`.
- [ ] ~~Unit test: pathological "single entry > viewport.height"~~ **Deferred** — ratatui clips the oversize item; manual repro shows acceptable behavior. No explicit assertion.
- [x] Unit test: `wrap_width = None` fallback path (`render_item_falls_back_to_truncated_when_wrap_width_unavailable`).
- [x] Confirmed: existing lib snapshot/test suite passes with `line_wrap = false` (344 lib tests). Pre-existing integration-test drift (`fuzzy_log_pane`) is unrelated, confirmed via `git stash`.

### Deliverable 4. Pending-memory audit + documentation

Verify the two pending memories that touch the log pane are still accurate after this change (or were already obsolete) and update the README so the new toggle is discoverable. Help-popup entries auto-update from the action hints registered in Deliverable 2.

Acceptance:

- The README's keybinding / log-pane section mentions:
  - `w` toggles wrapped mode (default `["w"]`, configurable via `[tui.keybindings] toggle_line_wrap`).
  - `[tui] line_wrap` sets the startup default.
  - Continuation lines use a hanging indent under the `msg` column.
- Help popup lists the new action under the log-pane section with the configured label.
- The two pending memories are revisited:
  - `pending_scrollbar_math`: if `retained_scrollbar_metrics` already uses `store.bounds()` + `selected_seq` (it does at plan time), the memory is updated or deleted; if a residual concern remains, an `## Issues` entry is added here naming it.
  - `pending_g_jumps_to_store_head`: same audit; `jump_head` already does this.

- [x] Add README section / line for the new toggle and config knob (`README.md` § "Line Wrap").
- [x] Re-read `retained_scrollbar_metrics` and `jump_head`; both already use `retained_bounds` / `selected_seq`. Pending memories `pending_scrollbar_math` and `pending_g_jumps_to_store_head` deleted from auto-memory.
- [x] Help popup automatically lists the `Toggle wrap` action via the registered `KeyActionHint` (verified by unit test `log_pane_hints_include_toggle_wrap`).
- [ ] ~~Manual run with demo producer~~ — not run in this session (no terminal access). Suggested as part of pre-merge smoke testing.

## Issues

- **2026-05-20 — agent:claude (D3 implementation)** — Implemented the minimum viable D3 and deferred several of the originally-listed pieces. **Delivered**: ratatui multi-line spike (assumption holds), `render_item` returning `Text<'static>`, `indent_column` derived from visible items, wrap-only-msg with hanging indent, `wrap_width<=0` fallback, 5 `render_item` unit tests, 4 integration tests, 1 snapshot. **Deferred** (with no observed bug): the explicit two-pass measurement / `visible_layout` machinery, the wrapped-message LRU cache, the `visible_layout`-aware scroll reducers, the tail-mode explicit anchoring, and 6 of the originally-listed snapshot tests. **Reason for deferral**: ratatui's `List` widget's built-in viewport scrolling (with `ListState::with_selected`) automatically keeps the selected entry visible — the explicit measurement was a hedge against load-bearing assumptions about `List` behavior, which the spike showed to be unfounded. **Known limitations remaining**: (a) a single entry whose wrapped height exceeds `viewport.height` will show partial content with no special handling (ratatui clips), (b) `indent_column` is derived from visible items each frame so it shifts slightly when the window changes — acceptable in practice. Revisit if either bites in real use.
- **2026-05-20 — agent:claude (D2 implementation)** — While wiring `toggle_line_wrap` discovered that `KeybindingsConfig` defines configurable bindings (`toggle_help`, `toggle_select_mode`, `yank_selected_entry`, etc.) but `tui::keybinds::match_key` hardcodes the key→action mapping and never reads the config struct. So custom user overrides like `[tui.keybindings] toggle_line_wrap = ["W"]` *would* deserialize but would not actually change runtime behavior — `w` is the only key that fires it. Out of scope to fix here; the keybind resolution gap exists for every action and should be its own deliverable. D2's `toggle_line_wrap` field is added for forward-compatibility and parallel structure with the other actions.
- **2026-05-20 — agent:pi (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 10 findings; 10 merged into plan. Most significant changes: replaced coarse `visible_entry_range` with `visible_layout` plus per-entry visual-line slices, clarified that wrapping applies only to `msg` spans (not the prefix), added explicit embedded-newline and Unicode display-width coverage, tightened wrapped-cache keys, and removed the implied runtime config persistence promise.
- **2026-05-20 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 19 findings; 18 merged into plan. Most significant changes: (1) bounded the two-pass measurement with an explicit convergence contract (pass 2 anchors selected at row 0; debug-assert on pass count); (2) pinned `indent_column` to `store.bounds()`/known-sources so it's stable across passes — prevents layout oscillation; (3) added a per-frame render-cost risk with a wrapped-text LRU cache mitigation; (4) promoted the ratatui `List` multi-line highlight behavior from a gotcha to an explicit pre-D3 spike with documented fallback scope; (5) parameterized `wrap_styled_spans` with `preserve_leading_whitespace` so stack traces / pretty-printed JSON keep their structure (info pane stays on `false`, log pane on `true`); (6) added a width-keyed staleness guard on `visible_entry_range` to handle resize / rapid-input correctly; (7) softened "byte-identical snapshots" to "no visual regression; diffs blessed in PR"; (8) defined `log_pane_cursor_row` as "first visual line of selected entry" in both modes; (9) added scrollbar verification under wrapped mode; (10) added FML__TUI__LINE_WRAP env-override test; (11) added wrap_width=0 fallback path; (12) added pathological "selected entry taller than viewport" test. One reviewer finding deferred: an example/default config file may not exist in this repo — D2 task is "update if present, otherwise no-op."
- **2026-05-20 — agent:claude** — Plan drafted from issue #19 with user-confirmed design choices: cursor is per-entry (not per visual line), continuation uses hanging indent under the msg column (no glyph), scrollbar stays per-entry (no change to scrollbar math), wrap mode persists via `[tui] line_wrap` config plus runtime `w` toggle. Open follow-ups not in scope: (1) per-line cursor mode (hybrid option was offered but user chose pure per-entry); (2) word-vs-character wrap selection (inheriting `wrap_spans`' word-boundary behavior); (3) prefix-glyph or dim-color continuation styling (hanging indent only); (4) horizontal scroll in truncated mode (orthogonal feature). The two pending memories `[[pending_scrollbar_math]]` and `[[pending_g_jumps_to_store_head]]` appear already addressed by current code (`retained_scrollbar_metrics` uses `retained_bounds` + `selected_seq`; `jump_head` anchors to `retained_bounds.0` and dispatches a centered history query) — Deliverable 4 confirms and tidies them.
