# fml

hello

```
┌─────────────────────────────────────┬──────────────────────┐
│ Log Pane                            │ Info                 │
│                                     │──────────────────────│
│   [src-a] request timeout host=x    │ timestamp  ...       │
│   [src-b] pod restarted reason=oom  │ level      error     │
│ > [src-a] connection refused host=x │ message    conn...   │
│   [src-c] dial tcp: no such host    │ host       x         │
│   [src-a] retrying after backoff    │ source     src-a     │
│                                     │──────────────────────│
│                                     │ Preview              │
│                                     │──────────────────────│
│                                     │ [src-a] req started  │
│                                     │ [src-a] req timeout  │
│                                     │>[src-a] conn refused │
│                                     │ [src-a] retrying...  │
├─────────────────────────────────────┴──────────────────────┤
│ Query  conn refused                                        │
├────────────────────────────────────────────────────────────┤
│ SEARCH  src-a,src-b,src-c  3/120 matches                   │
└────────────────────────────────────────────────────────────┘
```

## Log Pane Terms

- Head -> Oldest entry of the log store
- Tail -> Most recent entry of the log store
- Rendered Window -> Visible log lines in the log pane
- Rendered Head -> Top of the visible log lines
- Rendered Tail -> Bottom of the vissible log lines
- Retained Window -> Log lines the log pane has access to, which may extend past a Rendered Window, but not encompass a full ring buffer

## TODOs

### 0. Source Producer Identity

Sources should carry the producer identity before the source selector tree is implemented.
The `producer` field on `Source` is the top-level grouping key of the source selector
tree (Producer -> Group -> Display Name).

[x] Extend `Source` with a producer name or producer id field
[x] Require each producer to populate producer identity on `SourceFound` events
[x] Ensure stored log entries retain the producer identity through their embedded `Source`
[x] Update demo/fake producers and tests to include producer identity
[x] Document producer identity as the top-level source selector grouping key

### 1. Info Pane Selected Log Details

The info pane should display the log entry currently selected by the log pane cursor.

[x] Show selected log metadata and fields in the info pane
[x] Preserve fuzzy match metadata for the selected entry
[x] Apply the configured match highlight style inside the info pane
[x] Wrap message text using the full info pane width
[x] Add keyboard scrolling for info pane overflow with ctrl+up/down and ctrl+k/j
[x] Add snapshot coverage for selected log details with and without fuzzy highlights

### 2. Preview Pane Surrounding Mode

The preview pane's first mode should show nearby logs from the selected log entry's source.

[x] Introduce an enum-backed preview mode model with `Surrounding` as the only initial mode
[x] Add a "surrounding" preview mode
[x] Fetch and render entries around the selected sequence, e.g. cursor -7 through cursor +7
[x] Restrict surrounding preview entries to the selected log line's source
[x] Clearly mark the selected entry in the preview pane
[x] Add tests for tail, history, and fuzzy selections driving surrounding preview output

### 3. Source Selector Popup

A source selector popup should allow enabling and disabling sources before real producers are hooked up.

[] Add a configurable `ctrl+s` keybinding for opening the source selector
[] Design the source tree hierarchy as Producer -> Group -> Display Name
[] Validate and document the Producer -> Group -> Display Name hierarchy as the long-term source selector model
[] Allow toggling an individual source
[] Allow toggling a group and applying the state to all child sources
[] Allow toggling a producer and applying the state to all child groups and sources
[] Apply selected source filters to tail, history, and fuzzy searches
[] Add tests for popup navigation, producer toggles, group toggles, source toggles, and search filtering

### 4. Refined Default Theme

The default theme should be clearer and more cohesive before real producer output increases visual density.

[] Review current default colors across log pane, info pane, preview pane, query box, status bar, borders, selection, and matches
[] Improve contrast between normal text, metadata, selected rows, focused borders, and fuzzy highlights
[] Keep built-in themes cohesive while avoiding one-note palettes
[] Update snapshots where intentional visual changes affect rendered output

### 5. Store And Search Progress Visibility

The TUI should expose useful capacity and progress information without adding noise.

[] Show log store capacity and retained-buffer usage somewhere persistent, likely the status bar
[] Show fuzzy search progress toward completion while a scan is incomplete
[] Make the display distinguish retained buffer progress from fuzzy scan progress
[] Add reducer and snapshot tests for capacity/progress states

### 6. Fuzzy Search Validation And Docs

Fuzzy behavior should be validated and documented now that searches can run while logs continue arriving.

[] Document nucleo query syntax and the frizbee fallback config
[] Document what happens when new logs arrive while old retained chunks are still being scanned
[] Validate partial emissions, completion emissions, and re-scans after retained bounds change
[] Validate source filtering behavior during fuzzy search
[] Add or update tests for live arrivals during long fuzzy scans
