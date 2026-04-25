# fml

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

## Log Pane Functionality

### Terms:

- Head -> Oldest entry of the log store
- Tail -> Most recent entry of the log store
- Rendered Window -> Visible log lines in the log pane
- Rendered Head -> Top of the visible log lines
- Rendered Tail -> Bottom of the vissible log lines
- Retained Window -> Log lines the log pane has access to, which may extend past a Rendered Window, but not encompass a full ring buffer

### Normal Scrolling functionality

> Note: Assume VIM Mappings for scrolling in addition to arrow keys

When first starting out, the user will be in the "tail" mode. Upon pressing the up-arrow the user will "pause" the log pane. 
If the user scrolls to the top of the Rendered Window, and continues to scroll up, the Rendered Window will start to scroll through
the Retained Window - e.g. like you would expect of a log viewing application.

If the user scrolls back to the bottom of the Rendered Window, as is now at the latest seq - they will be put back into "tail" mode.
If the seq is not the latest, they they will scroll down the Retained Window like you would expect of a log viewing application,

#### Head and Tail

The keys of Home/g will take a user to the head of the log store, e.g. the oldest sequence kept - and enter "LogPaneState" mode.
The keys of End/G will take a user to the head of the log store, e.g. the latest seq kept - and enter "tail" mode.

### Fuzzy Scrolling Functionality

When the user issues a search, their cursor should be put to the Rendered Tail. Scrolling functionality should be the same, other than
the fact that they are scrolling through the fuzzy scrolling search results, rather than the store itself.

Pressing Home for instance would take them to the least matched result from the fuzzy search.

## Fuzzy Searching Extension

Right now we cut-off the fuzzy search if it doesn't make it within a specific time, but we should extend this so that the first result is emitted
at that time - and the search continues, with emissions being at that time (or completed search reached).

## TODO — minimal functionality

Rough order — each step assumes the previous ones are done.

### 1. Producer abstraction
- [x] `LogProducer` trait with `start`/`stop` (`fml/src/producer.rs`).
- [x] `ProducerEvent` enum: `SourceFound`, `SourceLost`, `StoreEvent` (`fml/src/event.rs`).
- [x] Event bus channels + `App::run` arm dispatching to `producer::handle_producer_event`.
- [x] `ProducerState { sources: Vec<Source> }` slot on `AppState`.
- [x] Flesh out `handle_producer_event` (`fml/src/producer.rs`): on `SourceFound` push to `state.producer.sources` (idempotent), on `SourceLost` remove by `SourceId`, on `StoreEvent` `try_send` the `NewLogEntry` into `event_bus.store_tx`.
- [x] Decide on a registration surface: `App` owns `Vec<Box<dyn LogProducer>>` and exposes `register_producer`. `run()` calls `start` on each after the TUI spawns and `stop` on each before TUI cleanup.
- [x] Pin down the shutdown contract for `LogProducer::stop` — documented on the trait and module: implementations must keep cancellation state behind a shared handle (`Arc<AtomicBool>` / `CancellationToken`) cloned into the spawned task so `stop` can flip it through `&self`.
- [x] Make sure each producer carries a `SourceId` — added `LogProducer::source_id(&self)` to the trait so registration and filtering can name a source without starting the producer.

### 2. Demo producer
- [x] `FakeProducer` struct + trait impl skeleton (`fml/src/producer/fake.rs`).
- [x] Implement `start`: spawn a tokio task that emits `SourceFound` once, then ticks out synthetic `StoreEvent(NewLogEntry)` values (use the `fake` crate already in deps).
- [x] Implement `stop`: signal the spawned task to exit (see shutdown contract above).
- [x] Vary `level`, `source.id`, and `fields` across emitted entries so highlighting and source filters are visibly exercised.
- [x] Wire the `--demo` CLI flag (`fml/src/main.rs:37`) through `App::new` so it actually constructs and registers a `FakeProducer`.

