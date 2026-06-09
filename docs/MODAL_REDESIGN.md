# FML Modal TUI Redesign

The TUI is redesigned around one mental model: **fml is a modal editor opened
on a live log buffer**. The interaction grammar is vim/helix; the "file" is
the shared `RingBufferStore`; every pane is an independent viewport over that
buffer with its own source filter, search, and cursor.

## Why redesign

The previous TUI was a fixed grid of widget slots (query box, log pane, info
pane, preview pane) with popups layered on top. Focus, filtering, and search
were hard-wired to exactly two search targets (`LogPane`, `PreviewPane`).
Sporadic issues traced back to that shape: event routing depended on which
slot was focused and which popup was open, selected-entry state was broadcast
between widgets via events, and the preview/field-picker flows each carried
their own little state machines. There was no way to express "two views over
the same logs" — which is the core triage motion.

## The target workflow

1. Launch fml → logs stream in, the single pane **follows the tail**.
2. Something interesting scrolls past → press any motion (or `Esc`) to drop
   out of follow; you are now in NORMAL mode on a stable buffer position.
3. `/` to fuzzy-search the pane's filtered view; matches show as a grep-style
   results view; `Enter` confirms, `Enter` on a hit jumps back into the
   stream centered on that entry; `n`/`N` walk hits in context.
4. Found a thread worth keeping? `:vs` (or `Ctrl-w v`) — the pane is cloned
   into a split. Keep the anchor in one split, keep digging in the other,
   each with its own `:filter`, search, and cursor.
5. Need a separate investigation? `:tabnew` gives a fresh workspace; `gt` /
   `gT` switch tabs.
6. `v` selects exactly the characters you want (a request id, a token) with
   `h`/`l`/`w`/`b`/`0`/`$`; `V` selects whole lines; `y` copies via OSC52.
   `y` in NORMAL yanks the cursor entry as JSON. What you see is what you
   yank: selection and copy both read the same rendered row text.

## Architecture

### Kept (unchanged or near-unchanged)

- `RingBufferStore` — monotonic seq ids, ring retention.
- Search workers (`tail`, `history`, `fuzzy`, `field_matched`) and the
  latest-wins-per-target contract.
- Producers (fake/file/docker/kubernetes), normalizers, source model.
- `App::event_loop` — the single reducer loop over TUI/search/producer
  channels.
- Config loading, themes, clipboard (OSC52).

### Replaced

- All widget/slot/popup machinery (`tui/widgets`, `tui/layout`,
  `tui/keybinds`, `state/tui_state`) is deleted.
- `SearchTarget` is now **pane-addressed**: `SearchTarget::Pane(PaneId)`.
  Panes are created and destroyed freely; each owns exactly one search
  engine slot. Results for dead panes are dropped on routing.
- Input handling is a **mode machine**, not per-widget `handle_event`.

### State model

```text
AppState
├── workspace: Workspace
│   ├── tabs: Vec<Tab>, active_tab
│   ├── mode: Mode (Normal | Visual{anchor} | Search | Command)
│   ├── pending: count / g / z / Ctrl-w prefix state
│   ├── prompt: line editor for / and :
│   └── Tab
│       ├── tree: Node = Leaf(PaneId) | Split{axis, [Node; 2]}
│       ├── panes: Vec<Pane>, focused: PaneId
│       └── Pane
│           ├── filter: Vec<String> (patterns matched against source
│           │   id/name/group/producer; resolved to SourceIds at dispatch)
│           ├── view: Stream{entries} | Results{entries, matches}
│           ├── active query (Tail / History / Fuzzy)
│           ├── cursor_seq, follow: bool
│           └── hits: Vec<u64> (confirmed search, drives n/N)
├── store, producer (sources), search (per-target engine slots), event_bus
```

**Mode is global; follow is per-pane.** "Tail mode" in the UI is simply
"focused pane has `follow = true` and mode is Normal" — shown as `TAIL` in
the status line. Any cursor motion breaks follow; `F` (or `:tail`) restores
it. This means a split can sit in tail mode while you investigate in another
— which is the point of splits.

**Cursor is seq-anchored.** The cursor is a store sequence id, not a row
index. Result application clamps it to the nearest retained entry, so ring
eviction and live appends never make the cursor "jump rows".

