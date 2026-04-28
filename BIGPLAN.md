# BIGPLAN: Source Selector Popup

## Plan Overview

The source selector popup lets users narrow visible logs by producer, group, or individual source. Done means the popup opens from a hardcoded `ctrl+s` keybinding, presents the long-term `Producer -> Group -> Display Name` hierarchy clearly, supports keyboard navigation and toggling at every level, and applies the selected source set consistently to tail, history, fuzzy, and preview-adjacent searches. This is a production filtering mechanism developed while demo sources are the primary test fixture; acceptance criteria are the same regardless of which producers are running.

## Risks

- **Filter semantics drift** - Empty `sources` on the wire (`SearchEvent::Search.sources: Vec<SourceId>`) means "all sources." The popup must distinguish all/none/partial without overloading that convention. **None-selected is handled at the UI boundary, not on the wire**: when the conversion helper sees an empty enabled set, the popup cancels any in-flight log-pane query, does *not* dispatch a new search, and renders a "no sources selected" empty state in the log pane. The wire format is never changed; only the explicit list and the wildcard `[]` are ever emitted.
- **Snapshot scope and wildcard semantics** - The popup tree is rendered from an open-time snapshot of `producer_state.sources` taken on popup open; the tree is stable mid-session (sources arriving or disappearing while the popup is open are not reflected). The persistent enabled-source set lives in `TuiState` across popup sessions. The conversion helper that produces `SearchEvent::Search.sources` compares the enabled set against the **live** `producer_state.sources` at dispatch time: enabled set ⊇ live → `[]` wildcard; otherwise → explicit sorted list (intersected with live). This prevents a new source arriving after popup-open from silently appearing in `[]` results — the wildcard only emits when the user's enabled set already covers everything currently live. Upgrading the tree projection to live data later is a localized D3 change and does not touch the conversion helper interface.
- **Between-session source churn** - The persistent enabled set must stay in sync with `producer_state` events even while the popup is closed: `ProducerEvent::SourceFound` adds the new ID to enabled (preserves the historical "all sources visible" default); `ProducerEvent::SourceLost` removes it. Without this, enabled drifts away from live and the conversion helper either over-filters (excludes new sources the user expected to see) or accumulates phantom IDs.
- **Popup focus conflicts** - Global keys, widget-local keys, and reserved fallbacks already share the input path. Popup-local handling must run before focused-widget handling so navigation/toggles do not leak into the log pane or query box.
- **Config-loadable keybindings deferred** - `KeybindingsConfig` exists in the config struct for TOML deserialization but `resolve()` was removed with `keybindings.rs`. The popup will use a hardcoded `ctrl+s` added as `CustomizedKeyAction::ToggleSourceSelector` in `keybinds.rs`. Wiring user-remappable bindings for the source selector is future work; any deliverable that needs it must rebuild the configurable infrastructure in `keybinds.rs` first.
- **Hierarchy ambiguity** - `Source::producer` is the top-level selector label, but only some backends need producer-specific labels; Kubernetes should likely use namespaces, while Docker compose/project names belong in `Source::group`. Use stable source IDs internally, render missing groups under a predictable label, and treat producer/group/display names as labels only.
- **Search refresh churn** - Toggle reflects in the checkbox UI synchronously. The actual search redispatch reuses the existing query-box debounce window (same constant) so rapid `space` presses coalesce into one search. Without this the popup will spawn N searches for N keystrokes.
- **Popup key precedence vs globals** - When the popup is open, only its local keys (`ctrl+s`, `esc`, `up`/`k`, `down`/`j`, `space`, `enter`, `a`, `n`) are consumed by the popup. Global fallbacks such as `ctrl+c` (quit) must remain active and run before popup-local handling. All other keys are swallowed by the popup so they do not leak to the underlying focused widget. Without an explicit precedence rule, popup keys leak into the query box / log pane and quit may be blocked.

## Plan Details

### UX Contract

