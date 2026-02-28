# Implementation Plan: fml

## Context

fml is a pre-implementation Rust project with core types (`LogEntry`, `LogLevel`, `FeedKind`), 120 ignored test stubs across 9 harnesses, 5 benchmark suites, and zero functionality. The goal is to restructure into a workspace with separate crates and implement the full application in phases, starting with the TUI for early visual feedback.

The central architectural insight: **feeds push normalized entries into a shared Store via channels; tabs are materialized views** — each subscribes to a `tokio::broadcast` for new-entry notifications and reads from the Store on demand with its own filter. No data copying.

All phases should have tests generated, and when possible, phases should be completing performance testing and adding benchmarks where appropriate.

At the end of each phase the "code-review" plugin should be utilised.

## Workspace structure

```
fml/
├── Cargo.toml               ← workspace manifest + root binary crate
├── crates/
│   ├── fml-core/            ← types, store, normalizer, search, export, config
│   ├── fml-feeds/           ← kubernetes, docker, file, stdin implementations
│   └── fml-tui/             ← ratatui app, widgets, tabs, theme
├── src/main.rs              ← binary: CLI (clap), wiring, headless, MCP
├── tests/                   ← workspace-level integration harnesses (stay here)
├── benches/                 ← workspace-level criterion suites (stay here)
└── scripts/
```

Dependency graph:

```
fml (binary)
├── fml-tui    → fml-core
├── fml-feeds  → fml-core
└── fml-core
```

`fml-tui` and `fml-feeds` are siblings — neither depends on the other. The binary wires them together.

## Architecture: message bus with materialized views

```
Feed Worker A ─┐
               ├─ normalize() ─► mpsc::Sender<LogEntry> ─► Store::push()
Feed Worker B ─┘                                              │
                                                         broadcast(seq)
                                                              │
                                        ┌─────────────────────┼──────────────┐
                                        ▼                     ▼              ▼
                                   MainTab               FreezeTab     CorrelateTab
                                   filter: selected      filter: pod-x  filter: req_id=abc
                                        │                     │              │
                                        ▼                     ▼              ▼
                                   store.range(last_seen..)  ...            ...
                                   + apply StoreFilter
```

**Why not per-tab channels**: Backfill. When a freeze tab opens, it needs history instantly — scan the store backward with a filter. If tabs only received new entries via channels, historical data would require a separate query path. One source of truth (store) + notification (broadcast) is simpler and correct.

**Broadcast overflow**: `tokio::broadcast` has fixed capacity (default 1024). If a tab lags behind, it gets `RecvError::Lagged(n)`. The tab resets `last_seen_seq` to the store's current minimum seq and re-scans. No data loss.

**Store concurrency**: `Arc<RwLock<StoreInner>>`. Write lock held only during `push()` (single writer). Read lock for queries (multiple readers). If profiling shows contention, swap inner for a lock-free ring — API stays the same.

## Git Flow

For each new phase, we will first create a new branch named after a phase. Before we can complete a phase fully, we will need to merge this branch into main through a pull request via `gh pr`.

## Tracking

Issues are created at the start of each phase via `gh issue create`. Each row is a discrete deliverable with its own issue.

### Phase 1 — Workspace restructure

| # | Deliverable | Issue | Status | Completed |
|---|------------|-------|--------|-----------|
| 1.1 | Create workspace Cargo.toml + crate manifests | | | |
| 1.2 | Move core types to `fml-core` | | | |
| 1.3 | Create `fml-feeds` with `FeedHandle` trait stub | | | |
| 1.4 | Create `fml-tui` with `App` stub | | | |
| 1.5 | Migrate all test/bench imports to `fml_core::` | | | |
| 1.6 | Delete old `src/lib.rs`, verify `cargo test --workspace` (120 ignored) | | | |
| 1.7 | Merged | | | |

### Phase 2 — TUI shell (mock data)

| # | Deliverable | Issue | Status | Completed |
|---|------------|-------|--------|-----------|
| 2.1 | Theme + colour palette | | | |
| 2.2 | Event system (crossterm → semantic `AppEvent`) | | | |
| 2.3 | Tab bar widget | | | |
| 2.4 | Producer tree widget (collapsible, hjkl nav, selection) | | | |
| 2.5 | Log stream widget (scrollable, pause/resume, `G` to tail) | | | |
| 2.6 | Query bar widget (text input, greed slider) | | | |
| 2.7 | App shell + layout + focus switching | | | |
| 2.8 | Add help popup displaying possible actions (`?` to open) | | | |
| 2.10 | Add a config.toml reading for TUI | | | |
| 2.11 | Wire `cargo run` → TUI with hardcoded mock data | | | |
| 2.12 | Merged | | | |

