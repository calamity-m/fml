# BIGPLAN: Startup Log Backfill

## Plan Overview

This effort changes ingest from live-only tailing to bounded startup history loading for real producers. When a `file`, `docker`, or `kubernetes` producer first tracks a live source during this process, it should emit bounded existing logs for that source, then continue following live logs with the current source lifecycle and filtering behavior. The user-facing policy is a 30-minute default provider backfill window where the backend supports time filtering, plus a hard safety cap of 5,000 lines per source; setting the configured line cap to `0` disables optional backfill. Done means running sources show recent pre-follow logs in the retained store before or alongside newly arriving live logs, without adding demand-loaded history, global timestamp sorting, or Docker stopped-container browsing.

## Risks

- **Backfill floods the event loop** — A busy source can produce thousands of historical entries immediately, and the current producer event path inserts entries one at a time. The mitigation is to enforce the per-source hard cap before emission, keep `start` non-blocking by doing work in spawned tasks, and verify bounded multi-source bursts against concrete event-loop and render responsiveness thresholds.
- **Aggregate source count multiplies the cap** — The 5,000-line cap is per source, so hundreds of sources still create a large startup burst. The mitigation is to add provider-side backfill concurrency limits or an explicit measured acceptance threshold before broadening beyond the current per-source cap.
- **Backfill-to-live handoff can lose lines** — Separate history and follow operations create a gap where file appends or provider log writes can occur between requests. The mitigation is to use a stable handoff point per provider: files follow from the captured EOF used for backfill; Docker prefers a single follow stream with bounded history options; Kubernetes documents or narrows the handoff gap with overlap/deduplication tests.
- **Provider semantics differ** — Docker and Kubernetes expose server-side time and tail options, while plain files do not have trustworthy per-line timestamps. The mitigation is to treat `since` as authoritative for provider APIs and best-effort for files: file backfill reads from the end and applies the line cap because timestamp filtering cannot be done safely.
- **Backfill fetch failures can hide missing history** — A failed history request should not prevent live following, but silent failure makes the retained store misleading. The mitigation is to log current-log backfill failures, continue live follow where possible, and classify Kubernetes `previous=true` “no previous logs” responses narrowly rather than swallowing unrelated API errors.
- **Kubernetes previous logs have a different lifecycle** — `previous=true` logs belong to a terminated container instance, not the currently running stream. The mitigation is to emit previous logs before current startup logs for the same pod/container source, cap them like other backfill, and avoid promising ongoing previous-log discovery after startup.
- **Sequence order is not timestamp order** — The store assigns monotonic sequence IDs in arrival order, so concurrent source backfills will interleave by producer scheduling rather than by log timestamp. This is accepted scope: preserve per-source order, keep cross-source order as arrival order, and document that no global sort pipeline is part of this change.

## Plan Details

### Backfill policy

Add an ingest history config that is shared by real producers:

```toml
[ingest]
backfill_window_secs = 1800
backfill_max_lines_per_source = 5000
```

`backfill_window_secs` is the provider-side time window for backends with time filtering. `backfill_max_lines_per_source` is a hard safety cap; `0` disables startup backfill for all producers but still follows live logs. Demo producer remains live-only. Backfill is applied when a real producer first tracks a live source during this process; it is not a reconnect catch-up mechanism and does not browse stopped Docker containers.

### Source and ordering model

Backfill does not introduce new store semantics. Producers still emit `SourceFound` before any entries for that source, then `StoreEvent` for backfilled entries in source-local chronological order, then live entries from the follow stream. For Kubernetes, source-local chronological order means previous-container logs before current-container startup logs before current live logs. Cross-source ordering remains whatever order events reach the app's producer channel.

### Provider behavior

- `file` — On startup, announce the source, capture the file EOF, read up to the last `backfill_max_lines_per_source` complete lines before that EOF, emit them oldest-to-newest, then continue following appends from the captured EOF. When the file is missing at startup, preserve current behavior and start from the beginning when it is later created. The 30-minute window is not applied to raw files.
- `docker` — For each running container, announce the source, request logs with `since` set to the configured window and a tail cap matching `backfill_max_lines_per_source`, emit returned lines in stream order, then follow live logs. Disabled backfill must preserve live-only behavior with `tail("0")`. Stopped containers remain out of scope.
- `kubernetes` — For each running pod container, announce the source, request bounded `previous=true` startup logs for the same pod/container and treat the specific “no previous logs” response as normal, then request bounded current startup logs using `since_seconds` plus `tail_lines`, then follow live logs with the existing reconnect loop. Disabled backfill must preserve live-only behavior with `tail_lines: Some(0)`. Previous logs are startup history only; reconnect catch-up is not expanded in this plan.

### Critical Files