The popup is a centered overlay, not a new focusable layout pane. Opening it preserves the underlying focused pane; closing it returns users to that pane. The tree uses tri-state checkboxes for producer and group rows:

- `[x]` means all descendant sources are enabled.
- `[ ]` means no descendant sources are enabled.
- `[-]` means some descendant sources are enabled.

Recommended local keys:

- `ctrl+s` opens or closes the popup (hardcoded via `CustomizedKeyAction`; user-remappable keybindings are future work).
- `esc` closes the popup without rolling back any toggles (immediate-apply; no staged cancel).
- `up` / `k` and `down` / `j` move the popup cursor.
- `space` toggles the highlighted producer, group, or source.
- `enter` closes the popup (same as `esc` under immediate-apply).
- `a` enables all sources, `n` disables all sources.

Toggles update the checkbox UI synchronously. Search redispatch is debounced at the same window used by the query box so rapid toggles coalesce into one search. Disabling all sources is allowed; the popup cancels the in-flight log-pane query and the log pane shows an explicit "no sources selected" empty state (not an all-sources fallback). `esc` is a simple close, not a rollback.

Global fallbacks like `ctrl+c` (quit) remain active while the popup is open. Only the keys listed above are consumed locally; all other keys are swallowed by the popup and do not leak to the underlying focused pane.

The popup tree scrolls when the number of rows exceeds the available height. Maximum popup height is approximately 80% of terminal rows; rows outside the visible window are clipped. The cursor moves through all rows and the visible window follows. No scrollbar is rendered — keep it simple. At narrow terminal widths a simplified layout is used (drop the count column, condense the footer) as shown in the small terminal mockup above. If the tree is too tall for even the fallback layout, the same cursor-follows scrolling applies — no scrollbar. The footer shows only the keys that are implemented; future keys such as `/` are added when that work lands.

Producer naming should follow the source collection boundary users recognize:

- Kubernetes: namespace labels such as `namespace1` or `namespace2`.
- Docker: top-level producer label `docker`; compose/project labels such as `docker-compose group 1` and `ungrouped` live under `group`.
- File: usually just `file`, with path/category details handled by group and display name.

### ASCII Mockups

Default open state with all demo sources enabled:

```text
+--------------------------------------------------------------------------+
| Log Pane                                          | Info Pane             |
| ...                                              | ...                   |
|                                                  |                       |
|                  +-- Sources ----------------------------------+         |
|                  | Filter visible logs by source                |         |
|                  |                                              |         |
|                  | > [x] fake                         3/3       |         |
|                  |     [x] backend                   2/2       |         |
|                  |       [x] Service A                         |         |
|                  |       [x] Service B                         |         |
|                  |     [x] frontend                  1/1       |         |
|                  |       [x] Service C                         |         |
|                  |                                              |         |
|                  | space toggle  a all  n none  esc close      |         |
|                  +----------------------------------------------+         |
| > query                                                                  |
+--------------------------------------------------------------------------+
```

Partial producer/group state after disabling one source:

```text
+-- Sources ------------------------------------------------------+
| Filter visible logs by source                                  |
|                                                                |
|   [-] fake                                           2/3        |
|   > [-] backend                                     1/2        |
|       [ ] Service A                                            |
|       [x] Service B                                            |
|     [x] frontend                                    1/1        |
|       [x] Service C                                            |
|                                                                |
| 1 source hidden. Tail/history/fuzzy searches use enabled IDs.   |
+----------------------------------------------------------------+
```

Toggling a group applies to children:

```text
Before space on "backend"             After space on "backend"

[-] fake                    2/3       [x] fake                    3/3
> [-] backend               1/2       > [x] backend               2/2
    [ ] Service A                         [x] Service A
    [x] Service B                         [x] Service B
  [x] frontend              1/1         [x] frontend              1/1
    [x] Service C                         [x] Service C
```

Multiple real producer/group shapes:

```text
+-- Sources ------------------------------------------------------+
| > [-] file                                           5/8        |
|     [x] /var/log                                     3/3        |
|     [-] app                                          2/5        |
|   [x] docker                                         4/7        |
|     [x] docker-compose group 1                       4/4        |
|       [x] api                                        2/2        |
|       [x] web                                        2/2        |
|     [ ] ungrouped                                    0/3        |
|       [ ] standalone containers                      0/3        |
|   [-] namespace1                                     6/10       |
|     [-] deployments                                  4/7        |
|     [x] jobs                                         2/3        |
|   [ ] namespace2                                     0/4        |
|     [ ] deployments                                  0/4        |
|                                                                |
| Source names may repeat; source IDs stay hidden unless needed. |
+----------------------------------------------------------------+
```

Small terminal fallback:

```text
+-- Sources ----------------------+
| > [-] fake              2/3     |
|     [-] backend         1/2     |
|       [ ] Service A             |
|       [x] Service B             |
|     [x] frontend        1/1     |
|       [x] Service C             |
| space toggle   esc close        |
+---------------------------------+
```

### Interaction Sketch

```text
on ctrl+s:
  if popup is closed:
    snapshot current producer_state.sources as open-time tree projection
    open popup with cursor on first producer
  else:
    close popup
  (snapshot is for UI tree rendering only; the conversion helper below
  always reads live producer_state.sources at dispatch time)

on ProducerEvent::SourceFound (popup open or closed):
  add source ID to persistent enabled set (preserves "see all" default)
  if popup is open: snapshot is unchanged, new source is invisible until reopen

on ProducerEvent::SourceLost (popup open or closed):
  remove source ID from persistent enabled set
  if popup is open: snapshot is unchanged, lost source still appears in tree

on popup up/down:
  move cursor within visible tree rows (scroll window follows)

on popup space:
  if row is source:    flip that source ID in enabled set
  if row is group:     set all child source IDs to inverse of current aggregate
  if row is producer:  set all descendant source IDs to inverse of current aggregate
  recompute aggregate checkbox states
  schedule log-pane search redispatch (debounced, same window as query box)

dispatch_log_pane_search(query):
  let live = current producer_state.sources
  if enabled is empty:
    cancel in-flight log-pane query
    render "no sources selected" empty state in log pane
    do not emit SearchEvent::Search
  else if enabled superset of live:
    emit SearchEvent::Search { sources: [], ... }    # wildcard
  else:
    emit SearchEvent::Search { sources: (enabled & live).sorted_by_tree_order(), ... }
```

### Critical Files

- `README.md` - Tracks TODO item 3 and the acceptance criteria that this plan expands.
- `fml/src/log.rs` - Owns `Source { producer, id, display_name, group }`, where `producer` is the top-level selector label and `group` captures second-level buckets such as Docker compose projects.
- `fml/src/state/producer_state.rs` - Stores known sources from producer events; the popup tree should derive from this state.
- `fml/src/state.rs` - Owns `AppState`, widget registration order, and the shared state needed by rendering and event dispatch.
- `fml/src/state/tui_state.rs` - Likely home for popup open/closed state, cursor, enabled source IDs, open-time source snapshot, and active filter helpers.
- `fml/src/tui/keybinds.rs` - Owns `StaticKeyAction`, `CustomizedKeyAction`, and `match_key()`; add `CustomizedKeyAction::ToggleSourceSelector` here.
- `fml/src/config/tui.rs` - `KeybindingsConfig` handles TOML deserialization; `resolve()` is absent until configurable keybindings are rebuilt.
- `fml/src/tui.rs` - App-level TUI input dispatch where popup-local handling should take precedence.
- `fml/src/tui/widgets/` - Popup rendering should probably live as a focused widget module, while the existing panes keep rendering underneath.
- `fml/src/tui/widgets/query_box.rs` - Dispatches tail/fuzzy searches and currently sends `sources: Vec::new()`.
- `fml/src/tui/widgets/log_pane.rs` - Dispatches history/head/tail actions and selection changes that may need source filter propagation.
- `fml/src/app.rs` - Starts initial tail search with all sources and provides current demo sources for UX/test fixtures.
- `fml/src/search.rs` and `fml/src/search/*` - Existing search paths already accept source filters; tests should prove every mode honors the selected IDs.