### Phase 3 — Store + normalizer

| # | Deliverable | Tests | Issue | Status | Completed |
|---|------------|-------|-------|--------|-----------|
| 3.1 | Store ring buffer + `push()` / eviction | 3 | | | |
| 3.2 | Store sequence numbers + `range()` / `latest()` / `get()` | 3 | | | |
| 3.3 | Store concurrent reads + `Arc<RwLock>` | 2 | | | |
| 3.4 | Store filters (producer, level) + `StoreFilter` | 3 | | | |
| 3.5 | Store broadcast notifications | — | | | |
| 3.6 | Normalizer: JSON parser | 5 | | | |
| 3.7 | Normalizer: logfmt parser | 2 | | | |
| 3.8 | Normalizer: unstructured pattern detection | 3 | | | |
| 3.9 | Normalizer: fallback + synthetic fields + edge cases | 5 | | | |
| 3.10 | Normalizer: insta snapshots | 4 | | | |
| 3.11 | Merged | 4 | | | |

### Phase 3.5 — Wire store → TUI

| # | Deliverable | Issue | Status | Completed |
|---|------------|-------|--------|-----------|
| 3.5.1 | App takes `Arc<Store>` + broadcast receiver | | | |
| 3.5.2 | Log stream reads from store | | | |
| 3.5.3 | Producer tree populates from `store.producers()` | | | |
| 3.5.4 | Demo mode (fake entries at configurable rate, default 10/sec) | | | |
| 3.5.5 | Wire -> TUI with generated mock data based on a `--demo` flag | | | |
| 3.5.6 | Merged | | | |

### Phase 4 — Feed implementations

| # | Deliverable | Tests | Issue | Status | Completed |
|---|------------|-------|-------|--------|-----------|
| 4.1 | `FeedHandle` trait + `FeedEvent` enum | — | | | |
| 4.2 | stdin feed | 9 | | | |
| 4.3 | File feed (inotify, rotation, truncation, glob) | 10 | | | |
| 4.4 | Kubernetes feed (kubectl subprocess, reconnect, backoff) | 9 | | | |
| 4.5 | Docker feed (API client, frame decoding, socket auto-detect) | 10 | | | |
| 4.6 | Wire feeds into binary + TUI producer tree | — | | | |
| 4.7 | Merged | | | | |

### Phase 4.5 — Mock Feed Generators

| # | Deliverable | Tests | Issue | Status | Completed |
|---|------------|-------|-------|--------|-----------|
| 4.5.1 | Create fake feed generators that can be used for demoing  | | | |
| 4.5.2 | Wire -> TUI with generated mock data based on a `--demo` flag | | | |
| 4.5.3 | Document demo, intention is to show off capabilities to interested users quick and easily | | | |
| 4.5.4 | Evaluate and expose mock feed generators for randomized statisical benchmarking | | | |
| 4.5.5 | Merged | | | | |

### Phase 5 — Search engine

| # | Deliverable | Tests | Issue | Status | Completed |
|---|------------|-------|-------|--------|-----------|
| 5.1 | Ontology clusters (`phf` static data, 7 domain families) | — | | | |
| 5.2 | Semantic graph (TermNode, weighted edges, BFS) | — | | | |
| 5.3 | FST index (prefix scan over ontology terms) | — | | | |
| 5.4 | Greed-gated expansion (layers 0→10) | 8 | | | |
| 5.5 | Negative prefix inference | 2 | | | |
| 5.6 | Generalised fuzzy searching - skim | 2 | | | |
| 5.7 | Query parsing (key:value + bare terms) | 3 | | | |
| 5.8 | Full search pipeline + ranking | 5 | | | |
| 5.9 | Greed monotonicity property test | 2 | | | |
| 5.10 | Evaluate backwards semantic referencing (token -> auth) | 2 | | | |
| 5.11 | Implement backwards semantic referencing (token -> auth) | 2 | | | |
| 5.12 | Wire search into TUI query bar + highlight matched spans | 2 | | | |
| 5.13 | Merged | | | | |

### Phase 6 — Tabs + export + headless