**Stream windows are demand-paged.** A pane in stream view holds a window of
entries (Tail or History query). Motion near the window edge redispatches a
`History` query centered on the cursor; workers re-emit on their tick so the
window stays fresh as the ring evicts. `gg`/`G` jump to the retained bounds.

**Search is per-pane grep + jump.** `/` live-dispatches `Query::Fuzzy` to
the focused pane's engine (filtered by its sources). The pane shows the
match list (seq-ordered, highlighted). `Enter` confirms and records hit
seqs; `Enter` on a hit re-enters stream view centered there; `Esc` abandons
and restores the stream. `n`/`N` jump the stream cursor between recorded
hits.

### Chrome

Panes are borderless, text-editor style: a one-column `│` gutter separates
side-by-side splits, and every pane carries a vim-style reversed statusline
(`[filter] /term TAIL … seq/high`) as its bottom row — stacked splits are
separated by those statuslines alone. The global bottom line is the cmdline:
mode badge, `/` and `:` prompts, notices, store stats, pending keys. Only
the detail/help overlays use borders.

### Event flow

```text
crossterm key ──TuiEvent::Input──▶ reducer
  keymap(mode, pending, key) → Action
  Action mutates Workspace; may dispatch SearchEvent::Search{Pane(id), ...}
SearchEvent::Result{Pane(id)} ──▶ route to pane (any tab) → pane applies
ProducerEvent::SourceFound/Lost ──▶ producer.sources; panes with pattern
  filters redispatch so worker source snapshots stay correct
Render tick ──▶ recursive split layout → per-pane draw → status/prompt line
```

## Keymap (v1, hardcoded)

| Mode | Keys | Action |
|---|---|---|
| NORMAL | `j` `k` (count ok), `Ctrl-d`/`u`, `Ctrl-f`/`b` | move cursor / half / full page |
| NORMAL | `h` `l` `0` `$` `w` `b` | column motions within the row |
| NORMAL | `gg` / `G` | oldest / newest retained entry |
| NORMAL | `F` | re-enter follow (tail) |
| NORMAL | `/` | fuzzy search prompt |
| NORMAL | `n` / `N` | next / previous confirmed hit |
| NORMAL | `v` / `V` | charwise / linewise visual selection |
| NORMAL | `y` | yank cursor entry as JSON (OSC52) |
| NORMAL | `Enter` | results view: jump to hit in stream; stream: toggle detail overlay |
| NORMAL | `Ctrl-w` + `v/s` | vertical / horizontal split (clones pane) |
| NORMAL | `Ctrl-w` + `h/j/k/l` | directional focus |
| NORMAL | `Ctrl-w` + `q` / `o` | close pane / only |
| NORMAL | `gt` / `gT` | next / previous tab |
| NORMAL | `?` | help overlay |
| NORMAL | `Esc` | clear pending / hits / results view |
| VISUAL | motions, `y`, `v`/`V`, `Esc` | extend, yank selection, switch kind / leave |
| SEARCH | text, `Enter`, `Esc` | live fuzzy; confirm; abandon |
| COMMAND | `:q :qa :sp :vs :only :tabnew :tabclose :tabn :tabp :filter :tail :clear :help` | see below |
| any | `Ctrl-c` | quit |

`:filter pat[,pat]` sets the pane's source patterns (substring match against
producer/group/name/id); a `=name` pattern matches a source's name or id
exactly; bare `:filter` clears. Source discovery is built into the cmdline:
`Tab` completes command names and, for `:filter`, live source names/groups/
producers (cycling vim-style), and `:sources` (alias `:ls`) opens a fuzzy
picker — type to narrow, `Tab` toggles, `Ctrl-a` toggles all narrowed rows,
`Enter` writes the focused pane's filter as exact `=name` patterns (with
nothing toggled it takes the highlighted row). `:clear` drops search
results/hits back to plain stream. `:q` closes the focused pane; closing the
last pane closes the tab; closing the last tab quits.

## Deliberately deferred

- Configurable keybindings (the keymap table is one match function; wiring
  the existing `[tui.keybindings]` config back in is mechanical).
- Line wrap (truncate-only in v1), mouse support (capture is left **off** so
  native terminal selection works), field-matched preview (`:trace`-style
  command can dispatch the existing worker later), analysis tabs.
