# claude

# TODO: we must specify the use of flamegraphs, ideally we can link these in with benchmarks.
# TODO: we must reiterate and enforce the need for performance testing - waiting until the last phase will not suffice
# TODO: flesh out Backwards resolution in CLAUDE.md
# TODO: Re-read IMPLEMENTATION_PLAN.md, changes have been made since last viewing
# TODO: Add merged steps to phases that don't have them.

> **fml** — *Feed Me Logs*
>
> *The log triage tool you open when something is already broken. Built for high-stress, time-pressured moments — multi-source ingestion, semantic search that thinks ahead of you, and a UI that gets out of the way so you can find the line that matters before the incident gets worse.*

`fml` is a terminal TUI for log aggregation, triage, and searching. It ingests a live stream of log lines from a configurable source feed, normalises them into a structured internal representation, and lets you search, filter, and export with minimal friction. The design philosophy is speed-first: the UI should never block on ingestion, search should feel instant, and the keybindings should keep your hands off the mouse.

## Architecture

The application is split into four loosely coupled layers:

1. **Ingestor** — connects to the chosen source feed, reads raw bytes, and pushes normalized `LogEntry` structs onto an async channel.
2. **Store** — an in-memory ring buffer of log entries with indexed metadata (level, source, timestamp, key-value pairs extracted during normalization). The store is the single source of truth; the UI reads from it, never from the feed directly.
3. **Search** — a query engine that runs against the store. Supports exact key-value queries, regex, and the greedy semantic expansion described below. Results are a filtered, ranked view over the store — no copying.
4. **UI** — a `ratatui` application that renders the current view, handles input, and dispatches commands back to the ingestor and search layers.

Communication between layers uses `tokio` channels. The ingestor and store live on background tasks; the UI runs on the main thread driven by a `crossterm` event loop.

## Tech Stack