| # | Deliverable | Tests | Issue | Status | Completed |
|---|------------|-------|-------|--------|-----------|
| 6.1 | Tab trait + TabManager | — | | | |
| 6.2 | MainTab (live-tail, selected producers) | — | | | |
| 6.3 | FreezeTab (`y` on producer → scoped tab) | — | | | |
| 6.4 | CorrelateTab (`c` on log line → field picker → cross-producer) | — | | | |
| 6.5 | Export: raw format | 4 | | | |
| 6.6 | Export: jsonl format | 4 | | | |
| 6.7 | Export: csv format | 4 | | | |
| 6.8 | Export: empty/edge cases + insta snapshots | 3 | | | |
| 6.9 | Headless mode (all flags, TTY detection, exit codes) | 12 | | | |
| 6.10 | Config file loading (`~/.config/fml/config.toml`) | — | | | |
| 6.11 | Merged | | | | |

### Phase 7 — MCP server + polish

| # | Deliverable | Issue | Status | Completed |
|---|------------|-------|--------|-----------|
| 7.1 | MCP server (`fml --mcp`, `fml_query` tool) | | | |
| 7.2 | Skim picker (feed selection, producer fuzzy-jump) | | | |
| 7.3 | Configurable keybindings from config.toml | | | |
| 7.4 | Merged | | | |


### Phase 8 — Stretch Goals - Runtime corpus learning

This is a stretch goal and aspirational. It should not be enabled by default.

| # | Deliverable | Issue | Status | Completed |
|---|------------|-------|--------|-----------|
| 8.1 | Evaluate options for runtime learning, create research document (cover learning, performance, binary sizing, maintenance and storage/persistence) | | | |
| 8.2 | Implement semantic learning on existing semantic graph for incoming feed lines | | | |
| 8.3 | Wire in results from runtime learning into search engine | | | |
| 8.4 | Performance profiling + optimization pass | | | |
| 8.5 | Merged | | | |

### Summary

| Phase | Deliverables | Tests unignored | Status |
|-------|-------------|-----------------|--------|
| 1 | 7 | 0 | |
| 2 | 12 | 0 | |
| 3 | 11 | 33 | |
| 3.5 | 6 | 0 | |
| 4 | 7 | 38 | |
| 4.5 | 5 | 0 | |
| 5 | 13 | 22 | |
| 6 | 11 | 27 | |
| 7 | 4 | 0 | |
| **Total** | **76** | **120** | |

### Benchmark baselines

| Bench | Groups | Phase | Baselined | Issue |
|-------|--------|-------|-----------|-------|
| `store_bench` | 3 | 3 | | |
| `normalization_bench` | 4 | 3 | | |
| `ingestor_bench` | 4 | 4 | | |
| `search_bench` | 5 | 5 | | |
| `export_bench` | 3 | 6 | | |
| `pipeline_bench` | 4 | 6 | | |
| `tui_bench` | 3 | 6 | | |

---

## Phase 1: Workspace restructure

**Goal**: Convert single crate to workspace. Everything compiles, all 120 ignored tests still pass.

### What changes

1. Root `Cargo.toml` → workspace manifest with `members = ["crates/*"]` + root binary package
2. `crates/fml-core/` — move `LogEntry`, `LogLevel`, `FeedKind` from `src/lib.rs` into `crates/fml-core/src/types.rs`. Create stub modules: `normalizer`, `store`, `search`, `export`, `config`
3. `crates/fml-feeds/` — `FeedHandle` trait stub + empty feed modules (`kubernetes`, `docker`, `file`, `stdin`)
4. `crates/fml-tui/` — stub `App` struct + `pub fn run() -> Result<()>`
5. Old `src/lib.rs` → deleted (types live in fml-core now)
6. All test/bench imports: `use fml::` → `use fml_core::`
7. Root `src/main.rs` stays as the binary entry point

### Files

| Path | Action |
|------|--------|
| `Cargo.toml` | Rewrite as workspace + binary |
| `crates/fml-core/Cargo.toml` | Create |
| `crates/fml-core/src/lib.rs` | Create (re-exports) |
| `crates/fml-core/src/types.rs` | Create (moved from src/lib.rs) |
| `crates/fml-core/src/{normalizer,store,search,export,config}.rs` | Create (stubs) |
| `crates/fml-feeds/Cargo.toml` | Create |
| `crates/fml-feeds/src/lib.rs` | Create (FeedHandle trait) |
| `crates/fml-feeds/src/{kubernetes,docker,file,stdin}.rs` | Create (stubs) |
| `crates/fml-tui/Cargo.toml` | Create |
| `crates/fml-tui/src/lib.rs` | Create (App stub) |
| `src/lib.rs` | Delete |
| `src/main.rs` | Update imports |
| `tests/**/*.rs`, `tests/common/**/*.rs` | Update imports |
| `benches/*.rs` | Update imports |