- `fml/src/config.rs` — Adds and loads the top-level ingest/backfill config.
- `fml/src/config/ingest.rs` — Defines the ingest backfill config and defaults.
- `fml/src/main.rs` — Adds CLI overrides only if the implementation chooses to expose them; config is the required control surface.
- `fml/src/app.rs` — Passes resolved ingest backfill settings into producer constructors.
- `fml/src/producer.rs` — Holds the producer trait and event reducer; useful for documenting the backfill ordering contract without changing store semantics.
- `fml/src/producer/file.rs` — Implements startup file backfill and keeps follow/rotation behavior intact.
- `fml/src/producer/docker.rs` — Changes Docker log options from `tail("0")` live-only to bounded startup history plus live follow.
- `fml/src/producer/kubernetes.rs` — Changes Kubernetes `LogParams` from `tail_lines: Some(0)` live-only to bounded startup history plus live follow, and adds startup previous-log retrieval.
- `fml/src/store.rs` — Confirms that retained capacity and sequence IDs remain arrival-order based; no store API change is planned.
- `README.md` — Documents default backfill behavior, config keys, provider caveats, and disable semantics.

### Gotchas

- The current Docker and Kubernetes tails use `tail("0")` / `tail_lines: Some(0)` explicitly to skip existing logs; those lines are the live-only switch.
- `LogProducer::start` must still return promptly; backfill work belongs inside the spawned producer tasks.
- Source blocking must happen before backfill work starts so blocked sources emit neither `SourceFound` nor historical entries.
- Backfill failure for current file/Docker/Kubernetes logs should be logged and non-fatal; live follow should still start where the provider allows it.
- File backfill must preserve partial-line semantics: only complete lines should be emitted, matching the existing `LineBuffer` behavior.
- File backfill must avoid the tail-then-open-at-EOF race by capturing a follow offset before reading historical lines.
- Kubernetes previous-log error handling must distinguish the normal “no previous container logs” response from RBAC, network, and API failures.
- The ring buffer can evict older backfilled entries while startup is still in progress when many sources each hit the cap; that is acceptable and should be described as retention behavior, not an ingest failure.

### Pseudo-code / Sketches

```text
for each discovered real source:
  if source is blocked:
    skip all work for that source
  send SourceFound(source)
  if backfill_max_lines_per_source > 0:
    history = provider.fetch_history(
      since = now - backfill_window,
      max_lines = backfill_max_lines_per_source,
    )
    for line in history oldest_to_newest:
      send StoreEvent(normalize(line, source))
  start current live follow loop for that source
  on history failure:
    log the failure and start live follow when possible
```

```text
file startup:
  source = source_for_path(path)
  send SourceFound(source)
  if file exists:
    eof = current_file_len(path)
    emit tail_complete_lines_before(path, eof, max_lines) oldest_to_newest
    reader = FileReader::open_at(path, eof)
  else:
    reader = None
  continue existing notify-driven follow/reopen loop
```

## Deliverables

### Deliverable 1. Backfill configuration and wiring

Add a small ingest config surface that defines the shared startup-history policy and threads it to real producers. The required controls are `backfill_window_secs` defaulting to `1800` and `backfill_max_lines_per_source` defaulting to `5000`; `0` for the line cap disables startup backfill. CLI flags are optional implementation detail only if they stay small and mirror config exactly.

- [x] Add `fml/src/config/ingest.rs` with documented `IngestConfig` defaults.
- [x] Add `ingest: IngestConfig` to `Config` and config serialization/deserialization tests covering TOML and `FML__INGEST__*` environment overrides.
- [x] Define a small copyable runtime settings type for producers if using `Config` directly would couple producers too broadly.
- [x] Thread the settings through `App::new` into `FileProducer`, `DockerProducer`, and `KubernetesProducer` constructors.
- [x] Update README config examples and producer docs with the 30-minute / 5,000-line defaults and `0` disable behavior.

### Deliverable 2. File producer startup backfill

Change `FileProducer` so an existing file contributes recent complete lines at startup before normal append-following begins. Because raw files do not have reliable timestamps before normalization, this deliverable enforces the hard line cap and documents that the time window is provider-side only for file producers in this plan.

- [x] Add a file-tail helper that reads up to the last N complete lines before a captured EOF without loading unbounded files into memory.
- [x] Emit startup file backfill oldest-to-newest after `SourceFound` and follow from the captured EOF rather than reopening at the later EOF.
- [x] Preserve current missing-file, create, delete, rename, and truncate behavior after startup.
- [x] Add unit tests for bounded startup backfill, `0` disable, complete-line-only handling, large-file bounded reads, and append race avoidance across the backfill/follow handoff.
- [x] Add an integration-style producer test that verifies startup backfill entries precede appended live entries for one file source.

### Deliverable 3. Docker running-container backfill

Change `DockerProducer` so each running container emits bounded existing stdout/stderr history before its live follow stream. The scope remains currently running containers only; stopped containers and demand-loaded Docker history are out of scope.

