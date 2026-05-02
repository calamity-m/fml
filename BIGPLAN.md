# BIGPLAN: Real Producer Functionality (Docker, Kubernetes, File)

## Plan Overview

Today the producer module is mostly scaffolded — `FakeProducer` works (powering `--demo`), `DockerProducer` and `KubernetesProducer` exist as structs without working `LogProducer` impls, `producer/file.rs` is empty, and the normalizer panics via `todo!()` if called. This effort makes file, docker, and kubernetes producers real and selectable from the CLI via a repeatable `--producer KIND[:ARG]` flag, replacing the `--demo` flag with `--producer demo`. "Done" means a user can run `fml --producer file:/var/log/app.log --producer docker --producer kubernetes:my-ns` and see live log entries from all three sources flowing through the same source-selector tree, with the normalizer extracting `msg`/level/timestamp from JSON and pattern-matched lines.

## Risks

- **bollard log frame demultiplexing** — When a container is started without a TTY, docker's log API multiplexes stdout/stderr by prefixing each chunk with an 8-byte header. Reading the byte stream as plain text yields garbage. Mitigation: use bollard's `LogOutput` enum (it pre-decodes frames) and consume `into_bytes()` per item, not the raw HTTP body.
- **File rotation edge cases** — `notify` events differ across rotation strategies (atomic rename vs copy-truncate vs delete+recreate), and rapid rotation can fire `Remove` before `Create` is observable. Mitigation: keep the watcher on the *parent directory* of the target path so we see the recreate, and re-stat + reopen on any `Remove`/`Rename` for the watched path. Document the supported strategies in the file producer's module doc.
- **kube `log_stream` connection drops without watcher re-emit** — `kube::runtime::watcher` reconnects on stream drops, but per-pod `log_stream` calls do not, and a transient network blip can drop the log stream while the pod stays healthy (so the watcher won't re-emit it). Without intervention, logs from that pod silently stop until the pod is `Modified` for some unrelated reason. Mitigation: each `tail_pod_container` subtask reconnects on stream close (while still in `tracked` and uncancelled) using exponential backoff — 100ms → 1s → 10s, capped at 10s, reset on the first successful read after a reconnect. This is live-tail robustness, not guaranteed catch-up: lines emitted while disconnected can still be missed because this plan does not track `since_time`/timestamps for replay.
- **docker daemon unreachable / events stream drops** — `Docker::connect_with_local_defaults()` fails when `/var/run/docker.sock` is missing or the daemon is down, and the `events()` stream can drop mid-session if the daemon restarts. Without explicit handling, fml panics or silently stops emitting. Mitigation: surface connection failures at construction (`DockerProducer::new` returns `Err(ProducerError::Docker(...))`); `App::new` logs and skips a failed docker producer so other producers keep running. Mid-session `events()` drops emit a `tracing::warn!` and let the spawned task exit; we do not auto-reconnect at this stage (deferred follow-up).
- **crate-version API assumptions** — Pseudo-code and gotchas assert specific shapes for `bollard 0.20` (`LogOutput` variants, `events()` filter `HashMap<String, Vec<String>>`), `kube::runtime::watcher` events (`Applied`/`Modified`/`Deleted`/`Restarted`), `Pod::log_stream` signature, and `notify::RecommendedWatcher`'s callback model. If the workspace's actual crate versions differ, three deliverables' pseudo-code becomes invalid simultaneously. Mitigation: as the first task of each producer deliverable, verify the crate version + features in `fml/Cargo.toml` and reconcile the gotchas/pseudo-code before writing implementation.
- **real-world log file robustness (non-UTF-8, unbounded lines)** — Production log files contain non-UTF-8 bytes (binary, mixed encodings, partial multi-byte sequences across reads) and pathologically long single lines (multi-MB single lines from poorly-behaved apps). Reading naively crashes or OOMs. Mitigation: every producer's line buffer uses lossy UTF-8 decoding (`String::from_utf8_lossy`) and truncates lines longer than 64 KiB with a `... [truncated]` marker. Applies uniformly to file, docker, and kube producers.
- **kubernetes deployment context is narrow** — `Kubeconfig::read()` only honours `KUBECONFIG` env + `~/.kube/config`. In-cluster service-account configs (running fml from inside a pod), explicit `--context` overrides via flag, and multi-cluster federation are out of scope for this BIGPLAN. Mitigation: state this explicitly in the kube producer's module docs and Deliverable 5; expansion to `kube::Config::infer()` is a follow-up.
- **Removing `--demo` is a breaking CLI change** — Anyone who has `fml --demo` muscle-memorised will get a parse error. Mitigation: this is a single-user project at present, and the new form is short (`fml --producer demo`). Surface in commit message; no compat shim.
- **Source-id collisions across repeated producer flags** — `--producer demo --producer demo` and repeated real producers can create multiple producer instances. If emitted sources share IDs, `handle_producer_event` silently dedups by `Source.id` even when `Source.producer` differs. Mitigation: enumerate demo source IDs (`demo-1`, `demo-2`, …) at CLI-parse time; for kube, include namespace in the source id (`{namespace}/{pod}/{container}`) so two namespaces may contain the same pod/container names safely. Repeated identical docker/file/kube specs remain tolerated and can still duplicate log entries (tracked in Issues).

## Plan Details

### Critical Files

- `fml/src/main.rs` — `Cli` struct gains a repeatable `--producer` `Vec<String>`; loses `--demo`. `App::new` signature changes accordingly.
- `fml/src/app.rs` — `App::new` takes a parsed `Vec<ProducerSpec>` and builds `Box<dyn LogProducer>` instances. `demo_sources()` becomes obsolete.
- `fml/src/producer.rs` — re-exports for new producer kinds plus the `ProducerSpec` enum (the parsed CLI shape).
- `fml/src/producer/file.rs` — currently empty; fill with `FileProducer` (tail-from-EOF + notify-based rotation handling).
- `fml/src/producer/docker.rs` — add `LogProducer` impl driving a list + `events()` watch loop.
- `fml/src/producer/kubernetes.rs` — replace `todo!()` with watcher-driven pod-container discovery + per-container `Pod::log_stream` tail with reconnect-on-close.
- `fml/src/producer/fake.rs` — `FakeProducer::new` already takes a `Source`; CLI `--producer demo` constructs one with a synthetic id (`demo-1`, …) per occurrence.
- `fml/src/producer/normalizer/json.rs` — replace `todo!()` at `msg:` with `obj.get("msg").or(obj.get("message")).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| raw.to_string())`.
- `fml/src/producer/normalizer/pattern.rs` — replace `todo!()` at `msg:` with `raw.to_string()`.
- `fml/src/error.rs` — `ProducerError` grows variants for CLI parse errors, IO, notify, and kube paths. Audit done in Deliverable 1.
- `fml/Cargo.toml` — add `testcontainers` as a `dev-dependencies` entry; ensure `tokio-util` is present with default features (for `CancellationToken`); confirm `notify`, `bollard`, `kube`, `kube-runtime`, `k8s-openapi` are all declared with the features the producers need.
- `fml/src/config.rs` — audit only; we do not add config-file producer specification in this BIGPLAN, but we confirm no existing config field collides with the new CLI flow.

### Gotchas

- `LogProducer::start(&self, tx)` is **synchronous** and must return promptly. Long-running ingest belongs in a `tokio::spawn` inside `start`. Each producer's `start` must return within ~50ms even when the underlying source (file path, docker socket, kube API) is unreachable — we test this explicitly.
- **Cancellation pattern**: each producer holds a `CancellationToken` (from `tokio_util::sync::CancellationToken`) field. `start` clones it into the spawned root task; the root task creates per-subtask child tokens via `token.child_token()` so individual containers/pods can be cancelled independently when they retire (e.g. on docker `die` or kube `Deleted`). `LogProducer::stop` cancels the parent, which propagates to all children. `SourceLost` is emitted by the task that owns source lifecycle: file root task for its file source, docker event loop for container death/destruction, and kube watcher/reconcile logic for pod/container removal. `stop` itself only cancels; it does not need to emit `SourceLost` during app shutdown.
- Producers **must** emit `ProducerEvent::SourceFound` before any `StoreEvent` referencing that source — `handle_producer_event` adds new sources to `state.producer.sources` *and* enables them in the source-selector. An entry referencing an unknown source will store but won't be filterable until the source arrives.
- The `producer` field on `Source` is documented as "Top-level grouping key for the source selector tree (Producer → Group → Display Name)". For kubernetes we set `producer = namespace`, intentionally departing from "the producer kind name" — this gives users a namespace-scoped tree. For docker/file/demo, `producer = "docker"`/`"file"`/`"fake"`. The mixed semantic categories in this key are a known wart; tracked in Issues.
- Source IDs must be **stable and globally unique across live producers** so multi-source filters in tail/history/fuzzy stay meaningful. For docker, use the long container ID, not the name (names can be changed by `docker rename`). For kube, `{namespace}/{pod}/{container}` avoids collisions when multiple namespaces contain the same pod/container names. For files, use the canonicalized absolute path when the file exists; if it does not exist yet, absolutize against the current working directory so the id does not change after the file appears.
- `notify::RecommendedWatcher` is **callback-based, not stream-based**. Bridge it with a bounded `tokio::sync::mpsc::channel(64)` whose sender lives in the watcher's callback closure; the spawned task `select!`s on the receiver alongside the cancellation token. Bound the channel so a flood of FS events back-pressures rather than allocates unbounded.
- `notify::RecommendedWatcher` should watch the **parent directory** of the target file, not the file itself, or rotation deletes blind it. Filter incoming events to those whose `paths` contain the target.
- bollard 0.20's `Docker::logs(...)` returns a `Stream<Item = Result<LogOutput, Error>>` where `LogOutput` is `StdOut(Bytes)` / `StdErr(Bytes)` / `StdIn(Bytes)` / `Console(Bytes)`. Consume via `match` and treat any of stdout/stderr as a log line — combine into the same source.
- bollard log lines come bytes-at-a-time, not lines-at-a-time. Buffer until a `\n` is seen before emitting one `StoreEvent`. Same for kube `log_stream` (which is line-oriented when wrapped with `BufReader::lines`, so use that wrapper).
- **Bollard "die-before-start" race**: if we list running containers then subscribe to `events()`, containers that start in the gap are missed and containers that die in the gap may produce a `SourceLost` for an id we never saw. Fix: subscribe to `events()` *before* listing, buffer events seen in the gap, then dedup against the listed set by container id before emitting `SourceFound`. Document the chosen reconciliation in the docker producer.
- Repeated identical `--producer` flags (`--producer docker --producer docker`) construct two producer instances. They will both emit `SourceFound` for the same container ID; `handle_producer_event` silently dedups by `id`, so the second one's sources get dropped while its stream still pumps duplicate `StoreEvent`s. Document this as known behaviour rather than dedup at parse time — it's the simplest path and the failure mode is "you get double the entries".
- `--producer file:` parser uses `splitn(2, ':')` (not `split_once` semantics that lose the rest, and not `split(':')`) so paths containing `:` (e.g. timestamped log dirs `/var/log/2026-05-02:12:00.log` or Windows-style paths) are taken whole as the path. Unit test covers this.
- **Per-line decoding policy**: every producer normalizes line-by-line through `String::from_utf8_lossy(...)` over the buffered bytes. Lines longer than `64 * 1024` bytes are truncated and have `... [truncated]` appended before going to the normalizer. Applies uniformly to file/docker/kube. Add a small shared helper if the duplication itches; otherwise inline it.
- `pattern.rs`'s `todo!()` sits inside the construction of the `Some(NewLogEntry { ... })` branch; the function still has a separate `if level.is_none() && ts.is_none() { return None; }` early-return that **must be preserved unchanged**. Same applies to `json.rs` — `todo!()` is only reached when JSON parses successfully, so the `serde_json::from_str(raw).ok()?` propagation stays intact.
- `kube::config::Kubeconfig::read()` reads from the standard locations (`KUBECONFIG` env, `~/.kube/config`). The active context's namespace is at `contexts[active].context.namespace` and is `Option<String>`. Falling back to `"default"` literal preserves `kubectl` semantics. In-cluster contexts and `--context` flag overrides are out of scope (see Risk).
- Kubernetes init-container scope is **currently-running only**. A completed init container can have historical logs, but it will not have `state.running.is_some()` and is out of scope for this live-tail plan. If users need completed init logs, add a follow-up that tails terminated init containers once without `follow=true`.
- Kubernetes reconnects use `tail_lines: Some(0)` on reopen. That prevents duplicate backfill but means lines written during a dropped connection can be missed; fixing that requires timestamps/since-time bookkeeping and is deliberately out of scope for this pass.
- bollard's `Docker::events(...)` filter syntax: `HashMap<String, Vec<String>>` with keys like `type` and `event`. Filter to `type=container, event=start|die|destroy` so we don't get drowned by image-pull events.
- The current `DockerProducer::list_running_containers` already filters by `status=running` but also passes `.all(true)` — that's contradictory. Set `.all(false)` (or drop it) and rely on the status filter, since `all=true` returns stopped containers too.
- `ContainerSummary::names` is `Option<Vec<String>>`. Fallback chain for `display_name`: first name without `/` → if no names, the short container id (`&id[..12]`).
- testcontainers integration tests **require a running docker daemon**. The `integration` feature gate marks the tests as opt-in; it does not soft-skip when docker is missing. Running `cargo test --features integration` without docker fails loudly. Document this in the test module's preamble.
- Demo source enumeration: when the CLI sees N `--producer demo` occurrences, mint `Source { id: "demo-N", display_name: "Demo N", producer: "fake", group: None }` — preserves the existing FakeProducer's `producer = "fake"` string.

### Pseudo-code / Sketches

#### `--producer` parser

```text
ProducerSpec ::= Demo | File(PathBuf) | Docker | Kubernetes(Option<String>)

parse(s: &str) -> Result<ProducerSpec, ProducerError::Cli>:
    let (kind, arg) = match s.splitn(2, ':').collect::<Vec<_>>().as_slice():
        [k] => (*k, None)
        [k, a] => (*k, Some(*a))
    match (kind, arg):
        ("demo", None)             -> Demo
        ("file", Some(path))       -> File(PathBuf::from(path))
        ("docker", None)           -> Docker
        ("kubernetes", None)       -> Kubernetes(None)            # active context ns
        ("kubernetes", Some(""))   -> Err(empty namespace)
        ("kubernetes", Some(ns))   -> Kubernetes(Some(ns))
        _                          -> Err(unknown kind / arg shape)
```

`build_producers(specs: Vec<ProducerSpec>) -> Vec<Box<dyn LogProducer>>` enumerates demo specs to mint unique IDs and constructs each producer. Construction failures (e.g. docker daemon unreachable) are logged at warn-level and skipped, so a partial set of producers can still run.

#### File producer task loop

```text
spawn:
    source_path = canonicalize(path).or_else(|_| absolutize_against_cwd(path))
    source = Source { producer: "file", id: source_path, display_name: file_name, group: parent_dir, ... }
    emit SourceFound(source)

    let (notify_tx, mut notify_rx) = mpsc::channel(64)
    let watcher = notify::recommended_watcher(move |evt| { let _ = notify_tx.try_send(evt); })
    watcher.watch(parent_dir(source_path), RecursiveMode::NonRecursive)

    let mut file = open_or_none(source_path)
    if let Some(f) = &mut file: f.seek(End)
    let mut buf = Vec::<u8>::new()

    loop:
        select:
            Some(evt) = notify_rx.recv() =>
                if !evt.paths.contains(source_path): continue
                match evt.kind:
                    Modify | Create =>
                        if file is None: file = try_open(source_path)
                        if let Some(f) = &mut file:
                            for line_bytes in read_new_lines_into(&mut buf, f):
                                let line = decode_line(line_bytes)   # lossy + truncate
                                emit StoreEvent(normalizer.normalize(&line, source.clone()))
                    Remove | Rename =>
                        file = None  # wait for Create
            _ = cancel.cancelled() =>
                break

    emit SourceLost(source_path)   # only on shutdown
```

#### Docker producer top-level

```text
spawn (cancel = parent token):
    let events = docker.events(filter: type=container, event=start|die|destroy)  # subscribe FIRST
    let initial = list_running_containers().await
    let mut tracked: HashMap<ContainerId, ChildToken> = HashMap::new()

    for c in initial:
        if !tracked.contains_key(&c.id):
            emit SourceFound(container_to_source(c))
            let child = cancel.child_token()
            spawn tail_container(c.id, source.clone(), tx.clone(), child.clone())
            tracked.insert(c.id, child)

    while let Some(evt) = events.next():   # already subscribed; no gap
        match evt.action:
            "start" if !tracked.contains_key(&evt.id) =>
                emit SourceFound + spawn tail_container + insert tracked
            "die" | "destroy" if let Some(child) = tracked.remove(&evt.id) =>
                child.cancel()             # tail stream closes naturally too
                emit SourceLost(evt.id)
        if cancel.is_cancelled(): break
```

`tail_container` reconnect: since docker logs streams close cleanly when the container dies, we *don't* retry — natural close means SourceLost is in flight. We log a warn if the stream errors before the container actually died.

#### Kubernetes producer top-level (per namespace)

```text
spawn (cancel = parent token):
    let pods: Api<Pod> = Api::namespaced(client, &namespace)
    let mut tracked: HashMap<(PodName, ContainerName), ChildToken> = HashMap::new()

    let mut watcher_stream = watcher(pods.clone(), watcher::Config::default())
    while let Some(event) = watcher_stream.next():
        match event:
            Applied(pod) | Modified(pod) =>
                for cs in pod.running_containers():    # both regular + init
                    let key = (pod.name, cs.name)
                    if !tracked.contains_key(&key):
                        emit SourceFound(pod_container_to_source(&pod, &cs, &namespace))
                        let child = cancel.child_token()
                        spawn tail_pod_container(pods.clone(), key.clone(), source.clone(), tx.clone(), child.clone())
                        tracked.insert(key, child)
            Deleted(pod) =>
                for (key, child) in tracked.drain_filter(|k| k.0 == pod.name):
                    child.cancel()
                    emit SourceLost(format!("{}/{}/{}", namespace, key.0, key.1))
            Restarted(observed) =>
                let (additions, removals) = reconcile_restarted(observed, &mut tracked)
                for (key, child) in removals: child.cancel(); emit SourceLost(...)
                for (pod, cs) in additions: emit SourceFound(...) + spawn tail + insert tracked
        if cancel.is_cancelled(): break

tail_pod_container(api, (pod_name, cname), source, tx, cancel):
    let mut backoff = Duration::from_millis(100)
    loop:
        if cancel.is_cancelled(): break
        match api.log_stream(&pod_name, &LogParams { container: Some(cname), follow: true, tail_lines: Some(0), .. }).await:
            Ok(stream) =>
                let lines = BufReader::new(stream).lines()
                let mut got_one = false
                while let Some(Ok(line)) = lines.next().await:
                    got_one = true
                    let decoded = decode_line(line.as_bytes())
                    emit StoreEvent(normalizer.normalize(&decoded, source.clone()))
                    if cancel.is_cancelled(): break
                if got_one: backoff = Duration::from_millis(100)   # reset on success
                tracing::warn!("kube tail stream closed for {}/{}, retrying in {:?}", pod_name, cname, backoff)
            Err(e) =>
                tracing::warn!("kube log_stream open failed for {}/{}: {} — retry in {:?}", pod_name, cname, e, backoff)
        select:
            _ = sleep(backoff) => {}
            _ = cancel.cancelled() => break
        backoff = (backoff * 10).min(Duration::from_secs(10))
```

## Deliverables

### Deliverable 1. CLI `--producer` flag and producer-spec wiring

Replace `Cli::demo: bool` with `Cli::producer: Vec<String>`. Parse each value into a `ProducerSpec` enum. Wire `App::new` to take the parsed specs and build the corresponding `Box<dyn LogProducer>` instances. Preserve the demo experience by routing `--producer demo` to a `FakeProducer` constructed with a unique synthetic source per occurrence. This deliverable also handles a few cross-cutting hygiene items so later deliverables don't have to revisit shared files (`error.rs`, `Cargo.toml`, `App::new` call sites). End-to-end demonstrable on the demo path alone — `fml --producer demo --producer demo` should show two demo sources in the selector.

- [ ] Add `ProducerSpec` enum (`Demo`, `File(PathBuf)`, `Docker`, `Kubernetes(Option<String>)`) in `fml/src/producer.rs`.
- [ ] Implement `ProducerSpec::parse(&str) -> Result<Self, ProducerError>` using `splitn(2, ':')` so paths containing `:` are preserved. Explicit error messages for unknown kinds and missing/extra args.
- [ ] Add unit tests for the parser covering: each valid kind, kind with vs without arg, unknown kind, `file` without path, `docker:foo` (extra arg rejected), `demo:foo` (extra arg rejected), `kubernetes:` empty arg (rejected), and **`file:/var/log/2026-05-02:12:00.log` (path with embedded colons — full string after first `:` is the path)**.
- [ ] Replace `Cli::demo: bool` with `#[arg(long = "producer")] pub producer: Vec<String>` in `main.rs`.
- [ ] Replace `App::new(config, demo: bool)` with `App::new(config, specs: Vec<ProducerSpec>)`.
- [ ] **Audit and update all `App::new` call sites and `Cli::demo` references** across `fml/src/**` and `fml/tests/**` (including any `with_test_backend` paths, integration test helpers, and example code). Verify `cargo build --workspace` and `cargo test --workspace` compile after the signature change.
- [ ] **Audit `Config` struct** (`fml/src/config.rs` and submodules) for any existing producer/demo-related fields that this work would orphan; if any exist, list them and either migrate or document leaving them in place.
- [ ] **Audit `ProducerError`** (`fml/src/error.rs`) against the planned variants for all deliverables: `Cli(String)`, `Io(std::io::Error)`, `Notify(notify::Error)`, plus the existing `Kubernetes(String)` and `Docker(...)`. Add the variants upfront with `#[from]` impls so later deliverables don't have to revisit.
- [ ] In `App::new`, enumerate demo specs (`demo-1`, `demo-2`, …) and construct `FakeProducer` per occurrence. Stub-build (or skip with logged warning) other kinds for now — this deliverable exits at the demo wire-through.
- [ ] Producer construction failures (e.g. docker daemon unreachable, kubeconfig missing) are logged at warn-level and skipped in `App::new`'s build loop, so a partial set of producers can still run.
- [ ] Delete `fn demo_sources()` from `app.rs`.
- [ ] Manual smoke: `cargo run -- --producer demo --producer demo` shows two distinct demo sources in the selector tree.

### Deliverable 2. Normalizer fixes (json + pattern)

Make `Normalizer::normalize()` non-panicking so real producers can pipe lines through it. Defer logfmt entirely. JSON extracts `msg` (or `message`) as the entry message, falling through to the raw line if neither is present. Pattern keeps the raw line as `msg` since unstructured logs *are* their message. The change is constrained to the `Some`-construction branches of both files; the `None`-return paths (parse failure / no level-or-timestamp detected) are preserved verbatim.

- [ ] **Read `pattern.rs` and `json.rs` first** and confirm the `todo!()` in each sits inside a `Some(...)` construction reachable only after successful detection / parse. The `None`-return paths (`if level.is_none() && ts.is_none() { return None; }` in pattern.rs; `serde_json::from_str(raw).ok()?` in json.rs) must be preserved unchanged.
- [ ] In `fml/src/producer/normalizer/json.rs`, replace the `msg: todo!(),` line with `obj.get("msg").or_else(|| obj.get("message")).and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| raw.to_string())`.
- [ ] In `fml/src/producer/normalizer/pattern.rs`, replace the `msg: todo!(),` line with `raw.to_string()`.
- [ ] Add unit tests in `json.rs` covering: line with `msg`, line with `message`, line with both (msg wins), line with neither (falls back to raw), malformed JSON (parser short-circuits via `?` — returns None, no construction).
- [ ] Add unit tests in `pattern.rs` covering: line with detected level (msg = full raw line), line with detected timestamp (msg = full raw line), line with neither level nor timestamp (returns None — current behaviour preserved).
- [ ] Verify `cargo test --workspace` passes.

### Deliverable 3. File producer

Implement `FileProducer` for tailing a single file from EOF, surviving rotation via `notify`. One `FileProducer` instance per `--producer file:<path>`. Source id = canonicalized absolute path when the file exists, or the path absolutized against the current working directory when waiting for a not-yet-created file. Source `producer = "file"`, `display_name` = file name (last path component), `group` = parent directory name (gives a useful selector grouping when tailing multiple files in the same dir). This deliverable also establishes the **cancellation pattern** and the **per-line decoding policy** that Deliverables 4 and 5 reuse verbatim.

- [ ] Confirm `notify` is in `fml/Cargo.toml` with the required features for `RecommendedWatcher`; add if missing.
- [ ] Confirm `tokio-util` is in `fml/Cargo.toml` with default features (for `tokio_util::sync::CancellationToken`); add if missing.
- [ ] **Establish the cancellation pattern**: `FileProducer` holds a `CancellationToken` field; `start` clones it into the spawned task and creates per-subtask child tokens via `token.child_token()` if any inner tasks are spawned; `stop` cancels the parent. Document this pattern inline in `producer.rs` so Deliverables 4 and 5 reuse it identically.
- [ ] **Implement the shared line-decoding helper** (or inline equivalent): takes `&[u8]`, returns `String` via `String::from_utf8_lossy`; truncates to 64 KiB with `... [truncated]` marker if longer. Unit-test with non-UTF-8 bytes and a 100 KiB single line.
- [ ] Add `FileProducer` struct in `fml/src/producer/file.rs` with fields for the path, normalizer, and cancellation token.
- [ ] Resolve file source identity with `canonicalize(path)` when possible; if the file does not exist yet, absolutize the path against the current working directory so relative paths do not change source id after creation.
- [ ] Implement `LogProducer for FileProducer` — `start` spawns a task that opens-and-seeks-to-end (or waits for the file to appear), watches the parent directory via `notify::RecommendedWatcher`, and on append events reads new bytes line-by-line through the decoding helper and the normalizer.
- [ ] **Bridge `notify::RecommendedWatcher`'s callback** into a `tokio::sync::mpsc::channel(64)` so the spawned task can `select!` on it alongside the cancellation token. Use `try_send` in the callback and log a warning on full-channel back-pressure.
- [ ] Handle rotation: on `Remove`/`Rename` events for the watched path, drop the file handle and wait for the next `Create`/`Modify` event before reopening.
- [ ] Wire `ProducerSpec::File(path)` into `App::new` to construct `FileProducer`.
- [ ] Unit-test the pure helpers (path → Source mapping; line-buffer flush behavior given partial-line input across reads; decoding helper with non-UTF-8 + over-length input).
- [ ] **Unit-test that `LogProducer::start` returns within 50ms** even when the target path doesn't exist (the producer should sit and wait for the file via the watcher, not block on file open).
- [ ] **Integration test (gated by `integration` feature)**: tempfile + tokio task that appends N lines with sleeps + `FileProducer` in a separate task + drain `ProducerEvent`s from the receiver and assert SourceFound + N StoreEvents arrive in order.
- [ ] **Integration test (gated by `integration` feature)**: rotation — write 5 lines, rename file, create new file at same path, write 5 more lines, assert all 10 are observed.
- [ ] Manual smoke: `cargo run -- --producer file:/tmp/foo.log` + `echo hi >> /tmp/foo.log` in another terminal shows the line.

### Deliverable 4. Docker producer

Replace the scaffolded `DockerProducer` with a working implementation: subscribe to `events()` *before* listing running containers (race-free), reconcile, then maintain the live set as containers come and go. Spawn a per-container log-tail subtask that streams `Docker::logs(follow=true, tail=0)` and emits StoreEvents through the normalizer. Source id = container id, display_name = container name (no leading `/`) or short id fallback, group = compose project label (`com.docker.compose.project`) if present else image name. Producer field = `"docker"`. Reuses the cancellation pattern and line-decoding helper from Deliverable 3.

- [ ] Confirm `bollard = "0.20"` (or current pin) is in `fml/Cargo.toml` with the `chrono` feature; verify `LogOutput`, `Docker::events`, and `Docker::logs` API shapes match the gotchas before writing implementation.
- [ ] Implement `LogProducer for DockerProducer` — `start` spawns a task that subscribes to `events()` first, then lists containers, then dedups and emits `SourceFound` + spawns `tail_container` subtasks.
- [ ] Implement `tail_container(docker, container_id, source, tx, cancel)` that streams `Docker::logs(follow=true, stdout=true, stderr=true, tail="0")`, line-buffers across `LogOutput::StdOut`/`StdErr` chunks, decodes via the shared line helper, and emits `StoreEvent` per line through `Normalizer::normalize`.
- [ ] Wire the `events()` subscription to drive ongoing churn: on `start` emit `SourceFound` + spawn tail subtask + track child token, on `die` cancel the child token + emit `SourceLost` + drop from tracked.
- [ ] Implement `container_summary_to_source(c: &ContainerSummary) -> Source` mapping with: id = long container id, display_name = first name without `/` (fallback to short id when names is empty), group = compose project label or image name, producer = `"docker"`.
- [ ] Fix the existing `list_running_containers()` filter contradiction (`all=true` + `status=running`) — drop the `.all(true)`.
- [ ] **Handle "docker daemon unreachable"**: `DockerProducer::new` returns `Err(ProducerError::Docker(...))` when `Docker::connect_with_local_defaults()` fails; `App::new` logs warn-level and skips the producer (other producers continue). Add a unit test that constructs against a bogus socket path and asserts the error.
- [ ] Wire `ProducerSpec::Docker` into `App::new` to construct `DockerProducer`.
- [ ] Unit-test `container_summary_to_source` with: container with compose label (group = project), container without compose label (group = image), container with multiple names (display_name = first), container with `/` prefix on name (stripped), **container with no names — display_name falls back to short container id (12 hex chars)**.
- [ ] **Unit-test that `LogProducer::start` returns within 50ms** even when `Docker::connect` fails (use a bogus socket path or a stub that errors on connect).
- [ ] **Integration test (gated by `integration` feature, uses `testcontainers`)**: spin up a `busybox` container running `sh -c 'while true; do echo line-$i; i=$((i+1)); sleep 0.1; done'`, connect `DockerProducer` to the local daemon, drain `ProducerEvent`s, assert `SourceFound` for the test container's ID arrives, assert at least 3 `StoreEvent`s with msg matching `^line-\d+$`, stop the container, assert `SourceLost` arrives.
- [ ] Add `testcontainers = "..."` to dev-dependencies (resolve concrete version when adding).
- [ ] Add a preamble doc-comment on the integration test module: "requires a running docker daemon; tests fail loudly when none is available — use `cargo test --features integration` only on machines with docker."
- [ ] Manual smoke: `docker run --rm --name fml-smoke busybox sh -c 'while true; do echo hi; sleep 1; done'` in one terminal + `cargo run -- --producer docker` in another. Verify the new source appears, lines stream, `docker stop fml-smoke` triggers `SourceLost`.

### Deliverable 5. Kubernetes producer

Replace the `todo!()` `LogProducer` impl on `KubernetesProducer` with a watcher-driven, per-pod-container tail with reconnect-on-close. One `KubernetesProducer` per `--producer kubernetes[:ns]`. Bare form resolves the active kubeconfig context's namespace (literal `"default"` fallback when unset). Source id = `{namespace}/{pod_name}/{container_name}`, display_name = container_name, group = pod_name, producer = namespace string. Running init containers and sidecars each get their own source; already-terminated init containers are out of scope for this live-tail pass. Reuses the cancellation pattern and line-decoding helper from Deliverable 3. Per-tail retry-with-backoff prevents a transient drop from permanently silencing a healthy pod, but it does not guarantee replay of lines emitted during the disconnected window.

- [ ] Confirm `kube = "3.1"` (or current pin), `kube-runtime` (if separate), and `k8s-openapi` versions and features in `fml/Cargo.toml`; verify `kube::runtime::watcher::Event` variants (`Applied`/`Modified`/`Deleted`/`Restarted`) match the pseudo-code before implementation. If the API has evolved, update the pseudo-code and gotchas first.
- [ ] Add `KubernetesProducer::resolve_namespace() -> Result<String, ProducerError>` that reads `Kubeconfig::read()`, finds the active context, returns `context.namespace.unwrap_or_else(|| "default".to_string())`. Used when `--producer kubernetes` is bare.
- [ ] Document in the kube producer's module doc that **in-cluster service-account configs, `--context` overrides, and multi-cluster federation are out of scope**; users hitting these will see a clear `Kubeconfig::read()` error.
- [ ] Implement `LogProducer for KubernetesProducer` — `start` spawns a task that runs `kube::runtime::watcher` over `Api<Pod>` in the configured namespace.
- [ ] On watcher `Applied`/`Modified`: for each container in `pod.status.container_statuses + initContainerStatuses` whose `state.running.is_some()`, if not already tracked, emit `SourceFound` + spawn `tail_pod_container` + insert into `tracked` with a child cancellation token. Completed init containers are intentionally skipped in this deliverable.
- [ ] On watcher `Deleted(pod)`: for each tracked entry whose pod_name matches, cancel the child token and emit `SourceLost`.
- [ ] **Implement `Restarted(pods)` reconciliation explicitly**: extract a pure helper `reconcile_restarted(observed: &[Pod], tracked: &mut HashMap<...>) -> (additions, removals)` that diffs the new pod set against the `tracked` map. For vanished `(pod_name, container_name)` pairs cancel their subtask token + emit `SourceLost`; for newly running containers emit `SourceFound` + spawn `tail_pod_container`. Unit-test the helper with: empty observed (everything removed), partial overlap, full overlap (no changes), all-new, mix of additions and removals.
- [ ] Implement `tail_pod_container(api, (pod_name, cname), source, tx, cancel)` using `Pod::log_stream(&pod_name, &LogParams { container: Some(cname), follow: true, tail_lines: Some(0), .. })`, wrap in `BufReader`, iterate lines, decode via the shared helper, emit `StoreEvent` per line through the normalizer.
- [ ] **Implement reconnect-on-close with exponential backoff**: when the log_stream closes or errors while `cancel` is unset and the container is still `tracked`, sleep `backoff` and reopen `log_stream`. Backoff schedule: 100ms → 1s → 10s, capped at 10s, reset to 100ms on the first successful read after a reconnect. Emit a `tracing::warn!` on each reconnect attempt, including that catch-up across the disconnected window is not guaranteed. Stop retrying when the cancellation token fires. Extract the backoff state machine as a small helper and unit-test it independently of the kube API.
- [ ] Implement `pod_container_to_source(pod, container_name, namespace) -> Source` mapping: `producer = namespace`, `id = format!("{namespace}/{pod_name}/{container_name}")`, `display_name = container_name`, `group = Some(pod_name)`.
- [ ] Wire `ProducerSpec::Kubernetes(ns)` into `App::new` — `ns.unwrap_or_else(KubernetesProducer::resolve_namespace)?` then construct.
- [ ] Unit-test `pod_container_to_source` (id format, group = pod_name, producer = namespace).
- [ ] Unit-test `resolve_namespace` against synthetic Kubeconfig fixtures: context with namespace, context without namespace (returns `"default"`), missing active context (error).
- [ ] **Unit-test that `LogProducer::start` returns within 50ms** even when the kube API is unreachable (synthetic client pointing at a closed port).
- [ ] No integration tests at this stage — explicitly deferred (see Issues).
- [ ] Precondition for manual smoke: confirm `kubectl get pods` succeeds against the active context. If the developer has no cluster, they need to set one up (kind/minikube/local) before running this step.
- [ ] Manual smoke: `cargo run -- --producer kubernetes` against the user's local cluster; verify pods in the active namespace surface, container churn (e.g. `kubectl rollout restart deployment/foo`) is reflected, and `kubectl delete pod <name>` triggers `SourceLost`.
- [ ] **Open a follow-up issue** for "Source enrichment with kubernetes workload kind (deployment/statefulset/job/cronjob/pod)" — referenced in the Issues section. File the follow-up so the deferral is durable; do not implement here.

## Issues

- **2026-05-02 — agent:codex (adversarial review)** — Plan reviewed with two local adversarial passes (Risks & Assumptions, Completeness & Scope). 5 findings; 5 merged into plan. Most significant changes: kube source IDs now include namespace to avoid global `Source.id` collisions, kube reconnect wording now admits disconnected-window loss instead of promising lossless catch-up, and completed init-container logs are explicitly out of scope for this live-tail pass.
- **2026-05-02 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 22 findings; ~20 merged into plan as new risks, gotchas, and tasks. Most significant change: per-tail retry with exponential backoff added to Deliverable 5 (chosen over "accept the gap") to address silent log dropouts on transient kube/network failures. Other notable additions: explicit `Restarted` reconciliation helper + tests, `App::new` call-site audit task, crate-version verification step at the head of each producer deliverable, line-decoding policy (lossy UTF-8 + 64 KiB truncate), notify callback bridging via `mpsc`, splitn for paths with `:`, "start returns within 50ms" tests for all real producers.
- **2026-05-02 — agent:claude (deferred from grill)** — `Source` likely needs a richer way to convey kubernetes workload kind (deployment / statefulset / job / cronjob / pod) and the workload name. The grill-time discussion considered injecting `kubernetes_type` and `kubernetes_type_assignment` into per-entry `fields` but settled on "this belongs on `Source` itself, design later". Out of scope for this BIGPLAN. Deliverable 5 has an explicit follow-up checkbox to file this as a tracked issue once the kube producer lands and the gaps in the source-selector UI are visible.
- **2026-05-02 — agent:claude (deferred from grill)** — CI strategy for the `integration` feature flag is undecided. The flag exists in `fml/Cargo.toml`; tests written under it run via `cargo test --features integration`. Whether/where they run on CI is a follow-up after we decide whether the project has a CI pipeline at all. The integration tests will fail loudly (not soft-skip) when docker is missing — that's a deliberate "loud failure" choice.
- **2026-05-02 — agent:claude** — Repeated identical `--producer` flags (e.g. `--producer docker --producer docker`) are tolerated by the parser and *will* spawn duplicate producer instances. The downstream `handle_producer_event` dedups `SourceFound` by id, so the source selector tree looks normal, but each duplicate's tail subtasks still emit duplicate `StoreEvent`s — meaning the user sees each line twice (or N times). This is the "simplest path" we agreed to in the grill. If it becomes a footgun in practice, dedup at parse time on the `ProducerSpec` `Eq` boundary.
- **2026-05-02 — agent:claude (review-noted)** — Graceful shutdown ordering across the new producers is unverified. When the user quits, each producer's `stop` cancels its parent token, but slow-cancellation streams (kube `log_stream`, bollard `logs`) can take seconds to actually return on the next poll. Open question: do we add an abort-after-timeout in `App::run`, or live with the occasional sluggish exit? Defer until we see it in practice; revisit after manual smoke on Deliverable 5.
- **2026-05-02 — agent:claude (review-noted)** — The `Source.producer` field has mixed semantic categories under this plan (`"docker"`/`"file"`/`"fake"` are kind names; for kubernetes it's the namespace string). A user with `--producer demo --producer docker --producer kubernetes:my-ns` sees top-level groups `fake`, `docker`, `my-ns` — visually inconsistent. The grill chose this deliberately for the namespace-scoped tree shape on the kube side. Tracked here in case a TUI snapshot ever surfaces the inconsistency badly enough to warrant a redesign (e.g. adding a separate "producer kind" axis to the source selector).
