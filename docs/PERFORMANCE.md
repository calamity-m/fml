# Performance Notes & Refinement Backlog

State of play after the 2026-06 fuzzy-search optimization round, plus a
backlog of refinements worth considering later. Numbers come from the
bench harness and live runs against a kubernetes firehose (6 pods,
~1,100 entries/s sustained, ~7 KB JSON entries with a 1 KB random
payload blob).

## How to measure

- **Scan throughput bench** (synthetic firehose-shaped entries):

  ```sh
  cargo test --release -p fml --lib bench_fuzzy_scan -- --ignored --nocapture
  ```

- **Live telemetry**: run with `--debug` and watch `/tmp/fml.*`:
  - `event loop heartbeat` — events/5s per channel plus channel backlogs.
    Any non-zero backlog means the reducer loop is falling behind.
  - `slow render` — logged whenever a frame exceeds 33 ms.

- Always measure release builds; debug is 10–50x slower in the matchers
  and will mislead you.

## What's already done (don't re-litigate)

| Change | Effect |
|---|---|
| Incremental rescan | One full pass per query, then O(new entries); no perpetual rescans under live ingest |
| Borrowed leaf-walk haystacks | No per-scan cloning/serialization of field values; needle matches values only, not JSON syntax/keys |
| Split emission + traceback-on-emit | Highlight tracebacks computed only for displayed hits; full uncapped seq list still emitted for `n`/`N`; per-tick payload capped at `fuzzy_result_limit` (default 1k, was 20k) |
| Field leaf cap (`fuzzy_max_field_bytes`, default 512) | Nucleo (default matcher): 36k → 383k entries/s (10.6x). Frizbee: ~+10% (its SIMD prefilter already rejected blobs) |

Current single-core scan rates on firehose-shaped entries: nucleo ~380k/s,
frizbee ~135k/s, matcher ceiling on bare messages ~3M/s.

## Backlog, roughly by value

### 1. Memory ceiling (the real limit)

Observed: 2.9 GB RSS at ~390k retained firehose entries (~7.4 KB each).
The default store capacity of 1M entries implies ~7 GB on such a feed.
Throughput is not the constraint — memory is.

High-level options, cheapest first:

- **Byte-budget eviction**: evict by total retained bytes instead of (or in
  addition to) entry count. Honest behavior on fat feeds, tiny change to
  `RingBufferStore`.
- **Field representation**: `HashMap<String, serde_json::Value>` per entry
  is pointer-heavy. Interning common keys (`trace_id`, `http`, …) or
  storing fields as a compact serialized blob parsed on demand trades CPU
  for a large per-entry saving.
- **Disk spill (sqlite or custom)**: only needed if retention must exceed
  RAM. Note: a DB is a *storage* decision, not a search-speed decision —
  fuzzy matching still happens in app code; FTS would only serve as a
  candidate pre-filter (see §4, which gets you the same shape in memory).

### 2. Frizbee batch-path overhead

After the leaf cap, capped **nucleo outperforms frizbee** (383k vs 135k
entries/s) despite frizbee's SIMD core being far faster on raw haystacks
(~3M/s msg-only). The gap is per-batch plumbing: assembling four haystack
vectors and making four `match_list` calls per 256-entry chunk. If frizbee
matters (it's opt-in config), flatten to a single haystack list with an
owner map, or use its `Matcher`/`match_list_into` APIs to reuse buffers.
Alternatively: accept nucleo as the better default and deprioritize.

### 3. Parallel scanning

The scan is a single task. `frizbee::match_list_parallel` exists, or chunk
the snapshot across `spawn_blocking` workers for either matcher — a near
core-count multiplier when a fresh query hits a large buffer. Only worth
it if first-pass latency on multi-million-entry stores becomes noticeable
(at ~380k/s, a full 1M-entry pass is ~3 s with partials streaming).

### 4. Candidate pre-elimination index

For very large buffers, stop scanning non-candidates at all:

- **Char bitmask** (8 bytes/entry, computed at insert): a one-AND rejection
  test — "does this entry contain all the needle's characters anywhere".
  Cheap, helps selective needles, useless for short/common ones.
- **Trigram index** (tens of bytes/entry, maintained on insert/evict):
  intersect the needle's trigrams to get a candidate seq set, fuzzy-score
  only those. This is exactly what sqlite FTS would provide, minus the
  disk; the candidates→rerank pipeline carries over unchanged if storage
  later moves to a DB. Typo tolerance weakens pruning — require fewer
  trigram hits when `max_typos` is set.

Hold until §1/§3 are exhausted; the incremental scan already makes this a
first-pass-only cost.

### 5. Live-search dispatch debounce

`/` dispatches a fresh fuzzy query on every keystroke. Latest-wins
cancellation keeps it correct, but each keystroke restarts the full first
pass. `fuzzy_debounce_ms` exists in config but is not wired into the
modal TUI's live search. Wiring it in (or only dispatching after a short
quiet period) avoids wasted scans while typing fast — noticeable on large
buffers, irrelevant on small ones.

### 6. Emission payload at extreme hit counts

Each tick emits the full hit-seq list (`Vec<u64>`). At 500k+ hits that's
~4 MB per tick of copy — fine today, measurable someday. Options: emit
the seq list only when it changed, or emit deltas. Also: a needle that
matches half the store is a bad query; a "too many matches" hint in the
status line may be the better fix.

### 7. Render scaling

Currently zero slow renders (>33 ms) at 60 fps with multiple panes. Costs
that grow with pane count and height: per-char style buffers per visible
row, `store.stats()` lock per frame, and per-frame `row_text` rebuilds.
If profiling ever shows render hotspots: cache `row_text` per (seq, width)
for visible rows, or drop the frame rate. Don't optimize speculatively —
watch the `slow render` log line.

## Known non-problems

- **Event loop**: zero channel backlogs at ~1,100 entries/s ingest with
  live searches running; the reducer loop is nowhere near saturation.
- **Tail/history workers**: re-emit bounded windows on bounds change;
  cost is proportional to window size, not store size.
- **Docker batching**: multi-second pauses then thousands of entries at
  once is the Docker daemon (worse on WSL2/Docker Desktop), not fml.
