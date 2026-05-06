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

## Source Selector

Press `ctrl+s` to open or close the source selector popup. The selector is currently hardcoded to `ctrl+s`; user-configurable remapping is deferred. Some terminals reserve `ctrl+s` for XOFF flow control, so disable terminal flow control or remap it at the terminal level if the popup does not open.

Sources are organized as `Producer -> Group -> Display Name`. The `producer` field is the top-level row users recognize, such as `file`, `docker`, or a Kubernetes namespace. The optional `group` field is the second-level bucket, such as a Docker compose project, deployment group, path category, or `(ungrouped)` when no group exists. Display names are labels only; source IDs remain the identity used for filtering.

Use `up`/`down` or `k`/`j` to move through rows. Press `space` to toggle the highlighted source, group, or producer. Press `a` to enable all sources in the open selector snapshot, `n` to disable them, and `esc` or `enter` to close. Tail, history, and fuzzy searches all use the enabled source set. Disabling every source is allowed and shows a `No sources selected` empty state in the log pane.

## Fuzzy Search

Typing in the query box dispatches a debounced fuzzy search over the retained log store. The default matcher is `nucleo`; set `search.fuzzy_matcher = "frizbee"` to use the previous frizbee matcher. `search.fuzzy_max_typos` is only used by frizbee, and can be set to `0` for exact fuzzy-character matching or left unset for the library default.

Nucleo queries use nucleo's parsed pattern syntax. Plain words are fuzzy positive terms, a leading apostrophe requires a contiguous substring match such as `'timeout`, and a leading `!` excludes entries matching that term, such as `error !debug`. Negative-only queries return retained entries that do not match the excluded term, newest first.

Fuzzy searches scan a snapshot of the current retained bounds and emit partial ranked results while that snapshot is incomplete. If new logs arrive during a long scan, they do not appear in the in-flight snapshot; after the snapshot emits `complete = true`, the worker notices the changed retained bounds on the next tick and performs a fresh scan. Source filtering is applied when each scan snapshot is built, so partial progress totals, final results, and later re-scans all use the enabled source set.

## TODOs
