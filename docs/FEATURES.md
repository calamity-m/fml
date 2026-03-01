# Features

## UI Layout

```
┌──────────────────────────────────────────────────────────────┐
│ [1:main ●] [2:freeze:api-7f9b] [3:correlate:req-abc123] q:quit  ?:help │
├─────────────────────┬────────────────────────────────────────┤
│ ▼ prod-cluster      │                                        │
│   ▼ default         │  Log stream (scrollable, live tail)    │
│     ● api-7f9b4d  ✓ │                                        │
│     ● worker-4c2a ✓ │  [paused — 42 new lines]              │
│     ○ worker-9e1b   │                                        │
│   ▶ kube-system     │                                        │
│ ▶ staging           │                                        │
├─────────────────────┴────────────────────────────────────────┤
│  query: [level:error timeout_______]    greed:[=====-----]5  │
└──────────────────────────────────────────────────────────────┘
```

## Keybindings

| Key | Action |
|-----|--------|
| `Tab` | Cycle focus: Tree → Stream → Query → Tree |
| `/` | Jump focus to query bar |
| `?` | Toggle help popup |
| `:` | Open vim-style command bar |
| `q` | Quit (or close current non-main tab) |
| `G` | Jump to live tail |
| `[` / `]` | Decrease / increase greed (works from any pane) |
| `↑`/`k`, `↓`/`j` | Navigate up/down |
| `←`/`h`, `→`/`l` | Collapse / expand tree node |
| `Space` | Toggle producer selection |
| `Enter` | Toggle selection (leaf) or expand/collapse (parent) |
| `PageUp`/`Ctrl+u` | Scroll up one page |
| `PageDown`/`Ctrl+d` | Scroll down one page |

## Command bar (`:`)

Press `:` from any pane (except the query bar) to open the vim-style command line.

| Command | Effect |
|---------|--------|
| `q`, `quit` | Quit (close current tab if non-main) |
| `q!`, `quit!` | Force quit |
| `?`, `help` | Toggle help popup |
| `theme <name>` | Switch theme (`default`, `gruvbox`) |
| `ts`, `timestamps` | Toggle timestamp display |
| `tail` | Jump to live tail |
| `greed <0-10>` | Set greed level directly |

## Producer Tree

The left pane is a collapsible tree. Structure depends on the active feed:

| Feed | Level 1 | Level 2 | Level 3 |
|------|---------|---------|---------|
| `kubernetes` | kube context | namespace | pod |
| `docker` | compose project | container | — |
| `file` | directory | file | — |
| `stdin` | — | — | — |

Selection indicators: `✓` selected, `○` unselected, `◐` partially selected.

Selecting a parent node implicitly selects all its descendants. Toggling a child bubbles the new state up through all ancestors.

## Log Stream

Live-tailing view. Scrolling up pauses the display; lines keep arriving in the store. A banner shows pause state and buffered line count. `G` resumes live tail.

Each line is prefixed with its producer name (colour-coded per producer, stable across restarts) and an optional timestamp.

## Freeze / Yank

Press `y` with a producer node focused to open a new tab scoped to that producer alone. The tab is labelled `freeze:<producer-name>` and has its own independent query and scroll state. The main tab continues receiving all selected producers.

Freeze tabs are read-only views over the shared store — no data is duplicated, and they backfill instantly from buffered history.

Common pattern: main tab with `level:error` watching everything, freeze tab on a single noisy pod with a narrow query.

## Correlation

With the cursor on any log line, press `c` to open a field picker. Selecting a field opens a new tab pre-filtered to `<field>:<value>`, labelled `correlate:<value>`. The correlated tab searches across **all** producers (not just the currently selected ones), so cross-service traces surface naturally.

Typical use: correlate on `request_id` to follow a single HTTP request across api, worker, and gateway pods simultaneously.

## Export

The export dialog presents:

- **Scope** — entire store, active filter only, selected producer only, or selected producer + active filter.
- **Format** — raw lines, JSON-L, or CSV.
- **Destination** — temp file opened in `$FML_EDITOR` → `$EDITOR` → `vi`.

Export runs in the background; the UI stays responsive.

## Headless / Pipeline Mode

```bash
# Tail last 200 error lines and send to an LLM
fml --headless --feed kubernetes --namespace default --query "level:error" --tail 200 \
  | llm "summarise these errors"

# Tee to file and stream to LLM simultaneously
fml --headless --feed docker --container api --query "timeout" --greed 6 \
  | tee /tmp/api-timeouts.log | llm "what is causing the timeouts?"

# Exit after fixed duration
fml --headless --feed file --path ./app.log --duration 30s --query "exception"
```

When stdout is a TTY, headless mode colourises output. When piped, it emits plain text (or structured `jsonl`/`csv`).

## Claude Code Integration

### MCP Server

```json
// ~/.claude/settings.json
{
  "mcpServers": {
    "fml": { "command": "fml", "args": ["--mcp"] }
  }
}
```

Start with `fml --mcp`. The server exposes a single `fml_query` tool that accepts `feed`, `query`, `greed`, `tail`, and feed-specific parameters.

### `/fml` Skill

The `.claude/skills/fml.md` file defines a `/fml` slash command for use within Claude Code sessions:

```
/fml level:error greed:7
/fml --feed docker --container api timeout
/fml --namespace payments exception --tail 500
```
