> **fml** — *Feed Me Logs*
>
> *The log triage tool you open when something is already broken. Built for high-stress, time-pressured moments — multi-source ingestion, semantic search that thinks ahead of you, and a UI that gets out of the way so you can find the line that matters before the incident gets worse.*

`fml` is a terminal TUI for log aggregation, triage, and searching.

## Documentation

| Doc | Contents |
|-----|----------|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | System layers, data flow, crate layout, source feeds |
| [`docs/tui/ARCHITECTURE.md`](docs/tui/ARCHITECTURE.md) | TUI internals: event loop, AppEvent, focus FSM, widget pattern, rendering, themes |
| [`docs/FEATURES.md`](docs/FEATURES.md) | UI layout, keybindings, command bar, freeze/yank, correlation, export, headless mode |
| [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md) | Full `config.toml` reference and CLI flags |
| [`docs/TESTING.md`](docs/TESTING.md) | Test harnesses, running tests, benchmarks, flamegraphs |
| [`docs/search/GREEDY_ALGORITHM.md`](docs/search/GREEDY_ALGORITHM.md) | Greedy search expansion: graph structure, ontology, backwards resolution |

## Tech Stack

| Crate | Role |
|-------|------|
| [`ratatui`](https://crates.io/crates/ratatui) | TUI layout, widgets, and rendering |
| [`skim`](https://crates.io/crates/skim) | Fuzzy-find picker |
| [`fst`](https://crates.io/crates/fst) | Memory-mapped finite state transducer for term index |
| [`phf`](https://crates.io/crates/phf) | Compile-time perfect hash maps for ontology |
| [`clap`](https://crates.io/crates/clap) | CLI argument parsing |
| [`tracing`](https://crates.io/crates/tracing) | Structured debug logging (`--debug` flag) |

## Crate layout

```
fml/                  workspace root + binary (src/main.rs)
├── crates/
│   ├── fml-core/     LogEntry, Config, Store, Search, Normalizer
│   ├── fml-feeds/    Feed ingestors (docker, kubernetes, file, stdin)
│   └── fml-tui/      Ratatui app shell, widgets, themes, event system
└── docs/
```

## Expectations for Claude

When adding or modifying a feature:

1. **Run `cargo test --test <relevant_harness>`** before declaring the task done.
2. **Fill in stub tests** for any behaviour the change touches — a passing `#[ignore]` test is not a passing test.
3. **For search changes**, always run `cargo test --test search_harness` and explicitly check greed monotonicity holds end-to-end.
4. **For normalizer changes**, run `cargo insta review` and commit updated snapshots intentionally — never silently accept snapshot diffs.
5. **For performance-sensitive paths** (normalizer, search pipeline, store insert), run `cargo bench` before and after and check for regressions with `critcmp`. Applies from Phase 3 onward.
6. **For any regression or suspected bottleneck**, generate a flamegraph with `--profile-time 10` before touching code.
7. **When writing or updating documentation files**, use the `write-docs` skill
8. **When commiting** use the `git-commit` skill
9. **When starting and ending a new phase**, re-read and update the IMPLEMENTATION_PLAN.md document
10. **At the end of each phase**, run the `/code-review` skill to review all changes before merging via `gh pr`.
