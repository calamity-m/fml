# Configuration

`fml` reads `~/.config/fml/config.toml` on startup. The path can be overridden with `--config <path>`. If the file does not exist it is created with defaults. CLI flags take precedence over the config file.

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

[headless]
# Defaults for headless/pipeline mode flags.
format = "raw"       # raw | jsonl | csv
greed  = 4
tail   = 100
```

## Command-line flags

| Flag | Description |
|------|-------------|
| `--feed <name>` | Feed to open (`docker`, `kubernetes`, `file`, `stdin`) |
| `--config <path>` | Override config file path |
| `--query <expr>` | Initial query expression |
| `--greed <0-10>` | Greed level |
| `--debug` | Write debug logs to `/tmp/fml-debug.log` |
| `--headless` | Disable TUI, write matching lines to stdout |
| `--tail <n>` | Emit last N matching lines then exit |
| `--duration <t>` | Run for fixed duration then exit (`30s`, `5m`) |
| `--format <fmt>` | Output format for headless mode: `raw`, `jsonl`, `csv` |
| `--no-metadata` | Suppress injected fields in headless output |
| `--mcp` | Start MCP server instead of TUI |