### Gotchas

- `Source::group` is `Option<String>`; render missing groups as a stable bucket such as `(ungrouped)` without storing that label back into `Source`.
- Avoid forcing every backend into the same producer-specific naming rule. Kubernetes should likely use namespace names as top-level producers; Docker should use `docker` as producer and put compose/project names or `ungrouped` in `Source::group`; file sources can use `file`.
- Empty enabled set is a UI-level intercept: cancel the in-flight log-pane query and render a "no sources selected" empty state. Never dispatch `SearchEvent::Search` with an empty `sources` (that means "all sources" on the wire). Wire format is unchanged; only `[]` (wildcard) and explicit non-empty lists are ever sent.
- `ctrl+s` may be intercepted by some terminals as XOFF flow control. Document the conflict and note it in Deliverable 6 so users know to remap at the terminal level until configurable keybindings land.
- `LogPaneState::active_query: SearchKind` is just an enum tag — it does not carry the fuzzy term or history anchor. Source-filter redispatch needs a payload-bearing form (e.g. `active_query: Option<Query>` or a separate `last_dispatched: Option<Query>`). Update all `on_search_started` call sites in D5; this is a structural change, not a pure read.
- Filtering out the source whose log is currently selected/previewed is handled by the existing `TuiEvent` selected-entry cascade: when the visible log set no longer contains the selection, log-pane selection moves (or clears), which emits a selected-entry event that drives preview/info panes to their respective empty states. No separate preview wiring is needed in D5.
- The popup tree cursor index tracks position within the *visible scroll window*, not the absolute row index in the full tree. Scroll-aware navigation must keep these in sync.
- Snapshot tests should cover narrow widths because the popup has dense tree rows and footer help text.
- Deliverable 3 and 4 tests are state/UI-only; end-to-end filter propagation (queries actually using selected source IDs) is not verified until Deliverable 5 wires `query_box.rs` and `log_pane.rs`.

## Deliverables

### Deliverable 1. UX Contract

This deliverable settles the user-visible popup behavior before code changes. It should produce an agreed interaction model for hierarchy, tri-state display, cursor movement, toggle behavior, empty selection semantics, scroll behavior, and small-terminal fallback. No implementation deliverable should begin until all tasks here are checked.

- [x] Draft initial ASCII popup mockups in `BIGPLAN.md`
- [x] Decide whether toggles apply immediately or are staged behind apply/cancel — **immediate apply**
- [x] Refine producer naming so Kubernetes namespaces can be producers while Docker compose/project names are groups
- [x] Decide whether disabling every source is allowed — **yes; log pane shows explicit "no sources selected" empty state**
- [x] Define narrow-terminal fallback — **simplified layout (drop count column, condense footer); scrolling applies if tree overflows even the fallback height, no scrollbar**
- [x] Confirm the footer key set — **show only implemented keys; future keys like `/` are added when that work lands**
- [x] Decide preview pane scoping — **surrounding mode is always single-source scoped; future multi-source preview modes handle their own scoping when implemented**
- [x] Update `README.md` TODO wording if UX decisions change the acceptance criteria

### Deliverable 2. Popup Keybinding And State

*Depends on: Deliverable 1 (all tasks complete)*

This deliverable adds a hardcoded `ctrl+s` source selector action and enough TUI state to open, close, and navigate the popup without changing search results yet. The keybinding is added as `CustomizedKeyAction::ToggleSourceSelector` in `keybinds.rs` and matched in `match_key()`; config-loadable remapping is out of scope for this deliverable. It should keep reserved keys and existing pane focus behavior intact.

