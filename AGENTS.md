# Init Context

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:

- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them instead of picking silently.
- If a simpler approach exists, say so. Push back when warranted.
- Don't silently expand into wiring, integrations, or adjacent work that wasn't requested.
- If something is unclear, stop, name what's confusing, and ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No configurability that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:

- Don't improve adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing Rust style, module layout, and test patterns.
- If you notice unrelated dead code, mention it instead of deleting it.

When your changes create orphans:

- Remove imports, variables, functions, or tests that your changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:

- "Add validation" -> "Write tests for invalid inputs, then make them pass"
- "Fix the bug" -> "Write a test that reproduces it, then make it pass"
- "Refactor X" -> "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```text
1. [Step] -> verify: [check]
2. [Step] -> verify: [check]
3. [Step] -> verify: [check]
```

Strong success criteria let you loop independently. Weak criteria require clarification.

## 5. In-Code Documentation

**Public API must be documented. Internal logic should explain the why.**

For public Rust modules, traits, structs, enums, functions, and constants:

- Use rustdoc comments: `//!` for modules and `///` for items.
- Describe what the item is for and any non-obvious parameter, return, or concurrency constraints.
- If the types make everything clear, a one-liner is enough.

For internal code, comment the why, not the what:

- Event ordering, async cancellation, terminal lifecycle, and store/search invariants earn a short comment.
- Keep comments short. Delete comments that merely restate the code.

## 6. Pre-commit Hooks

**Prefer automated checks over repeated manual reminders.**

No pre-commit configuration is currently checked in. If adding one, base it on the commands this repo already uses naturally:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets`

Rustfmt is available, but `cargo fmt --check` currently reports existing formatting drift in `fml/src/producer/file.rs` and `fml/src/producer/normalizer/logfmt.rs`. Do not make it a blocking hook until that drift is fixed. Likewise, do not add `-- -D warnings` to clippy until the existing producer/normalizer warnings are cleared.

For local verification after Rust changes, run the smallest relevant command first, then broaden to `cargo test --workspace` when the change touches shared state, event flow, producers, search, or TUI rendering.

## 7. Repository Map

**Brief orientation. Where things live, where execution starts, how data moves.**

### Key directories

```text
fml/src/                 -> Rust crate source for the terminal log viewer
fml/src/main.rs          -> CLI parsing, config/logging initialization, app startup
fml/src/app.rs           -> App construction, producer lifecycle, main async event loop
fml/src/tui/             -> ratatui/crossterm rendering, input handling, layout, widgets
fml/src/state/           -> AppState plus focused TUI/search/producer state structs
fml/src/producer/        -> fake/file/docker/kubernetes log producers and normalizers
fml/src/search/          -> tail, history, and fuzzy search workers and reducer
fml/src/config/          -> TOML/env-backed config structs and built-in themes
fml/tests/               -> integration and snapshot-style tests
```

### Entry points

```text
fml/src/main.rs  -> `cargo run -p fml -- --producer demo` (interactive demo TUI)
fml/src/lib.rs   -> `cargo test -p fml` (library and integration tests)
```

### Data flow

```text
CLI/config -> App::new -> AppState + RingBufferStore + EventBus
          -> tui::spawn + registered LogProducer::start
          -> App::event_loop receives TuiEvent/SearchEvent/ProducerEvent
          -> reducers update AppState, store entries, dispatch searches, and render TUI
```

Producers emit `SourceFound`, `SourceLost`, and `StoreEvent` messages. Store events append to `RingBufferStore`; TUI actions dispatch `Query` values; search workers read retained log entries and return target-scoped results back through the event bus.

## 8. Project-Specific Notes

- The app is an async terminal UI built on tokio, crossterm, and ratatui.
- `App::event_loop` is the central reducer loop; keep event ordering changes deliberate and tested.
- `LogProducer::start` must return promptly; long-running ingest work belongs in a spawned task and must observe the producer cancellation contract.
- Producers should announce sources before emitting entries that reference them.
- `RingBufferStore` assigns monotonic sequence IDs while retaining only the configured capacity.
- Search is latest-wins per `SearchTarget`; stale worker results are intentionally discarded by request id.
- Source filtering uses stable source IDs, while display names are labels for users.
- Config loads from user config, local `.config/fml/config`, then `FML__*` environment overrides.

---

**These guidelines are working if:** diffs stay focused, new behavior has concrete verification, and clarifying questions happen before implementation rather than after mistakes.
