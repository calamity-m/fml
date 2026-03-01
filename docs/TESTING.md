# Testing

## Running the suite

```sh
# Full suite (all crates, all harnesses)
cargo test --all

# Single integration harness
cargo test --test search_harness

# With output (useful for seeing pending stub tests)
cargo test --all -- --nocapture

# Only non-ignored tests
cargo test --all -- --skip ignored

# Include stub tests (see todo! panics — useful for planning)
cargo test --all -- --include-ignored
```

## Harness policy

Files in `tests/` are **integration harnesses** — they start as stubs and are filled in as each module is built.

- **Unit tests** (single function or struct in isolation) live in the module's own `#[cfg(test)]` block.
- **Integration tests** (cross-module or externally observable behaviour) stay in the harness.
- By project completion, no harness file should remain except `tests/common/`. All unit tests migrate to their modules.

`tests/common/` holds shared test utilities. Promote it to a `fml-test-utils` workspace crate if it exceeds ~300 lines or is needed by more than two harnesses.

## What each harness covers

| Layer | Harness | Key invariants |
|-------|---------|----------------|
| Ingestor (Kubernetes) | `kubernetes_harness` | Producer tagging, reconnect, no duplicate lines on retry |
| Ingestor (Docker) | `docker_harness` | Frame decoding, compose naming, stderr tagging |
| Ingestor (File) | `file_harness` | Rotation, truncation, glob, all written lines received |
| Ingestor (Stdin) | `stdin_harness` | EOF behaviour, burst, headless exit |
| Normalizer | `normalization_harness` | Synthetic fields always present, JSON/logfmt/unstructured parsing, snapshots |
| Store | `store_harness` | Ring eviction, monotonic sequence numbers, concurrent safety |
| Search | `search_harness` | **Greed monotonicity** (most critical), all 7 domain families, negative prefix inference, results ⊆ store |
| Export | `export_harness` | All 3 formats × 4 scopes, insta snapshots |
| Headless | `headless_harness` | Process-level flag validation, exit codes, TTY detection |

## What the tests do NOT guarantee

- Correct rendering in a real terminal (the UI layer has no automated render tests)
- Behaviour with real Kubernetes clusters or Docker daemons
- Performance under production load (use the benchmarks for that)

## Snapshot tests

Snapshot files live in `tests/snapshots/`. Update after intentional format changes:

```sh
cargo test --test normalization_harness
cargo test --test export_harness
cargo insta review    # approve or reject each diff interactively
```

Never silently accept snapshot diffs — always review them with `cargo insta review`.

## Benchmarks

```sh
# Run all benchmarks
cargo bench

# Single bench group
cargo bench --bench search_bench

# HTML report (requires gnuplot)
open target/criterion/report/index.html

# Save a baseline
scripts/save_baselines.sh main

# Compare branches (requires: cargo install critcmp)
critcmp main candidate --threshold 5
```

### Flamegraphs

Flamegraphs are generated via `pprof` with the `criterion` feature. No `perf` or root access required.

**Setup** (`[dev-dependencies]` in `Cargo.toml`):

```toml
pprof = { version = "0.14", features = ["flamegraph", "criterion"] }
```

**Wire into a bench file**:

```rust
use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = search_benches;
    config = Criterion::default()
        .with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = exact_bench, greedy_expansion_bench,
}
```

**Generate and view**:

```sh
cargo bench --bench search_bench -- --profile-time 10 "greedy/full_pipeline"
open target/criterion/greedy/full_pipeline/.../profile/flamegraph.svg
```

Flamegraph SVGs are gitignored. Always generate fresh on the branch under investigation.

### Key benchmark groups

| Bench file | Groups |
|------------|--------|
| `pipeline_bench` | `json_ingest`, `logfmt_ingest`, `search_filter`, `export_raw` |
| `search_bench` | `exact`, `greedy_expansion`, `full_pipeline` |
| `store_bench` | `push`, `eviction`, `concurrent_read` |
| `tui_bench` | `full_frame_render`, `log_stream_500`, `producer_tree_200` |

### TUI render targets

| Widget | Target |
|--------|--------|
| Full frame (all panes, 50-row terminal) | < 2 ms |
| Log stream with 500 visible lines | < 1 ms |
| Producer tree with 200 nodes | < 0.5 ms |
| **Full frame at 120 fps** | **< 8 ms** |

TUI benchmarks use `ratatui::backend::TestBackend` (renders to an in-memory buffer, no real terminal needed):

```rust
let backend = TestBackend::new(200, 50);
let mut terminal = Terminal::new(backend)?;
c.bench_function("log_stream_500_lines", |b| {
    b.iter(|| terminal.draw(|f| render_log_stream(f, &state)))
});
```
