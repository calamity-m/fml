# fml — Feed Me Logs

> *The log triage tool you open when something is already broken. Built for high-stress, time-pressured moments — multi-source ingestion, semantic search that thinks ahead of you, and a UI that gets out of the way so you can find the line that matters before the incident gets worse.*

---

(yes, it also stands for what you think it stands for.)

---

## What it does

`fml` aggregates live log streams from multiple sources — Kubernetes, Docker, files, stdin — normalises them into a structured format, and lets you search, filter, and triage at speed. The core idea: by the time you open this tool, something is already on fire. Every design decision prioritises getting you to the relevant line as fast as possible.

Key features:

- **Multi-source ingestion** — tail Kubernetes namespaces, Docker containers, or local files simultaneously, with streams merged and tagged by producer.
- **Semantic search** — a greedy search algorithm expands your query across a domain ontology (auth, error, network, database, performance, …) so you find related terms you didn't think to type. Greed level is adjustable from exact-match to maximum expansion.
- **Structured normalisation** — JSON, logfmt, and common unstructured patterns are parsed on ingest, injecting searchable fields (`level`, `ts`, `producer`, `source`, …).
- **Tabs** — freeze a single producer into its own tab, or open a correlation tab locked to a field value (e.g. `request_id`) to trace a request across services.
- **Headless / pipeline mode** — run without a TUI and pipe filtered output directly to an LLM, `tee`, or any other tool.
- **Claude Code integration** — MCP server and `/fml` agent skill for querying logs from within a Claude Code session.

## Install

```bash
cargo install fml
```

## Quick start

```bash
# interactive TUI, kubernetes feed
fml --feed kubernetes

# headless — pipe errors to an LLM
fml --headless --feed kubernetes --namespace default --query "level:error" --tail 200 \
  | llm "summarise these errors and suggest root causes"
```

## Configuration

`fml` reads `~/.config/fml/config.toml` on first run and creates it with defaults if absent. See `CLAUDE.md` for the full schema.

## Keybindings (defaults)

| Key | Action |
|-----|--------|
| `/` | Focus query bar |
| `[` / `]` | Decrease / increase greed level |
| `Tab` | Cycle tabs |
| `y` | Yank focused producer into a freeze tab |
| `c` | Open correlation picker on current log line |
| `e` | Export dialog |
| `f` | Fuzzy-jump to producer in tree |
| `G` | Jump to tail (resume live scroll) |
| `q` | Close current tab |

All keybindings are configurable in `config.toml`.

## Why "fml"

Because if you're opening this, something is probably fucked and you are not having a good time. The tool should at least make the next few minutes slightly less awful.

*Feed Me Logs.* Sure. Let's go with that.
