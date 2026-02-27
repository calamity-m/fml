#![allow(unused)]
//! Search engine benchmarks.
//!
//! Measures the performance of the query pipeline at all greed levels and
//! store sizes. This is the most critical benchmark suite because the greedy
//! expansion algorithm must remain fast even at high greed and large store
//! sizes.
//!
//! # Groups
//!
//! | Group | What it measures |
//! |-------|-----------------|
//! | `exact` | Literal substring and key-value filter throughput |
//! | `greedy/expansion_only` | Cost of FST-backed term expansion at greed 1/3/7/10 |
//! | `greedy/full_pipeline` | Full search pipeline (expansion + filter + rank) at each greed level |
//! | `fst/prefix_scan` | Raw FST prefix scan throughput, independent of search pipeline |
//! | `scaling` | Full-pipeline throughput as store size grows from 1k to 1M |
//!
//! # Key performance targets (aspirational, not enforced in CI yet)
//!
//! - Exact search on a 100k-entry store: < 10 ms
//! - Greed=7 full pipeline on a 100k-entry store: < 50 ms
//! - FST prefix scan over 10k terms: < 1 ms
//!
//! # Viewing results
//!
//! ```sh
//! cargo bench --bench search_bench
//! open target/criterion/report/index.html
//! # Compare against saved baseline:
//! critcmp main candidate --threshold 5
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// ---------------------------------------------------------------------------
// Exact search
// ---------------------------------------------------------------------------

fn exact_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("exact");

    // 50% hit rate — half the store matches the query.
    group.bench_function("50pct_hit_rate_10k_store", |b| {
        b.iter(|| {
            // TODO: build 10k-entry store where 5k match "timeout";
            // greed=0 query "timeout"; assert result count ~5k.
            todo!("exact 50% hit rate bench")
        })
    });

    // 1% hit rate — only rare lines match.
    group.bench_function("1pct_hit_rate_10k_store", |b| {
        b.iter(|| {
            // TODO: build 10k-entry store where 100 match "timeout";
            // greed=0 query "timeout"; assert result count ~100.
            todo!("exact 1% hit rate bench")
        })
    });

    // Key-value filter: `level:error` restricts to a subset before scanning.
    group.bench_function("kv_filter_level_error", |b| {
        b.iter(|| {
            // TODO: build 10k-entry store; query "level:error"; measure filter step.
            todo!("exact kv filter bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Greedy expansion only (FST + graph traversal, no store scan)
// ---------------------------------------------------------------------------

fn greedy_expansion_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("greedy/expansion_only");

    for greed in [1u8, 3, 7, 10] {
        group.bench_with_input(BenchmarkId::new("auth", greed), &greed, |b, &g| {
            b.iter(|| {
                // TODO: call search::expand("auth", g); black_box the
                // returned term set. Does NOT scan the store.
                todo!("greedy expansion bench: greed={}", g)
            })
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Greedy full pipeline (expansion + store scan + ranking)
// ---------------------------------------------------------------------------

fn greedy_full_pipeline_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("greedy/full_pipeline");

    for greed in [1u8, 3, 7, 10] {
        group.bench_with_input(
            BenchmarkId::new("auth_10k_store", greed),
            &greed,
            |b, &g| {
                b.iter(|| {
                    // TODO: build 10k-entry store with auth-domain lines;
                    // run full search pipeline with query="auth" greed=g;
                    // black_box results.
                    todo!("greedy full pipeline bench: greed={}", g)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Raw FST prefix scan
// ---------------------------------------------------------------------------

fn fst_prefix_scan_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("fst");

    group.bench_function("prefix_scan_10k_terms", |b| {
        b.iter(|| {
            // TODO: build an FST from 10k terms; run a prefix scan for "auth";
            // black_box the result count.
            todo!("fst prefix scan bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Scaling: store size axis
// ---------------------------------------------------------------------------

fn scaling_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");

    for store_size in [1_000usize, 10_000, 100_000, 1_000_000] {
        group.throughput(Throughput::Elements(store_size as u64));
        group.bench_with_input(
            BenchmarkId::new("greed7_auth", store_size),
            &store_size,
            |b, &n| {
                b.iter(|| {
                    // TODO: build store of size n; run greed=7 "auth" query;
                    // black_box result count.
                    todo!("scaling bench: store_size={}", n)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion registration
// ---------------------------------------------------------------------------

criterion_group!(
    search_benches,
    exact_bench,
    greedy_expansion_bench,
    greedy_full_pipeline_bench,
    fst_prefix_scan_bench,
    scaling_bench,
);
criterion_main!(search_benches);
