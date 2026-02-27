#![allow(unused)]
//! Export throughput and latency benchmarks.
//!
//! Measures how fast the exporter can serialise log entries to each output
//! format, and how quickly the first byte appears (time-to-first-byte).
//!
//! # Groups
//!
//! | Group | What it measures |
//! |-------|-----------------|
//! | `throughput` | Entries/s for raw/jsonl/csv × 10k/100k entry counts |
//! | `latency` | Time-to-first-byte for each format at 10k entries |
//!
//! # Viewing results
//!
//! ```sh
//! cargo bench --bench export_bench
//! open target/criterion/report/index.html
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

#[derive(Debug, Clone, Copy)]
enum ExportFormat {
    Raw,
    Jsonl,
    Csv,
}

impl ExportFormat {
    fn name(self) -> &'static str {
        match self {
            ExportFormat::Raw => "raw",
            ExportFormat::Jsonl => "jsonl",
            ExportFormat::Csv => "csv",
        }
    }
}

// ---------------------------------------------------------------------------
// Throughput: format × entry count
// ---------------------------------------------------------------------------

fn throughput_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    let formats = [ExportFormat::Raw, ExportFormat::Jsonl, ExportFormat::Csv];
    let counts = [10_000usize, 100_000];

    for fmt in formats {
        for &count in &counts {
            group.throughput(Throughput::Elements(count as u64));
            group.bench_with_input(
                BenchmarkId::new(fmt.name(), count),
                &(fmt, count),
                |b, &(fmt, n)| {
                    b.iter(|| {
                        // TODO: build a store of n entries; run the exporter
                        // with format=fmt into a Vec<u8> sink; black_box the byte count.
                        todo!(
                            "export throughput bench: format={} entries={}",
                            fmt.name(),
                            n
                        )
                    })
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Latency: time-to-first-byte
// ---------------------------------------------------------------------------

fn latency_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency");

    let formats = [ExportFormat::Raw, ExportFormat::Jsonl, ExportFormat::Csv];

    for fmt in formats {
        group.bench_with_input(
            BenchmarkId::new("time_to_first_byte", fmt.name()),
            &fmt,
            |b, &fmt| {
                b.iter(|| {
                    // TODO: build a 10k-entry store; start the exporter;
                    // measure time until the first byte is written to the sink.
                    // This tests whether the exporter buffers unnecessarily.
                    todo!("export latency bench: format={}", fmt.name())
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion registration
// ---------------------------------------------------------------------------

criterion_group!(export_benches, throughput_bench, latency_bench);
criterion_main!(export_benches);