| Crate | Role |
|-------|------|
| [`ratatui`](https://crates.io/crates/ratatui) | TUI layout, widgets, and rendering |
| [`skim`](https://crates.io/crates/skim) | Fuzzy-find picker used in source/producer selection dialogs |
| [`fst`](https://crates.io/crates/fst) | Memory-mapped finite state transducer for fast prefix scans against the term ontology |
| [`phf`](https://crates.io/crates/phf) | Compile-time perfect hash maps for the static ontology cluster data |
| [`clap`](https://crates.io/crates/clap) | CLI argument parsing — feed selection, initial query, config path |

## Configuration

`fml` reads `~/.config/fml/config.toml` on startup (path overridable with `--config`). If the file does not exist it is created with defaults. All CLI flags have a corresponding config key; the CLI takes precedence over the file.

```toml
[general]
# Default feed to open on startup. If unset, fml shows the source picker.
default_feed = "kubernetes"
# Editor to open exported logs in. Falls back to $EDITOR, then vi.
editor = "code --wait"
# Ring buffer size (number of log lines kept in memory per session).
store_capacity = 100_000

[search]
# Default greed level (0 = exact, 10 = max expansion).
default_greed = 4
# Show matched-span highlights in the log stream.
highlight_matches = true

[ui]
# Show timestamps in the log stream pane.
show_timestamps = true
# Timestamp format (strftime).
timestamp_format = "%H:%M:%S%.3f"
# Width of the producer tree pane as a percentage of terminal width.
producer_pane_width_pct = 25

[keybindings]
# All keybindings can be overridden here.
toggle_focus   = "Tab"
query_focus    = "/"
greed_up       = "]"
greed_down     = "["
yank_producer  = "y"
correlate      = "c"
export         = "e"
scroll_to_tail = "G"

[feeds.kubernetes]
# Default context. If unset, uses current kubeconfig context.
context = ""
# Default namespace to expand on open.
default_namespace = "default"

[feeds.docker]
# Docker socket path.
socket = "/var/run/docker.sock"

[feeds.file]
# Glob patterns to offer in the file picker.
paths = ["~/logs/**/*.log", "/var/log/**/*.log"]
```

## Source Feeds

Only one feed is active at a time. The active feed is chosen at startup via CLI flags or an interactive `skim` picker. Within a feed, multiple **producers** can be selected simultaneously (e.g. several Kubernetes pods, or several Docker containers); their streams are merged and tagged with the producer name.

| Feed | Producer unit | How logs are read |
|------|---------------|-------------------|
| `docker` | container name/id | `docker logs -f` via subprocess or the Docker API |
| `kubernetes` | pod name (with optional container) | `kubectl logs -f` via subprocess |
| `file` | file path | `inotify`-based tail, handles rotation |
| `stdin` | — | raw stdin, useful for piping |

## Log Normalization

Raw log lines arrive as plain text. The ingestor attempts to parse each line into a structured `LogEntry` before it hits the store. Parsing is attempted in order:

1. **JSON** — if the line is valid JSON, all top-level keys are promoted to searchable fields.
2. **Logfmt** — `key=value` pairs are extracted.
3. **Common patterns** — heuristic regexes detect log levels (`INFO`, `WARN`, `ERROR`, …), timestamps, and request IDs in unstructured text, injecting synthetic fields.
4. **Fallback** — the raw line is stored as `message`, with feed-level metadata (`source`, `producer`, `timestamp`) injected regardless.

Synthetic fields added unconditionally: `source` (feed type), `producer` (container/pod/file name), `ts` (ingest time if not parsed from the line), `level` (best-effort).

## UI Layout

The UI is a full-screen `ratatui` layout with a tab bar at the top, a split body, and a query bar at the bottom:

```
┌──────────────────────────────────────────────────────────────┐
│ [1:main ●] [2:freeze:api-7f9b] [3:correlate:req-abc123]  +  │
├─────────────────────┬────────────────────────────────────────┤
│ ▼ prod-cluster      │                                        │
│   ▼ default         │  Log stream (scrollable, live tail)    │
│     ● api-7f9b4d  ✓ │                                        │
│     ● worker-4c2a ✓ │  [paused — 42 new lines]              │
│     ○ worker-9e1b   │                                        │
│   ▶ kube-system     │                                        │
│ ▶ staging           │                                        │
├─────────────────────┴────────────────────────────────────────┤
│  query: [level:error timeout_______]  greed: [=====-----] 5  │
└──────────────────────────────────────────────────────────────┘
```

### Tab Bar

Each tab is an independent view with its own query state and scroll position, all reading from the shared store. The first tab is always the main live-tail view. Additional tabs are opened by the yank and correlate operations (see below). Tabs can be closed with `q` when focused; the main tab cannot be closed.

### Producer Tree

The left pane is a collapsible tree rather than a flat list. The tree structure depends on the active feed:

| Feed | Level 1 | Level 2 | Level 3 |
|------|---------|---------|---------|
| `kubernetes` | kube context | namespace | pod (with per-container toggle) |
| `docker` | compose project (if any) | container | — |
| `file` | directory | file | — |
| `stdin` | — | — | — |

Selection semantics: selecting a node at any level implicitly selects all its descendants. A namespace selected means all pods in that namespace are tailed. Individual pods can be toggled within a selected namespace. The `✓` / `○` indicators reflect selection state; a partially-selected parent shows `◐`.

`skim` is invoked with `f` to fuzzy-jump to any node in the tree without navigating manually. The tree is otherwise keyboard-navigated with arrow keys / `hjkl`.

### Log Stream

Live-tailing view. Scrolling up pauses the display (lines keep arriving in the store). A banner shows the pause state and number of buffered new lines. `G` jumps back to the tail. Each line is prefixed with its producer name (colour-coded) and timestamp (toggleable). Matched spans from the active query are highlighted inline.

### Query Bar

Accepts the current search expression. Focus with `/`. The greed level sits beside the input, adjusted with `[` / `]`. All keybindings are configurable in `config.toml`.

## Search

Queries are evaluated as a pipeline:

1. **Key filter** — optional `key:value` prefix restricts which fields are scanned (e.g. `level:error message:timeout`).
2. **Term expansion** — each bare term is expanded via the greedy algorithm into a set of candidate terms (see **Greedy Algorithm** below).
3. **FST scan** — expanded terms are looked up against an `fst`-backed index over the store for prefix/infix matches.
4. **Ranking** — matched lines are scored by match density and recency; the stream view highlights the matched spans.

Exact mode (greed = 0) bypasses expansion entirely and does a literal substring or regex match.

## Freeze / Yank

Press `y` with a producer node focused in the tree to open a new tab scoped to that producer alone. The tab is labelled `freeze:<producer-name>` and has its own independent query and scroll state. The main tab continues receiving all selected producers. Freeze tabs are read-only views over the shared store — no data is duplicated, and they backfill instantly from buffered history.

Multiple freeze tabs can be open simultaneously. A common pattern: main tab with `level:error` watching everything, plus a freeze tab on a single noisy pod with a narrow query to isolate its behaviour.

## Correlation

With the cursor on any log line, press `c` to open a field picker (powered by `skim`). Selecting a field opens a new tab pre-filtered to `<field>:<value>`, labelled `correlate:<value>`. The correlated tab searches across **all** producers in the store (not just the ones currently selected in the tree), so cross-service traces surface naturally.

Typical use: correlate on `request_id` to follow a single HTTP request across api, worker, and gateway pods simultaneously. The tab remains live — new lines matching the correlation key continue to arrive in real time.

## Export

The export dialog (triggered from the command palette or a keybinding) presents the following options:

- **Scope** — entire store, active filter only, selected producer only, or selected producer + active filter.
- **Format** — raw lines, JSON-L, or CSV (key-value fields).
- **Destination** — write to a temp file and open in the user's configured editor (`$FML_EDITOR`, falling back to `$EDITOR`, then `vi`).

The export runs in the background so the UI stays responsive.

## Headless / Pipeline Mode

`fml` can run without a TUI, emitting filtered log lines to stdout. This is the primary way to feed logs into an LLM or another CLI tool.

```bash
# tail the last 200 error lines from a kubernetes namespace and send to an LLM
fml --headless --feed kubernetes --namespace default --query "level:error" --tail 200 \
  | llm "summarise these errors and suggest root causes"

# tee to a file while also streaming to an LLM
fml --headless --feed docker --container api --query "timeout" --greed 6 \
  | tee /tmp/api-timeouts.log | llm "what is causing the timeouts?"

# exit after a fixed duration instead of tailing indefinitely
fml --headless --feed file --path ./app.log --duration 30s --query "exception"
```

Headless mode flags (all also settable in `config.toml` under `[headless]`):

| Flag | Description |
|------|-------------|
| `--headless` | Disable TUI, write matching lines to stdout |
| `--query <expr>` | Initial query expression (same syntax as the TUI query bar) |
| `--greed <0-10>` | Greed level for semantic expansion |
| `--tail <n>` | Emit the last N matching lines from the buffer, then exit |
| `--duration <t>` | Run for a fixed duration then exit (e.g. `30s`, `5m`) |
| `--format <fmt>` | Output format: `raw` (default), `jsonl`, `csv` |
| `--no-metadata` | Suppress injected fields (`source`, `producer`, `ts`) |

When stdout is a TTY, headless mode still colourises output. When piped, it emits plain text (or structured `jsonl`/`csv` depending on `--format`).

## Claude Code Integration

Two integration points make `fml` usable from within a Claude Code session.

### MCP Server

`fml` exposes an [MCP](https://modelcontextprotocol.io) server that Claude Code can connect to via the `mcpServers` config. The server provides a single tool:

```json
{
  "name": "fml_query",
  "description": "Query live or buffered logs using fml. Returns matching log lines as JSON.",
  "inputSchema": {
    "feed":      "kubernetes | docker | file | stdin",
    "query":     "fml query expression",
    "greed":     "0-10 (default 4)",
    "tail":      "max lines to return (default 100)",
    "namespace": "kubernetes namespace (kubernetes feed only)",
    "context":   "kubeconfig context (kubernetes feed only)",
    "container": "container name (docker feed only)"
  }
}
```

Start the MCP server with `fml --mcp`. Add it to Claude Code's config:

```json
// ~/.claude/settings.json
{
  "mcpServers": {
    "fml": {
      "command": "fml",
      "args": ["--mcp"]
    }
  }
}
```

Claude Code can then call `fml_query` as a tool to fetch log context during debugging sessions without leaving the editor.

### Agent Skill

A `skills/fml.md` file in this repo defines a Claude Code slash command `/fml` that:

1. Prompts for a feed, query, and greed level if not provided inline.
2. Runs `fml --headless` with those parameters.
3. Passes the output as context to Claude for analysis.

Usage examples from within Claude Code:

```
/fml level:error greed:7
/fml --feed docker --container api timeout
/fml --namespace payments exception --tail 500
```

The skill file lives at `.claude/skills/fml.md` in this repo and is auto-discovered by Claude Code when working inside the project.

## Testing

The test suite is a safety net for Claude and for humans. Every significant
change to the codebase should be accompanied by a passing `cargo test --all`.
The harnesses are stubs today; fill them in as each layer is implemented.

### What "all tests pass" guarantees

| Layer | Harness | Key invariants enforced |
|-------|---------|------------------------|
| Ingestor (Kubernetes) | `kubernetes_harness` | Producer tagging, reconnect, no duplicate lines on retry |
| Ingestor (Docker) | `docker_harness` | Frame decoding, compose naming, stderr tagging |
| Ingestor (File) | `file_harness` | Rotation, truncation, glob, all written lines received |
| Ingestor (Stdin) | `stdin_harness` | EOF behaviour, burst, headless exit |
| Normalizer | `normalization_harness` | Synthetic fields always present, JSON/logfmt/unstructured parsing, snapshots |
| Store | `store_harness` | Ring eviction, monotonic sequence numbers, concurrent safety |
| Search | `search_harness` | **Greed monotonicity** (most critical), all 7 domain families, negative prefix inference, results ⊆ store |
| Export | `export_harness` | All 3 formats × 4 scopes, insta snapshots |
| Headless | `headless_harness` | Process-level flag validation, exit codes, TTY detection |

### What the tests do NOT guarantee

- Correct rendering in a real terminal (the UI layer has no automated tests yet)
- Behaviour with real Kubernetes clusters or Docker daemons
- Performance under production load (use the benchmarks for that)

### Running the tests

```sh
# Full suite
cargo test --all

# Single harness
cargo test --test search_harness

# With output (useful for seeing which planned tests are pending)
cargo test --all -- --nocapture

# Only non-ignored tests (skips all stub tests)
cargo test --all -- --skip ignored

# Run all ignored (stub) tests to see todo! panics — useful for planning
cargo test --all -- --include-ignored
```

### Snapshot tests

Snapshot files live in `tests/snapshots/`. To update them after an intentional
format change:

```sh
cargo test --test normalization_harness
cargo test --test export_harness
cargo insta review    # approve or reject each diff interactively
```

### Benchmarks

```sh
# Run all benchmarks (empty stubs complete instantly)
cargo bench

# Run a specific bench group
cargo bench --bench search_bench

# View HTML report (requires gnuplot)
open target/criterion/report/index.html

# Save a baseline for later comparison
scripts/save_baselines.sh main

# Compare main vs candidate branch (requires: cargo install critcmp)
critcmp main candidate --threshold 5
```

### Expectations for Claude when changing code

When Claude adds or modifies a feature:

1. **Run `cargo test --test <relevant_harness>`** before declaring the task done.
2. **Fill in stub tests** for any behaviour the change touches — a passing
   `#[ignore]` test is not a passing test.
3. **For search changes**, always run `cargo test --test search_harness` and
   explicitly check greed monotonicity holds end-to-end.
4. **For normalizer changes**, run `cargo insta review` and commit updated
   snapshots intentionally — never silently accept snapshot diffs.
5. **For performance-sensitive paths** (normalizer, search pipeline, store
   insert), run `cargo bench` before and after and check for regressions
   with `critcmp`.

## Greedy Algorithm

The search expansion system works in layers, each activated by a **greed level** (1–10). At low greed, only close morphological matches are returned. As greed increases, the search walks further across a semantic graph to find domain-related terms the user didn't explicitly type.

### Layers

| Greed | Layer | Example (`auth`) |
|-------|-------|-----------------|
| 1 | Morphological / prefix | `authenticated`, `authorization` |
| 3 | Synonym / ontology cluster | `login`, `credential`, `session` |
| 7 | Domain peers (1 hop) | `token`, `permission`, `principal` |
| 10 | Domain peers (2+ hops) | `bearer`, `jwt`, `oauth`, `expiry` |

Multi-hop traversal is how distant-but-related terms are reached. `auth → token → bearer` is two hops; at low greed that path is never walked.

### Semantic Graph Structure

The graph is a **directed weighted graph** where edges carry both a relationship type and a weight. The greed slider controls the minimum edge weight traversed and the maximum traversal depth.

```rust
struct TermNode {
    term: String,
    relations: Vec<(RelationType, String, f32)>, // (type, target, weight)
}

enum RelationType {
    Morphological, // auth -> authenticated
    Synonym,       // error -> failure
    DomainPeer,    // auth -> token
    Hypernym,      // unauthorized -> auth
    Implication,   // panic -> crash
}
```

Edges are bidirectional but **not symmetric in weight** — `auth → token` may be 0.8 while `token → auth` is only 0.5, reflecting that "token" is a broader search context than "auth".

### Ontology Definition

Clusters are defined in a static data file (compiled in via `include_str!` or `phf`) and cover the finite vocabulary of application logs. Approximately 150–200 terms across ~10 domain families covers the vast majority of real-world log patterns.

```toml
[[cluster]]
seed = "auth"
morphological = ["authenticate", "authenticated", "authentication", "authorized", "authorization"]
synonyms      = ["login", "signin", "credential"]
domain_peers  = [
    { term = "token",      weight = 0.8 },
    { term = "session",    weight = 0.8 },
    { term = "permission", weight = 0.7 },
    { term = "principal",  weight = 0.6 },
    { term = "identity",   weight = 0.6 },
]

[[cluster]]
seed = "token"
morphological = ["tokens"]
synonyms      = ["bearer", "credential", "secret"]
domain_peers  = [
    { term = "jwt",     weight = 0.9 },
    { term = "oauth",   weight = 0.8 },
    { term = "api_key", weight = 0.7 },
    { term = "expir",   weight = 0.6 }, # prefix — catches "expiry", "expired"
]
```

Broad domain families to cover:

- **auth** — login, token, session, bearer, jwt, oauth, permission, role, credential, expiry
- **error** — exception, failure, panic, fatal, crash, abort, stacktrace, caused_by
- **network** — timeout, connection, refused, socket, retry, unreachable, dns, tls, handshake
- **database** — query, deadlock, constraint, migration, transaction, rollback, pool
- **performance** — slow, latency, elapsed, duration, threshold, spike, queue, backpressure
- **lifecycle** — startup, shutdown, init, ready, healthy, degraded, restart, reload
- **resource** — oom, memory, disk, cpu, limit, exhausted, leak, gc, allocation

### Negative Prefix Inference

When the query begins with a negative prefix (`un-`, `fail-`, `err-`, `invalid-`, `no-`), the traversal automatically biases toward the **error** and **failure** clusters, without needing every negative form explicitly encoded.

```
"unauth"
  → prefix scan matches "unauthorized", "unauthenticated"
  → both are morphological children of seed "auth"
  → graph-walk proceeds from "auth"
  → negative prefix detected: weight-boost edges toward "error" / "failure" clusters
```

This means `unauth` at high greed naturally surfaces terms like `forbidden`, `rejected`, `denied`, and `401` — none of which needed to be explicitly linked to `unauth` in the ontology.

### Backwards resolution

The greedy algorithm must be backward-resolution possible, such that auth can be resolved from something like "expiry". Based on the greedy factor, auth may grab "expiry" on a greed value of `5`, we may need to set the greed value to `9` when we supply "expiry", to resolve back to auth.

### Rust Implementation Notes

- **Prefix/trie lookup:** [`fst`](https://crates.io/crates/fst) crate — memory-mapped finite state transducer, fast prefix scans
- **Static ontology map:** [`phf`](https://crates.io/crates/phf) — perfect hash function, compile-time static maps with zero runtime allocation
- **Graph traversal:** standard BFS over the cluster graph, gated by `(depth <= max_depth) && (weight >= min_weight)` where both thresholds are derived from the greed level
