#![allow(unused)]
//! Store throughput and scaling benchmarks.
//!
//! Measures insert and read performance of the in-memory ring buffer at
//! various sizes and concurrency levels.
//!
//! # Groups
//!
//! | Group | What it measures |
//! |-------|-----------------|
//! | `insert` | Single-threaded insert throughput at 1k/10k/100k/at-capacity |
//! | `read` | Read throughput for range, by-producer, and latest-N queries |
//! | `concurrent` | Throughput under 1-writer-5-readers and 5-writers-5-readers |
//! | `scaling` | Insert + read throughput as capacity grows from 1k to 1M |
//!
//! # Viewing results
//!
//! ```sh
//! cargo bench --bench store_bench
//! open target/criterion/report/index.html
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// ---------------------------------------------------------------------------
// Insert throughput
// ---------------------------------------------------------------------------

fn insert_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert");

    for entry_count in [1_000usize, 10_000, 100_000] {
        group.throughput(Throughput::Elements(entry_count as u64));
        group.bench_with_input(
            BenchmarkId::new("sequential", entry_count),
            &entry_count,
            |b, &n| {
                b.iter(|| {
                    // TODO: create a store with capacity = n; insert n entries;
                    // assert store.len() == n.
                    todo!("insert sequential bench: {} entries", n)
                })
            },
        );
    }

    // At-capacity: every insert evicts the oldest entry.
    group.bench_function("at_capacity_10k", |b| {
        b.iter(|| {
            // TODO: create store with capacity 10_000; insert 20_000 entries;
            // assert store.len() == 10_000.
            todo!("insert at-capacity bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Read throughput
// ---------------------------------------------------------------------------

fn read_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("read");

    // Range read: return all entries between two sequence numbers.
    group.bench_function("range_10k_store", |b| {
        b.iter(|| {
            // TODO: build a 10k-entry store; call store.range(seq_from, seq_to);
            // black_box the result.
            todo!("read range bench")
        })
    });

    // By-producer: filter entries by producer name.
    group.bench_function("by_producer_10k_store", |b| {
        b.iter(|| {
            // TODO: build a 10k-entry store with 5 producers; filter by one.
            todo!("read by_producer bench")
        })
    });

    // Latest N entries.
    group.bench_function("latest_100_of_10k", |b| {
        b.iter(|| {
            // TODO: build a 10k-entry store; call store.latest(100); black_box.
            todo!("read latest bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Concurrent access
// ---------------------------------------------------------------------------

fn concurrent_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent");

    // 1 writer, 5 readers — typical live-tail scenario.
    group.bench_function("1w5r", |b| {
        b.iter(|| {
            // TODO: spawn 1 writer task inserting 10k entries and 5 reader tasks
            // calling store.latest(100) in a loop; measure total time.
            todo!("concurrent 1w5r bench")
        })
    });

    // 5 writers, 5 readers — worst-case multi-feed scenario.
    group.bench_function("5w5r", |b| {
        b.iter(|| {
            // TODO: spawn 5 writer tasks and 5 reader tasks; measure throughput.
            todo!("concurrent 5w5r bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Scaling: capacity axis
// ---------------------------------------------------------------------------

fn scaling_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");

    for capacity in [1_000usize, 10_000, 100_000, 1_000_000] {
        group.throughput(Throughput::Elements(capacity as u64));
        group.bench_with_input(
            BenchmarkId::new("insert_then_scan", capacity),
            &capacity,
            |b, &cap| {
                b.iter(|| {
                    // TODO: build a store with the given capacity; fill it;
                    // then scan all entries. Reveals O(n) vs O(1) regressions.
                    todo!("scaling bench: capacity={}", cap)
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
    store_benches,
    insert_bench,
    read_bench,
    concurrent_bench,
    scaling_bench,
);
criterion_main!(store_benches);