### Verify

```sh
cargo check --workspace
cargo test --workspace   # 0 pass, 120 ignored
```

---

## Phase 2: TUI shell with mock data

**Goal**: `cargo run` launches a full-screen ratatui app with hardcoded mock data. All four panes render. Navigation works. `q` exits.

### Layout

Discretion:

The query needs to have the ability to use skim's fuzzy finding, and also the search engine's specific queries (e.g. greedy algorithm).
A possible implementation of this is to take the resolved search terms from the greedy algorithm, and provide them to skim's fuzzy
finding ability. Consider whether a separate "fuzzy search" should be used in the query, or for instance, do queries need to be:

* prepended - key:my key greed:auth hello-world
  * here we reduce to logs with key/value of key:my key
  * we then reduce logs with the greedy `auth` search
  * we finally fuzzy find over hello-world
* (preferred) automatically handled - key:my key auth
  * here we reduce to logs with key/value of key:my key
  * here we reduce logs with the greedy `auth` search, and fuzzy find over the

```
┌──────────────────────────────────────────────────────────────┐
│ [1:main ●]  [+]                                              │
├─────────────────────┬────────────────────────────────────────┤
│ ▼ prod-cluster      │ 12:00:01.234  api-7f9b  {"level":"…"} │
│   ▼ default         │ 12:00:01.312  worker-4  connection…    │
│     ● api-7f9b    ✓ │ 12:00:01.450  api-7f9b  timeout…      │
│     ● worker-4c   ✓ │                                        │
│     ○ worker-9e     │                                        │
│   ▶ kube-system     │                                        │
├─────────────────────┴────────────────────────────────────────┤
│  query: [________________________]  greed: [=====-----] 5    │
└──────────────────────────────────────────────────────────────┘
```

### What to implement

| File | Contents |
|------|----------|
| `fml-tui/src/lib.rs` | `pub fn run() -> Result<()>`, terminal setup/teardown |
| `fml-tui/src/app.rs` | `App` state, main event loop, layout |
| `fml-tui/src/event.rs` | `AppEvent` enum, crossterm → semantic key mapping |
| `fml-tui/src/theme.rs` | Colour palette, producer colours, style constants |
| `fml-tui/src/widgets/mod.rs` | Re-exports |
| `fml-tui/src/widgets/tab_bar.rs` | Tab strip renderer |
| `fml-tui/src/widgets/producer_tree.rs` | Collapsible tree, hjkl nav, selection toggle |
| `fml-tui/src/widgets/log_stream.rs` | Scrollable log lines, pause indicator, `G` to tail |
| `fml-tui/src/widgets/query_bar.rs` | Text input, cursor, greed slider display |
| `src/main.rs` | Call `fml_tui::run()` |

### Verify

```sh
cargo run                # TUI launches, mock data visible
# Tab/arrow keys navigate, q exits, / focuses query bar
```

---

## Phase 3: Store + normalizer

**Goal**: The data pipeline works. Raw strings → normalized `StoreEntry` with sequence numbers. Ring eviction, broadcast notifications, concurrent reads. 33 tests unignored.
The normalised logs should have enhanced metadata where possible - for example, retaining a docker's container name or PID, retaining a kubernete pod's name, a file's path,
etc.

### Store architecture

```rust
// fml-core/src/store/mod.rs
pub struct Store {
    inner: Arc<RwLock<StoreInner>>,
    broadcast_tx: broadcast::Sender<u64>,
}

struct StoreInner {
    buffer: VecDeque<StoreEntry>,
    capacity: usize,
    next_seq: u64,
}

pub struct StoreEntry {
    pub seq: u64,
    pub entry: LogEntry,
}
```

API: `push()`, `subscribe()`, `get(seq)`, `range(from, to)`, `latest(n)`, `by_producer()`, `by_level()`, `producers()`, `len()`, `capacity()`

### Normalizer pipeline

