.PHONY: ci release all check clippy fmt test test-verbose bench bench-check save-baselines clean

# Run the full verification suite (what CI should run)
ci: fmt check clippy test

# Alias for muscle memory
all: ci

# Build optimised release binary
release:
	cargo build --release

# Type-check without linking
check:
	cargo check --all-targets

# Lint (treat warnings as errors in CI; remove -D warnings locally if noisy)
clippy:
	cargo clippy --all-targets -- -D warnings

# Format check (use `make fmt-fix` to apply)
fmt:
	cargo fmt --check

fmt-fix:
	cargo fmt

# Run all tests (stubs are ignored, so this just confirms compilation + any real tests pass)
test:
	cargo test --all

# Same, but with output so you can see which planned tests are pending
test-verbose:
	cargo test --all -- --nocapture

# Run a single harness by name, e.g.: make test-one HARNESS=search_harness
test-one:
	cargo test --test $(HARNESS)

# Run all ignored (stub) tests — useful for reviewing what's planned
test-stubs:
	cargo test --all -- --include-ignored 2>&1 | grep -E "^test |FAILED|panicked" | head -60

# Compile benchmarks without running them (fast sanity check)
bench-check:
	cargo bench --no-run

# Run all benchmarks (empty stubs complete instantly; real benches produce HTML reports)
bench:
	cargo bench

# Save Criterion results as a named baseline; default label = current branch
# Usage: make save-baselines
#        make save-baselines LABEL=my-feature
save-baselines:
	scripts/save_baselines.sh $(or $(LABEL),)

# Update insta snapshots interactively after intentional format changes
snapshots:
	cargo test --test normalization_harness; cargo test --test export_harness; cargo insta review

clean:
	cargo clean
