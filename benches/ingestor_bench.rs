#![allow(unused)]
//! Ingestor throughput and latency benchmarks.
//!
//! Measures how fast each feed type can push lines from the source into the
//! store. Two dimensions:
//!
//! - **Throughput** (lines/s) — how many lines per second the ingestor
//!   can sustain under load.
//! - **Latency** (p50/p99) — time from a line appearing at the source to the
//!   entry being readable from the store.
//!
//! # Groups
//!
//! | Group | What it measures |
//! |-------|-----------------|
//! | `kubernetes/throughput` | Lines/s at 1, 10, and 50 concurrent pod streams |
//! | `kubernetes/latency` | p50 and p99 time-to-store for a single pod |
//! | `docker/throughput` | Lines/s multiplexed across N containers |
//! | `file/throughput` | Lines/s from inotify-based file tail |
//! | `stdin/burst` | Lines/s when all lines arrive in a single burst |
//!
//! # Viewing results
//!
//! ```sh
//! cargo bench --bench ingestor_bench
//! open target/criterion/report/index.html
//! ```
//!
//! Requires `gnuplot` for graph rendering. On Ubuntu: `sudo apt install gnuplot`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

// ---------------------------------------------------------------------------
// Kubernetes throughput
// ---------------------------------------------------------------------------

fn kubernetes_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("kubernetes/throughput");

    for pod_count in [1usize, 10, 50] {
        group.bench_with_input(
            BenchmarkId::new("pods", pod_count),
            &pod_count,
            |b, &pods| {
                b.iter(|| {
                    // TODO: spin up `pods` FakeProcessSpawner streams, each
                    // emitting 1 000 lines, feed through the kubernetes ingestor,
                    // and drain the store. Return the store len.
                    todo!("kubernetes throughput bench: {} pods", pods)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Kubernetes latency
// ---------------------------------------------------------------------------

fn kubernetes_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("kubernetes/latency");

    // p50 — median latency
    group.bench_function("p50", |b| {
        b.iter(|| {
            // TODO: measure time from line emission to store insertion.
            // Run 1 000 iterations, collect latencies, report p50.
            todo!("kubernetes latency p50 bench")
        })
    });

    // p99 — tail latency
    group.bench_function("p99", |b| {
        b.iter(|| {
            // TODO: same as p50 but report p99 of the latency distribution.
            todo!("kubernetes latency p99 bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Docker throughput
// ---------------------------------------------------------------------------

fn docker_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("docker/throughput");

    for container_count in [1usize, 5, 20] {
        group.bench_with_input(
            BenchmarkId::new("containers", container_count),
            &container_count,
            |b, &containers| {
                b.iter(|| {
                    // TODO: use FakeDockerApi; stream 1 000 lines per container;
                    // measure total throughput including frame decoding.
                    todo!("docker throughput bench: {} containers", containers)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// File throughput
// ---------------------------------------------------------------------------

fn file_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("file/throughput");

    group.bench_function("single_file", |b| {
        b.iter(|| {
            // TODO: write 10 000 lines to a tempfile; measure time from
            // first write to all lines appearing in the store.
            todo!("file throughput bench: single file")
        })
    });

    group.bench_function("10_files_glob", |b| {
        b.iter(|| {
            // TODO: write 1 000 lines to each of 10 tempfiles matching a glob;
            // measure total throughput.
            todo!("file throughput bench: 10 files via glob")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Stdin burst
// ---------------------------------------------------------------------------

fn stdin_burst(c: &mut Criterion) {
    let mut group = c.benchmark_group("stdin/burst");

    for line_count in [1_000usize, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("lines", line_count),
            &line_count,
            |b, &lines| {
                b.iter(|| {
                    // TODO: write `lines` log lines to a pipe in a single write;
                    // measure time until all lines are in the store.
                    todo!("stdin burst bench: {} lines", lines)
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
    ingestor_benches,
    kubernetes_throughput,
    kubernetes_latency,
    docker_throughput,
    file_throughput,
    stdin_burst,
);
criterion_main!(ingestor_benches);