`normalize(raw, source, producer) → LogEntry` tries in order:
1. JSON (`serde_json`) → promote top-level keys to fields
2. Logfmt (`key=value`) → extract pairs, handle quoted values
3. Patterns (regex) → detect level, timestamp, request ID
4. Fallback → raw as `message`, inject synthetic fields

After finding the format, it then must inject the required synthetic fields for the given producer.

### Files

| Path | Action |
|------|--------|
| `crates/fml-core/src/types.rs` | Add `StoreEntry`, `Serialize` derives |
| `crates/fml-core/src/store/mod.rs` | Create (Store, ring buffer, broadcast) |
| `crates/fml-core/src/store/filter.rs` | Create (StoreFilter) |
| `crates/fml-core/src/normalizer/mod.rs` | Create (pipeline) |
| `crates/fml-core/src/normalizer/json.rs` | Create |
| `crates/fml-core/src/normalizer/logfmt.rs` | Create |
| `crates/fml-core/src/normalizer/patterns.rs` | Create |
| `tests/snapshots/` | Create (insta snapshots) |

### Verify

```sh
cargo test --test store_harness           # 11 pass
cargo test --test normalization_harness   # 22 pass
cargo insta review                        # approve initial snapshots
```

---

## Phase 3.5: Wire store into TUI

**Goal**: Replace mock data with real store reads. Add a demo mode that pushes fake entries at 10/sec so the TUI has live data to display.

### What changes

- `App` takes `Arc<Store>` + `broadcast::Receiver<u64>`
- Log stream reads from `store.latest(visible_lines)` instead of hardcoded vec
- Producer tree populates from `store.producers()`
- Demo mode: background task pushes varied fake log entries into the store

### Verify

```sh
cargo run   # live-updating log stream, scroll up pauses, G resumes
```

---

## Phase 4: Feed implementations

**Goal**: Real log data flows from all four sources. 38 tests unignored. 

### FeedHandle trait

```rust
// fml-feeds/src/lib.rs
#[async_trait]
pub trait FeedHandle: Send + Sync {
    async fn discover_producers(&self) -> Result<Vec<ProducerInfo>>;
    async fn tail(&self, producers: Vec<String>, tx: mpsc::Sender<RawLogLine>) -> Result<()>;
}
```

Each feed normalizes via `fml_core::normalizer::normalize()` before sending.

### Implementation order

1. **stdin** — simplest, `tokio::io::stdin()` split on newlines
2. **file** — `notify` crate, rotation detection (inode change), truncation (size < offset), glob
3. **kubernetes** — `kubectl logs -f` subprocess, pod discovery via `kubectl get pods -o json`, reconnect with exponential backoff (capped 30s)
4. **docker** — HTTP client against configurable socket URL (auto-detect: `$DOCKER_HOST` → `/var/run/docker.sock` → `$XDG_RUNTIME_DIR/podman/podman.sock`), multiplexed frame decoding (8-byte header), compose label extraction

### Verify

```sh
cargo test --test stdin_harness           # 9 pass
cargo test --test file_harness            # 10 pass
cargo test --test kubernetes_harness      # 9 pass
cargo test --test docker_harness          # 10 pass
echo '{"level":"info","msg":"hi"}' | cargo run -- --feed stdin   # shows in TUI
```

---

## Phase 4.5: Mock Feed Generators

**Goal**: Randomized live data flows that allow for better performance testing and demoing of capabitlities.

### Performance testing

At this point we should be creating performance benchmarks where possible. For instance, bench marking the ingest flow for the chain we have thus far of  `feed -> normalisation -> store -> consumer tab` flow

#### Critical

From this point on, **ALL PHASES** must adhere to performance benchmarking, and modifications __must__ run existing benchmarks, and new features should extrapolate and build upon, or add new benchmarks.

### Performance Tuning

We should iterate here before we move to the search engine, to try and maximise performance of the ingest flow we have this far.

### Verify

* Run at a minimum, three separate code review cycles:
  1. Run performance benchmark and generate flamegraphs
  2. Analyze results of performance benchmarks and flamegraphs
  3. Isolate potential performance bottlenecks in results
  4. Modify code in a new branch and create a pull request using `gh pr`
  5. Spin up a sub-agent code-reviewer to analyze the pull request using `gh pr`
  6. action feedback, resubmit for re-review
  7. repeat steps 5-6 until approved
  8. merge changes

