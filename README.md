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

## Line Wrap

By default the log pane renders each entry on a single line and clips text past the right edge. Press `w` to toggle wrapped mode, in which long `msg` text wraps onto continuation lines indented under the `msg` column. Setting `[tui] line_wrap = true` (or `FML__TUI__LINE_WRAP=true`) makes wrapped mode the startup default. The custom binding can be remapped via `[tui.keybindings] toggle_line_wrap = ["..."]`.

## Keybindings

Configurable actions are remapped under `[tui.keybindings]`. Each action takes a
list of key specs; the first spec is the primary label shown in the help popup
(`?`) and status bar, and an empty list disables the action.

```toml
[tui.keybindings]
toggle_help            = ["?"]
toggle_source_selector = ["ctrl+s"]
toggle_preview_mode    = ["ctrl+p"]
show_info              = ["i"]
scroll_head            = ["g", "home"]
scroll_tail            = ["G", "end"]
toggle_select_mode     = ["f2"]
yank_selected_entry    = ["y"]
toggle_line_wrap       = ["w"]
```

A key spec is a lowercase string like `"?"`, `"enter"`, `"ctrl+s"`, `"pgdn"`, or
`"f2"`. Optional `ctrl`/`alt`/`shift` modifier prefixes are joined with `+`;
printable characters are written directly and keep their case (`"G"` is shift-g).

`ctrl+c` (quit), `tab` (cycle focus), `esc` (close popup), and the `j`/`k`/arrow
log-navigation keys are reserved fallbacks and are not remappable. An invalid key
spec aborts startup with a keybinding error.

## Log Producers

Attach log sources with the repeatable `--producer KIND[:ARG]` flag:

```sh
cargo run -p fml -- --producer demo
cargo run -p fml -- --producer file:/var/log/app.log
cargo run -p fml -- --producer docker
cargo run -p fml -- --producer kubernetes
cargo run -p fml -- --producer kubernetes:my-namespace
```

Multiple producers can run together:

```sh
cargo run -p fml -- \
  --producer file:/var/log/app.log \
  --producer docker \
  --producer kubernetes:my-namespace
```

Supported kinds:

- `demo` starts a synthetic demo source. Repeat it to create multiple demo sources.
- `file:<path>` tails one file from EOF and follows common rotation patterns.
- `docker` discovers currently running containers, tails stdout/stderr, and tracks container start/stop events. Requires access to the local Docker daemon.

  > **Note on Docker log delivery:** the Docker daemon delivers container logs in batches over its HTTP API, not as a continuous byte stream. On native Linux this batching is usually small enough to feel real-time. On WSL2 / Docker Desktop the additional VM boundary amplifies it noticeably — high-volume containers may appear to pause for 1–2 seconds and then deliver thousands of entries at once. This is a property of the Docker / bollard log stream and cannot be smoothed out in the producer.

- `kubernetes[:namespace]` watches running pod containers in one namespace and tails each container. Without `:namespace`, fml uses the active kubeconfig context namespace, falling back to `default`. Requires a working local kubeconfig; in-cluster service-account config and explicit context selection are not currently supported.

If a producer cannot be constructed, fml logs a warning and continues with the other producers.
Repeated identical real producers are allowed but may duplicate log entries.

For local showcase setups, see `examples/file` and `examples/docker`.

## Profiles

A profile is a named bundle of producers in `config.toml`. Activate one at startup with `--profile <name>`, or set `profile = "<name>"` in config:

```toml
profile = "dev"

[[profiles.dev.producers]]
type = "demo"

[[profiles.dev.producers]]
type = "kubernetes"
namespace = "team-a"
blocked = "^istio"

[[profiles.dev.producers]]
type = "docker"
skip_istio = true
```

Profile keys:

- `type` is one of `demo`, `file`, `docker`, `kubernetes`.
- `file` requires `file = "<path>"`.
- `kubernetes` accepts an optional `namespace`. Multiple kubernetes entries with different namespaces are allowed.
- `demo` is repeatable.
- `docker` may appear at most once per profile (a second is a config error).
- `blocked = "<regex>"` and `skip_istio = true` are accepted on `docker` and `kubernetes` entries.

If `--profile` (or `config.profile`) names a profile that doesn't exist, fml aborts with the available profile names.

### `--producer` precedence over a profile

When combined with `--profile`, each `--producer` matches profile entries by `(kind, disambiguator)`:

| CLI                         | Matches                                                  | Behavior                                |
| --------------------------- | -------------------------------------------------------- | --------------------------------------- |
| `--producer demo`           | nothing (demo is repeatable)                             | append a new demo                       |
| `--producer file:<path>`    | profile `file` entry with the same path string           | replace; otherwise append               |
| `--producer docker`         | the single profile `docker` entry                        | replace; otherwise append               |
| `--producer kubernetes:<n>` | profile `kubernetes` entry whose namespace equals `<n>`  | replace; otherwise append               |
| `--producer kubernetes`     | the unique profile `kubernetes` entry, if exactly one    | replace; ambiguous (>1) is a hard error |

A `--producer` override is a brand-new producer config block — it drops the matched profile entry's `blocked` / `skip_istio`. For durable settings, prefer config + `--profile`. Use `--producer` for ad-hoc overrides.

## Source blocking

Per-producer source filtering. Configured under each docker or kubernetes producer entry:

```toml
[[profiles.dev.producers]]
type = "kubernetes"
namespace = "team-a"
blocked = "^istio"     # regex matched against source.id OR source.display_name
skip_istio = true      # shortcut: substring-matches "istio-proxy"
```

Match semantics:

- `blocked` is a regex tested against both `source.id` and `source.display_name`. A match against either field blocks the source.
- `skip_istio` (or the global `--skip-istio`) adds the literal `istio-proxy` substring to the matcher. Substring match handles real ids like `istio-proxy-abc123` and pod-prefixed forms like `productpage/istio-proxy`.
- Both compose; neither overrides the other.

`--skip-istio` on the CLI ORs `skip_istio = true` into every kubernetes and docker producer in effect. It is additive, never subtractive: a profile entry already setting `blocked = "^istio"` plus `--skip-istio` results in both the regex and the literal substring being active. `--skip-istio` is a no-op for `file` and `demo` producers.

Blocked sources emit no events at all — no `SourceFound`, no `SourceLost`, no log entries. Block configuration is **static for the process lifetime**: there is no runtime mutation API and no UI affordance to unblock. To change blocking, restart with a different profile or CLI flags.

If `blocked` is an invalid regex, that one producer is skipped with a warning and the rest of the producers still start.

## Source Selector

Press `ctrl+s` to open or close the source selector popup. The binding is remappable via `[tui.keybindings] toggle_source_selector = ["..."]`. Some terminals reserve `ctrl+s` for XOFF flow control, so either remap it here, disable terminal flow control, or remap it at the terminal level if the popup does not open.

Sources are organized as `Producer -> Group -> Display Name`. The `producer` field is the top-level row users recognize, such as `file`, `docker`, or a Kubernetes namespace. The optional `group` field is the second-level bucket, such as a Docker compose project, deployment group, path category, or `(ungrouped)` when no group exists. Display names are labels only; source IDs remain the identity used for filtering.

Use `up`/`down` or `k`/`j` to move through rows. Press `space` to toggle the highlighted source, group, or producer. Press `a` to enable all sources in the open selector snapshot, `n` to disable them, and `esc` or `enter` to close. Tail, history, and fuzzy searches all use the enabled source set. Disabling every source is allowed and shows a `No sources selected` empty state in the log pane.

## Fuzzy Search

Press `/` to focus the query box and start typing a search term. Fuzzy search updates as you type. Press `Enter` to return focus to the log pane and resume navigation without clearing the active query. Backspace and delete edit the query while the query box is focused.

Typing in the query box dispatches a debounced fuzzy search over the retained log store. The default matcher is `nucleo`; set `search.fuzzy_matcher = "frizbee"` to use the previous frizbee matcher. `search.fuzzy_max_typos` is only used by frizbee, and can be set to `0` for exact fuzzy-character matching or left unset for the library default.

Nucleo queries use nucleo's parsed pattern syntax. Plain words are fuzzy positive terms, a leading apostrophe requires a contiguous substring match such as `'timeout`, and a leading `!` excludes entries matching that term, such as `error !debug`. Negative-only queries return retained entries that do not match the excluded term, newest first.

Fuzzy searches scan a snapshot of the current retained bounds and emit partial ranked results while that snapshot is incomplete. If new logs arrive during a long scan, they do not appear in the in-flight snapshot; after the snapshot emits `complete = true`, the worker notices the changed retained bounds on the next tick and performs a fresh scan. Source filtering is applied when each scan snapshot is built, so partial progress totals, final results, and later re-scans all use the enabled source set.

## TODOs