- [x] Add Docker log option construction that combines `follow(true)`, stdout/stderr, configured `since`, and the line cap as `tail`.
- [x] Keep disabled backfill explicitly live-only by preserving Docker `tail("0")` behavior.
- [x] Verify Docker `since + tail + follow` behavior with the bollard/Docker API version used by the project and record the result in tests or comments.
- [x] Ensure `SourceFound` and source-blocking behavior remain at the single `track_container` gate before any backfill can emit.
- [x] Preserve source-local order as delivered by Docker and keep cross-source ordering as channel arrival order.
- [x] Add tests around option construction and disabled-backfill behavior using existing Docker test patterns.
- [x] Update Docker README notes to distinguish daemon batching from intentional startup backfill.

### Deliverable 4. Kubernetes running and previous-log backfill

Change `KubernetesProducer` so each running pod container emits bounded startup history from the current container log and also attempts bounded `previous=true` startup history for that same pod/container. This is startup-only history; the existing reconnect loop still has its current catch-up limitation during later disconnected windows.

- [x] Add Kubernetes `LogParams` construction for startup history using `since_seconds` and `tail_lines` with `follow: false`.
- [x] Keep disabled backfill explicitly live-only by preserving Kubernetes `tail_lines: Some(0)` behavior for the follow stream.
- [x] Verify Kubernetes `since_seconds + tail_lines` behavior with the kube client/server API used by the project and record the result in tests or comments.
- [x] Emit previous startup history oldest-to-newest before current startup history, then start the existing live `follow: true` loop.
- [x] Attempt `previous=true` startup history with the same bounds, treating only the specific “no previous container logs” response as normal and non-fatal.
- [x] Preserve existing source blocking, pod watch reconciliation, and source lost behavior.
- [x] Add tests for params construction, disabled backfill, previous/current/live ordering, current-history failure fallback to live follow, and non-fatal missing previous logs using existing Kubernetes test patterns.
- [x] Document that Kubernetes previous logs are attached to the same pod/container source during startup and are not rediscovered after startup.

### Deliverable 5. Startup responsiveness and verification

Verify that capped backfill does not make the TUI or event loop unusable at startup. This deliverable focuses on observable behavior and regression coverage, not on introducing a new batching store API.

- [x] Add a stress-oriented test or benchmark-style test that emits capped multi-source backfill and confirms store bounds, producer channel backlog, and event processing complete within a documented threshold.
- [x] Verify `LogProducer::start` still returns promptly for file, Docker, and Kubernetes constructors/start paths.
- [x] Run the smallest relevant producer tests first, then `cargo test --workspace` after shared config and producer changes land.
- [x] Manually smoke-test `file`, `docker`, and `kubernetes` producers with default backfill, disabled backfill, and at least one source-blocking config.
- [x] Confirm README behavior matches observed startup ordering and retention behavior.

## Issues

- **2026-06-12 — agent:claude (re-track duplication fix)** — Post-review finding: tracking is keyed per source, and crash loops / container restarts remove and re-add the same key, so every re-track re-ran the full backfill — Kubernetes `previous=true` re-fetched the instance fml had already tailed live, and Docker's `since` window reached back into the pre-restart run. Fixed: Kubernetes keeps a `previously_tracked` key set and skips the previous-instance fetch on re-track (current-instance startup history still runs — it is genuinely new); Docker records a `last_seen` timestamp on die/destroy and clamps `since` forward to it on re-track (≤1s boundary overlap accepted, duplicates preferred over loss). Verified with mock-service tests asserting the actual request URIs (previous→current ordering, bounds in query, no `previous=true` on re-track) and a pure option-construction test for the Docker clamp.
- **2026-06-12 — agent:claude (implementation)** — All five deliverables implemented and verified. `IngestConfig` (`[ingest]`, `Copy`) threads through `App::new` into all three real producers. File backfill scans backwards in 64KiB chunks with a `(cap+1) × 64KiB` scan budget and hands off to the follow reader at the offset after the last complete line, so a trailing partial line is emitted whole once completed. Docker uses a single `follow + since + tail` stream (no handoff gap); Kubernetes fetches `previous=true` then current history (`since_seconds + tail_lines`, `follow: false`) before the unchanged `tail_lines: Some(0)` live loop, with the kubelet's "previous terminated container … not found" 400 classified narrowly as normal. Verification: 280 lib tests plus integration suites pass; clippy warning count unchanged (14 pre-existing); stress test (20 sources × 5,000 lines through the real event loop) completed in 0.22s against a documented 10s budget. Live smoke tests: file default/disabled backfill in the TUI; Docker backfill matched `docker logs --since 30m --tail 5000` exactly (686 lines, real daemon); Kubernetes against minikube `kube-system` emitted `storage-provisioner`'s 2 previous-instance lines before current startup logs (matched `kubectl logs -p`); a `blocked = "storage-provisioner"` profile produced zero entries for that source while others backfilled.
- **2026-06-10 — agent:pi (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 12 findings; 11 merged into plan. The most significant changes clarified backfill-to-live handoff gaps, Kubernetes previous/current ordering, disabled-provider behavior, and aggregate burst risk.