- [x] Add `CustomizedKeyAction::ToggleSourceSelector` to `keybinds.rs` and match `ctrl+s` in `match_key()`
- [x] Add popup state to `TuiState`: open/closed flag, cursor index, persistent enabled-source set, open-time source snapshot (used for tree rendering only)
- [x] Subscribe persistent enabled set to `ProducerEvent::SourceFound`/`SourceLost` so it stays in sync with `producer_state` while popup is closed (Found → add to enabled, preserves "see all" default; Lost → remove)
- [x] Define key precedence: when popup is open, global fallbacks (e.g. `ctrl+c` quit) run first; popup-local keys (`ctrl+s`, `esc`, `up`/`k`, `down`/`j`, `space`, `enter`, `a`, `n`) are consumed by the popup before focused widgets see them; all other keys are swallowed (no leak to underlying pane)
- [x] Add unit tests for open/close behavior, focus preservation, key swallowing, and `ctrl+c`-still-quits while open

### Deliverable 3. Source Tree Rendering And Navigation

*Depends on: Deliverable 2*

This deliverable renders the centered popup from known producer sources and supports keyboard navigation over tree rows. It should derive all display rows from `producer_state.sources`, sort them predictably, and never use display names as identity keys. Tests here are state/UI-only; end-to-end filter propagation is covered by Deliverable 5.

- [x] Build a tree projection from the **open-time snapshot** of `producer_state.sources` into producer, group, and source rows; the tree is stable mid-session (sources arriving or disappearing during the popup are not reflected until the next open). Future upgrade to live projection is a localized change here
- [x] Represent aggregate checkbox states as checked, unchecked, and mixed
- [x] Render a centered overlay with bounded width; implement cursor-follows scroll window with max height (~80% of terminal rows), no scrollbar
- [x] Implement narrow-terminal fallback layout (drop count column, condense footer); apply the same scrolling if the tree overflows fallback height
- [x] Move the cursor through visible rows with `up`/`k` and `down`/`j`; scroll window follows cursor
- [x] Add snapshot tests for all-enabled, partial, multiple-producer, ungrouped, narrow-width, and taller-than-viewport layouts

### Deliverable 4. Toggle Semantics

*Depends on: Deliverable 2*

This deliverable makes source, group, and producer rows change the enabled source set. It should handle newly discovered sources predictably and keep aggregate checkbox states correct after every toggle.

- [x] Toggle individual source IDs with `space`
- [x] Toggle group rows by applying the target state to all child source IDs
- [x] Toggle producer rows by applying the target state to all descendant source IDs
- [x] Confirm between-session policy from D2 also covers in-popup arrivals: a source discovered while the popup is open is added to the persistent enabled set (default-enabled) but does not appear in the tree until next open. Document this in code comments since the snapshot/live split is non-obvious
- [x] Implement Option B empty-selection at the UI boundary: allow disabling all sources; cancel the in-flight log-pane query, do not dispatch a new `SearchEvent::Search`, render "no sources selected" empty state. Wire format (`SearchEvent::Search.sources: Vec<SourceId>`) is unchanged
- [x] Add tests for source, group, producer, mixed-state, all-disabled, source-loss, in-popup-arrival, and reopen-after-arrival cases

### Deliverable 5. Search Filtering Integration

*Depends on: Deliverables 2, 3, 4*

This deliverable wires the enabled source set into every log-pane search dispatch. Tail, history, and fuzzy modes should all use the same conversion helper so empty/all/partial selection behavior cannot diverge by mode.

