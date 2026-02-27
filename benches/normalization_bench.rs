#![allow(unused)]
//! Normalizer throughput benchmarks.
//!
//! Measures how fast the normalizer can parse raw log bytes into `LogEntry`
//! structs. The normalizer is on the hot path for every ingested line, so
//! even small regressions compound at scale.
//!
//! # Groups
//!
//! | Group | What it measures |
//! |-------|-----------------|
//! | `json` | Parse throughput for compact, nested, and large JSON lines |
//! | `logfmt` | Parse throughput for short and long logfmt lines |
//! | `unstructured` | Heuristic regex detection on plain-text lines |
//! | `mixed_corpus` | Realistic mixed corpus (JSON + logfmt + unstructured) |
//!
//! # Viewing results
//!
//! ```sh
//! cargo bench --bench normalization_bench
//! open target/criterion/report/index.html
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn json_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("json");

    let compact = r#"{"ts":"2024-01-15T10:00:00Z","level":"INFO","msg":"ok"}"#;
    let nested = r#"{"ts":"2024-01-15T10:00:00Z","level":"ERROR","context":{"request":{"id":"abc","path":"/api"},"user":{"id":42}}}"#;
    let large = {
        // Build a large JSON line with 50 fields at bench time.
        // (We can't use const here, so we leak it as a String.)
        let mut obj = serde_json::Map::new();
        obj.insert("ts".to_string(), "2024-01-15T10:00:00Z".into());
        obj.insert("level".to_string(), "INFO".into());
        for i in 0..50usize {
            obj.insert(
                format!("field_{i}"),
                serde_json::Value::String(format!("value_{i}")),
            );
        }
        serde_json::to_string(&obj).unwrap()
    };

    group.throughput(Throughput::Elements(1));

    group.bench_with_input(BenchmarkId::new("compact", ""), &compact, |b, line| {
        b.iter(|| {
            // TODO: call normalizer::parse(line); black_box the result.
            todo!("json compact bench")
        })
    });

    group.bench_with_input(BenchmarkId::new("nested", ""), &nested, |b, line| {
        b.iter(|| todo!("json nested bench"))
    });

    group.bench_with_input(
        BenchmarkId::new("large_50_fields", ""),
        &large,
        |b, line| b.iter(|| todo!("json large bench")),
    );

    group.finish();
}

// ---------------------------------------------------------------------------
// Logfmt
// ---------------------------------------------------------------------------

fn logfmt_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("logfmt");

    let short = "ts=2024-01-15T10:00:00Z level=info msg=ok";
    let long = "ts=2024-01-15T10:00:00Z level=error msg=\"Connection refused\" host=db.internal \
                 port=5432 retry=3 duration_ms=4200 request_id=req-abc123 user_id=usr-999 \
                 service=payment-gateway region=us-east-1 error=\"dial tcp: connection refused\"";

    group.throughput(Throughput::Elements(1));

    group.bench_with_input(BenchmarkId::new("short", ""), &short, |b, line| {
        b.iter(|| todo!("logfmt short bench"))
    });

    group.bench_with_input(BenchmarkId::new("long", ""), &long, |b, line| {
        b.iter(|| todo!("logfmt long bench"))
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Unstructured
// ---------------------------------------------------------------------------

fn unstructured_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("unstructured");

    let lines = [
        "2024-01-15 10:00:00 INFO  Starting application version 2.4.1",
        "2024-01-15 10:00:01 ERROR Failed to connect to database after 3 retries",
        "[2024-01-15T10:00:03Z] WARN: Disk usage at 92% on /dev/sda1",
    ];

    group.throughput(Throughput::Elements(lines.len() as u64));

    group.bench_function("heuristic_regex", |b| {
        b.iter(|| {
            for line in &lines {
                // TODO: call normalizer::parse(line); black_box result.
                todo!("unstructured bench")
            }
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Mixed corpus
// ---------------------------------------------------------------------------

fn mixed_corpus_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_corpus");

    // 1 000 lines mixing JSON, logfmt, and unstructured in a 60/30/10 split.
    group.bench_function("1000_lines", |b| {
        b.iter(|| {
            // TODO: iterate over a pre-built corpus of 1 000 mixed lines;
            // call normalizer::parse on each; collect into Vec<LogEntry>.
            todo!("mixed corpus bench")
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion registration
// ---------------------------------------------------------------------------

criterion_group!(
    normalization_benches,
    json_bench,
    logfmt_bench,
    unstructured_bench,
    mixed_corpus_bench,
);
criterion_main!(normalization_benches);
