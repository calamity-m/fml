# fml — System Architecture

fml follows a message-bus pattern: many independent producers write to a single central store, and many independent consumers read from it. Producers and consumers never communicate directly — the store is the only shared state.

```
  PRODUCERS                     BUS                    CONSUMERS

 ┌─────────────────┐
 │ Ingestor        │  LogEntry
 │ (api-7f9b4d)    │ ──────────┐
 └─────────────────┘           │
                               │
 ┌─────────────────┐           ▼          ┌──────────────────────────┐
 │ Ingestor        │  ┌──────────────┐    │ Tab 1: main              │
 │ (worker-4c2a)   │─▶│    Store     │───▶│ query: level:error       │
 └─────────────────┘  │ (ring buffer)│    └──────────────────────────┘
                      │              │
 ┌─────────────────┐  │              │    ┌──────────────────────────┐
 │ Ingestor        │─▶│              │───▶│ Tab 2: freeze:api-7f9b4d │
 │ (worker-9e1b)   │  └──────────────┘    │ query: timeout           │
 └─────────────────┘                      └──────────────────────────┘

                                          ┌──────────────────────────┐
                                          │ Tab 3: correlate:req-123 │
                                          │ query: request_id:req-123│
                                          └──────────────────────────┘
```

One ingestor task runs per active producer. Each tab is an independent consumer with its own query and scroll position. Adding a tab does not start new ingestion — it opens another read view over the same store.

## Components

### Ingestors (`fml-feeds`)

Each ingestor connects to one producer (a pod, container, or file), reads raw bytes, normalises each line into a `LogEntry`, and writes it to the store. Ingestors run as supervised `tokio` tasks; a crash triggers reconnect rather than silently dropping the stream.

Normalisation converts raw log lines to `LogEntry` structs. Parsing is attempted in priority order:

1. **JSON** — valid JSON objects have all top-level keys promoted to searchable fields.
2. **Logfmt** — `key=value` pairs extracted.
3. **Heuristic regexes** — detect log level, timestamp, and request IDs in unstructured text.
4. **Fallback** — raw line stored as `message`.

Synthetic fields are injected unconditionally regardless of parse result:

| Field | Value |
|-------|-------|
| `source` | Feed type (`docker`, `kubernetes`, `file`, `stdin`) |
| `producer` | Container / pod / file name |
| `ts` | Ingest time (overridden if parsed from the line) |
| `level` | Best-effort (`trace`/`debug`/`info`/`warn`/`error`/`fatal`) |

### Store (`fml-core::store`)

The store is an in-memory ring buffer that all ingestors write to and all tabs read from. It is the only point of contact between producers and consumers.

- Fixed capacity (configurable, default 100 000 entries); oldest entries evict when full.
- Monotonic sequence numbers on every entry for deterministic ordering.
- Concurrent-safe: multiple reader tasks alongside one writer per active ingestor.

### Tabs / Search consumers (`fml-core::search`, `fml-tui`)

Each open tab holds an independent query and scroll position. When a tab renders, it runs its query against the store and displays the matching subset. Tabs share the store — no data is copied per tab.

The search pipeline each tab runs:

1. **Key filter** — optional `key:value` prefix restricts which fields are scanned.
2. **Term expansion** — each bare term is expanded via the greedy algorithm.
3. **FST scan** — expanded terms looked up against an `fst`-backed index for prefix/infix matches.
4. **Ranking** — results scored by match density and recency.

At greed 0 the expansion step is skipped; exact substring or regex matching only.

See [`docs/search/GREEDY_ALGORITHM.md`](search/GREEDY_ALGORITHM.md) for the full expansion spec.

The UI layer renders tabs and handles input. See [`docs/tui/ARCHITECTURE.md`](tui/ARCHITECTURE.md) for TUI internals.

## Source feeds

One feed type is active at a time. Within a feed, any number of producers can be selected; each gets its own ingestor task.

| Feed | Producer unit | Transport |
|------|---------------|-----------|
| `docker` | container name/id | Docker API over Unix socket |
| `kubernetes` | pod name (+ optional container) | `kubectl logs -f` subprocess |
| `file` | file path | `inotify`-based tail with rotation detection |
| `stdin` | — | raw stdin |

## Data types (`fml-core`)

```rust
pub struct LogEntry {
    pub seq: u64,                          // monotonic sequence number
    pub ts: chrono::DateTime<Utc>,
    pub level: Option<LogLevel>,
    pub source: FeedKind,
    pub producer: String,
    pub message: String,
    pub fields: IndexMap<String, String>,  // parsed key-value pairs
}

pub enum LogLevel { Trace, Debug, Info, Warn, Error, Fatal }
pub enum FeedKind { Docker, Kubernetes, File, Stdin }
```

## Crate layout

```
fml/                  workspace root + binary (src/main.rs)
├── crates/
│   ├── fml-core/     LogEntry, LogLevel, FeedKind, Config, Store, Search, Normalizer
│   ├── fml-feeds/    Feed-specific ingestors (docker, kubernetes, file, stdin)
│   └── fml-tui/      Ratatui app shell, widgets, themes, event system
└── docs/
```