- [x] Replace `LogPaneState::active_query: SearchKind` with a payload-bearing form (e.g. `active_query: Option<Query>` or a separate `last_dispatched: Option<Query>`) so source-filter redispatch can reconstruct tail / fuzzy term / history anchor without guessing. Update all `on_search_started` call sites
- [x] Add a conversion helper `dispatch_log_pane_search(query)` that compares the persistent enabled set against **live** `producer_state.sources`: enabled empty → cancel in-flight query, no dispatch, render empty state; enabled ⊇ live → emit `sources: []` (wildcard); else → emit `(enabled & live).sorted_by_tree_order()`
- [x] Apply debounce to source-filter redispatch using the same constant as the query-box debounce
- [x] Redispatch the current log-pane query (reconstructed from the payload-bearing `active_query`) when the enabled source set changes
- [x] Update initial tail, query box tail, query box fuzzy, log-pane history, and log-pane tail/head dispatches to route through the conversion helper
- [x] Leave preview pane surrounding searches single-source scoped (unaffected by source-filter changes); rely on the existing `TuiEvent` selected-entry cascade to drive preview/info panes to empty when the log pane has no selection
- [x] Add tests proving tail, history, and fuzzy searches receive the intended source IDs; include a test for none-enabled → empty log pane path; include a test for "enabled ⊇ live with new arrival mid-session" → still emits explicit list, not `[]`

### Deliverable 6. Documentation And Cleanup

*Depends on: Deliverable 5*

This deliverable updates user-facing docs and removes temporary ambiguity introduced during the feature. It should leave TODO item 3 either checked off or with only explicitly deferred sub-items.

- [x] Document the `Producer -> Group -> Display Name` hierarchy as the long-term model
- [x] Document the source selector keybinding and the `ctrl+s`/XOFF terminal conflict, noting remapping is not yet user-configurable
- [x] Update README TODO item 3 checkboxes as implementation lands
- [x] Run focused unit, snapshot, and integration tests for popup and search filtering

## Issues

- **2026-04-28 — calamity-m** — Removed `keybindings.rs` wholesale (it was dead code; `resolve()` was never called). New popup key goes into `keybinds.rs` as `CustomizedKeyAction::ToggleSourceSelector`. Config-loadable remapping is deferred and noted as a risk. D2 renamed and simplified accordingly.
- **2026-04-28 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). ~14 findings merged into plan. Most significant: scroll model was undefined; empty-selection had no implementation tasks (Option B chosen — allow all-disabled, show explicit empty state); legacy keybind interception promoted to Risk; `active_query` payload reconstruction surfaced as a prerequisite audit in D5; immediate-apply decision formally closed.
- **2026-04-28 — agent:claude (final adversarial pass)** — Critical gaps closed: (1) snapshot scope clarified — snapshot is for UI tree rendering only, conversion helper compares enabled set against **live** `producer_state.sources` at dispatch time, eliminating the silent-wildcard-collapse risk; (2) between-session source churn handled by subscribing the persistent enabled set to `ProducerEvent::SourceFound`/`SourceLost` (new task in D2); (3) none-selected mechanism committed to UI-intercept (cancel + empty state, wire format untouched); (4) `active_query` data-model change scoped explicitly in D5 (replace `SearchKind` tag with payload-bearing form); (5) preview/selection cascade noted as already handled by `TuiEvent` selected-entry emission, no extra D5 wiring; (6) popup key precedence rule added (globals like `ctrl+c` always run, popup-local keys consumed, others swallowed); (7) snapshot vs live tree projection explicitly leaves the upgrade path open as a localized D3 change.
- **2026-04-28 — open question, resolved** — Preview pane scoping: surrounding mode is single-source scoped and is unaffected by source-filter changes. Future multi-source preview modes will own their own scoping when implemented.
- **2026-04-28 - agent:codex** - Corrected producer naming refinement: Docker should remain a `docker` producer with compose/project names and `ungrouped` at the group level; Kubernetes namespaces remain the main specific top-level producer example.
- **2026-04-28 - agent:codex** - Refined producer naming model: top-level producer rows should be specific instance labels such as Kubernetes namespaces, Docker compose/project groupings, Docker `ungrouped`, or `file`, not generic backend categories.
- **2026-04-28 - agent:codex** - Initial `BIGPLAN.md` created for TODO 3 with ASCII UX mockups. Open UX decisions: immediate vs staged apply, whether zero selected sources is valid, and the version-one footer key set.
