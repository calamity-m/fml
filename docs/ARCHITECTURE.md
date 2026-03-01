# fml — System Architecture

`fml` is split into four loosely coupled layers that communicate exclusively through `tokio` async channels. No layer holds a reference to another; they share data only through the channel boundaries.

```
┌─────────────┐   raw bytes    ┌────────────┐  LogEntry   ┌───────┐
│  Ingestor   │ ─────────────▶ │ Normalizer │ ──────────▶ │ Store │
│ (feed task) │                │            │             │       │
└─────────────┘                └────────────┘             └───┬───┘
                                                              │ read
                                                         ┌────▼────┐
                                                         │ Search  │
                                                         └────┬────┘
                                                              │ results
                                                         ┌────▼────┐
                                                         │   UI    │
                                                         └─────────┘
```

## Layers

### 1. Ingestor

Connects to the chosen source feed, reads raw bytes (subprocess stdout, API stream, file tail), and forwards lines to the Normalizer. One ingestor task runs per active producer (pod, container, file). All ingestor tasks are supervised so crashes trigger reconnect rather than silently dropping the stream.

See `docs/FEATURES.md` for the producer/feed model.

### 2. Normalizer (`fml-core::normalizer`)

Converts raw log lines to `LogEntry` structs. Parsing is attempted in priority order:

1. **JSON** — valid JSON objects have all top-level keys promoted to searchable fields.
2. **Logfmt** — `key=value` pairs extracted.
3. **Heuristic regexes** — detect log level, timestamp, and request IDs in unstructured text.
4. **Fallback** — raw line stored as `message`.

Synthetic fields injected unconditionally regardless of parse result:

| Field | Value |
|-------|-------|
| `source` | Feed type (`docker`, `kubernetes`, `file`, `stdin`) |
| `producer` | Container / pod / file name |
| `ts` | Ingest time (overridden if parsed from the line) |
| `level` | Best-effort (`trace`/`debug`/`info`/`warn`/`error`/`fatal`) |

### 3. Store (`fml-core::store`)

An in-memory ring buffer of `LogEntry` values. The store is the **single source of truth** — the UI and search engine read from it; the ingestor writes to it; nothing else touches the data.

Key properties:
- Fixed capacity (configurable, default 100 000 entries); oldest entries evict when full.
- Monotonic sequence numbers on every entry for deterministic ordering.
- Concurrent-safe: multiple reader tasks (UI, search) alongside one writer (ingestor).

### 4. Search (`fml-core::search`)

A query engine that runs against the store. See `docs/search/GREEDY_ALGORITHM.md` for the full expansion spec.

Pipeline:

1. **Key filter** — optional `key:value` prefix restricts which fields are scanned.
2. **Term expansion** — each bare term is expanded via the greedy algorithm.
3. **FST scan** — expanded terms looked up against an `fst`-backed index for prefix/infix matches.
4. **Ranking** — results scored by match density and recency.

At greed 0 the expansion step is skipped; exact substring / regex matching only.

### 5. UI (`fml-tui`)

A `ratatui` application that renders the current view, handles input, and dispatches `AppEvent` values back to the store and search layers. Runs on the main thread driven by a `crossterm` event loop at 16 ms poll intervals (≈60 fps).

See `docs/tui/ARCHITECTURE.md` for the full TUI internals.

## Data types (`fml-core`)

```rust
pub struct LogEntry {
    pub seq: u64,                              // monotonic sequence number
    pub ts: chrono::DateTime<Utc>,
    pub level: Option<LogLevel>,
    pub source: FeedKind,
    pub producer: String,
    pub message: String,
    pub fields: IndexMap<String, String>,      // parsed key-value pairs
}

pub enum LogLevel { Trace, Debug, Info, Warn, Error, Fatal }
pub enum FeedKind { Docker, Kubernetes, File, Stdin }
```

## Source Feeds

Only one feed is active at a time. Within a feed, multiple **producers** can be selected simultaneously; their streams are merged and tagged.

| Feed | Producer unit | Transport |
|------|---------------|-----------|
| `docker` | container name/id | Docker API over Unix socket |
| `kubernetes` | pod name (+ optional container) | `kubectl logs -f` subprocess |
| `file` | file path | `inotify`-based tail with rotation detection |
| `stdin` | — | raw stdin, useful for piping |

## Crate layout

```
fml/                  workspace root + binary entry point (src/main.rs)
├── crates/
│   ├── fml-core/     LogEntry, LogLevel, FeedKind, Config, Store, Search, Normalizer
│   ├── fml-feeds/    Feed-specific ingestors (docker, kubernetes, file, stdin)
│   └── fml-tui/      Ratatui application shell, widgets, themes, event system
└── docs/             This documentation tree
```