Utilise the code-review plugin

---

## Phase 5: Search engine

**Goal**: Full query pipeline — exact mode, key-value filters, greed-gated semantic expansion, FST prefix scans. 22 tests unignored. **Most critical phase** — greed monotonicity must hold. The query needs to have the ability to use skim's fuzzy finding, and also the search engine's specific queries (e.g. greedy algorithm). A possible implementation of this is to take the resolved search terms from the greedy algorithm, and provide them to skim's fuzzy finding ability. Consider whether a separate "fuzzy search" should be used in the query, or for instance, do queries need to be:

* prepended - key:my key greed:auth hello-world
  * here we reduce to logs with key/value of key:my key
  * we then reduce logs with the greedy `auth` search
  * we finally fuzzy find over hello-world
* (preferred) automatically handled - key:my key auth
  * here we reduce to logs with key/value of key:my key
  * here we reduce logs with the greedy `auth` search, and fuzzy find over the outcome

**Critical** greed must be reverseable, such that auth -> token, we can resolve token -> auth, given enough of a greed value.

### Components

| File | Role |
|------|------|
| `search/ontology.rs` | ~200 terms across 7 domain families, `phf::phf_map!` for seed→cluster lookup |
| `search/graph.rs` | `TermNode`, `RelationType`, directed weighted edges, asymmetric weights |
| `search/fst_index.rs` | `fst::Set` built from ontology terms, `prefix_scan()` for morphological expansion |
| `search/expansion.rs` | Greed-gated BFS: 0=literal, 1-2=prefix/morphological, 3-6=synonyms, 7-9=domain peers (1-hop), 10=2+ hops. Negative prefix inference biases toward error/failure clusters |
| `search/mod.rs` | `SearchEngine` — parse query → expand terms → FST scan → filter store → rank by density+recency → return seq numbers |

### Critical invariant

**Greed monotonicity**: `expansion(term, greed=G) ⊇ expansion(term, greed=G-1)` for all G in 1..=10. This is tested exhaustively by `search_harness::prop_greed_monotonicity`.

### Verify

```sh
cargo test --test search_harness   # 22 pass, monotonicity holds
cargo bench --bench search_bench   # baseline numbers
```

We should build upon the existing benchmarks by creating benchmarks and flamegraphs for the search engine feature.

We should iterate here before we move to the tabs + export + headless, to try and maximise performance of the flow we have this far.

### Verify

* Run at a minimum, three separate code review cycles:
  1. Run performance benchmark and generate flamegraphs
  2. Analyze results of performance benchmarks and flamegraphs
  3. Isolate potential performance bottlenecks in results
  4. Modify code in a new branch and create a pull request using `gh pr`
  5. Spin up a sub-agent code-reviewer to analyze the pull request using `gh pr`
  6. action feedback, resubmit for re-review
  7. repeat steps 5-6 until approved
  8. merge changes

Utilise the code-review plugin

---

## Phase 6: Tabs + export + headless

**Goal**: Complete tab system (freeze/correlate), export pipeline, headless mode, config loading. 27 tests unignored. **All 120 tests pass after this phase.**

### Tab system (materialized views)

Each tab holds:
- `broadcast::Receiver<u64>` — new-entry notifications
- `last_seen_seq: u64` — position tracking
- `StoreFilter` — what entries this view cares about
- `ScrollState` — offset, paused flag

| Tab type | Created by | Filter | Searches |
|----------|-----------|--------|----------|
| `MainTab` | Always open | Selected producers + active query | Selected producers only |
| `FreezeTab` | `y` on producer node | Single producer + own query | Single producer |
| `CorrelateTab` | `c` on log line → field picker | `field:value` | **All** producers |

### Export

`export(entries, format, writer)` — three formats (raw, jsonl, csv) × four scopes (entire store, active filter, selected producer, producer + filter). Background task → temp file → open in editor.

### Headless

`fml --headless` — skip TUI, wire feed → store → search → stdout. Flags: `--query`, `--greed`, `--tail`, `--duration`, `--format`, `--no-metadata`. TTY detection for colour.

### Verify

```sh
cargo test --workspace   # 120 pass, 0 ignored
cargo bench              # full baseline report
```

---

## Phase 7: MCP server + polish

**Goal**: Claude Code integration, polish

- `fml --mcp` — axum server exposing `fml_query` tool (JSON-RPC)
- feature polish where possible, or required

---