### 3. App consumes producers and pushes to store
- [x] In `App::run`, after the TUI spawns, iterate registered producers and call `start(producer_event_tx.clone())` on each.
- [x] In `handle_producer_event`'s `StoreEvent` arm, insert the entry directly into `state.store`. The `LogStore` write path is synchronous now; there is no separate store writer channel anymore.
- [x] On shutdown, call `stop()` on each producer before TUI cleanup so background producer tasks can observe cancellation and exit.
- [x] Smoke-check producer ingestion with reducer tests that verify `StoreEvent` advances `store.bounds()` and persists the inserted entry.

### 4. Log pane: tail / history
- [] Add `current_results: Vec<SearchHit>` (and a seq→matches lookup) to `SearchState`, plus a `current_mode: ScrollMode`.
- [] In `search.rs` `SearchEvent::Result`, write results into `SearchState` instead of dropping them.
- [] On startup, dispatch an initial `Query::Tail` so the log pane has data before any user input.
- [] Replace the fake row generator in `tui/widgets/log_pane.rs` with a windowed read of `current_results` + `store.fetch_requested(...)`.
- [] Format rows as `Line::from(spans)` (ts, level, source, msg) and color by `theme.log_row_fg(level)`.
- [] Fix the cursor bound in `tui/widgets/log_pane.rs` — clamp to results length, not viewport height; guard against `height == 0`.
- [] Scrolling away from the latest row enters `ScrollMode::History` anchored on the selected seq; returning to latest resumes `Query::Tail`.

### 5. Log pane: fuzzy
- [ ] On `QueryBox` input change (or Enter), emit `SearchEvent::Search { query: Fuzzy(text), … }`; emit `Tail` when the box clears.
- [ ] Switch `LogPaneState.mode` to `ScrollMode::Search` while a fuzzy query is active.
- [ ] Reset cursor / scroll position when results swap so the user lands on the top hit.

### 6. Info pane shows selection
- [ ] Add `selected_seq: Option<u64>` to `LogPaneState`; update on every cursor move by translating through the current results vec.
- [ ] In `tui/widgets/info_pane.rs`, fetch the selected entry from the store and render: timestamp, level, source.id, msg, then each (key, value) field on its own line.
- [ ] Handle the empty-selection case (no logs yet, or cursor out of range) without panicking.

### 7. Preview pane
- [ ] Split `SearchState` handles into `main_handle`/`main_results` and `preview_handle`/`preview_results` so the preview's history query doesn't clobber the main query.
- [ ] Tag outgoing search requests with which slot they belong to so the `Result` arm routes correctly.
- [ ] When `selected_seq` changes, dispatch a `Query::History { middle_seq_id, buffer: <small> }` against the preview slot.
- [ ] Render the preview slice in `tui/widgets/preview_pane.rs` like the log pane, but visually mark the row where `seq == selected_seq` (the `>` in the mockup).
- [ ] Replace `todo!()` in `preview_pane.rs:46` and `status_bar.rs:104` with `_ => {}` so unfocused-pane events can't panic.

### 8. Highlighting
- [ ] Build a `HashMap<u64, Vec<Match>>` lookup from the active fuzzy result so the renderer doesn't scan linearly each frame.
- [ ] In the log pane, split the `msg` of matched rows into spans and apply `theme.log_match_fg` (+ `log_match_bold`) at the indices in `Match::indices`.
- [ ] In the info pane, apply the same span-styling to matched fields (key matches the `Match.key` from the active hit for that seq).
- [ ] Verify highlight + selection styles compose (selected row keeps `log_selected_bg`, matched chars still show `log_match_fg`).

### 9. Polish / status
- [ ] Status bar: show `current_mode`, active sources, and `results.len() / store.bounds()` to satisfy the `SEARCH  src-a,src-b,src-c  3/120 matches` line in the mockup.
- [ ] Remove or populate the unused `items` / `search_results` fields on `LogPaneState` — pick one home for "currently displayed seq ids".
- [ ] Add a smoke test that boots `--demo`, lets the tail worker tick, and asserts the log pane state reflects ingested entries.
