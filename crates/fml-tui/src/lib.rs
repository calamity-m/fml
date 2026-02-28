//! fml TUI — ratatui application shell.

pub mod app;
pub mod commands;
pub mod event;
pub mod theme;
pub mod widgets;

pub use app::App;

/// Start the TUI with hardcoded mock data (Phase 2 entry point).
pub fn run() -> anyhow::Result<()> {
    let config =
        fml_core::config::Config::load().unwrap_or_else(|_| fml_core::config::Config::defaults());
    let theme = theme::Theme::load_default();
    App::new(mock_entries(), config, theme).run()
}

// ---------------------------------------------------------------------------
// Mock data — replaced by real feeds in Phase 4
// ---------------------------------------------------------------------------

fn mock_entries() -> Vec<fml_core::LogEntry> {
    use chrono::{Duration, Utc};
    use fml_core::{FeedKind, LogEntry, LogLevel};

    let now = Utc::now();

    // (producer, level, message template)
    type Template = (&'static str, LogLevel, &'static str);
    const NORMAL: &[Template] = &[
        ("api-7f9b4d", LogLevel::Debug, "GET /healthz 200 OK (1ms)"),
        (
            "api-7f9b4d",
            LogLevel::Info,
            "GET /api/v1/users 200 OK (12ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Info,
            "POST /api/v1/orders 201 Created (38ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Info,
            "GET /api/v1/products 200 OK (8ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Info,
            "PUT /api/v1/users/42 200 OK (22ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Info,
            "DELETE /api/v1/sessions/99 204 No Content (4ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Debug,
            "Database connection acquired from pool (pool=18/20)",
        ),
        ("api-7f9b4d", LogLevel::Debug, "Cache hit ratio: 94.2%"),
        (
            "api-7f9b4d",
            LogLevel::Debug,
            "trace: dial tcp 10.0.0.5:5432 established",
        ),
        (
            "api-7f9b4d",
            LogLevel::Trace,
            "Middleware chain completed in 0.4ms",
        ),
        ("worker-4c2a", LogLevel::Debug, "Polling queue — 0 messages"),
        ("worker-4c2a", LogLevel::Debug, "Queue depth: 14 pending"),
        ("worker-4c2a", LogLevel::Info, "Dequeued job type=email"),
        ("worker-4c2a", LogLevel::Info, "Job completed in 2100ms"),
        (
            "worker-4c2a",
            LogLevel::Info,
            "Dequeued job type=report_generate",
        ),
        ("worker-4c2a", LogLevel::Info, "Job completed in 8400ms"),
        (
            "worker-4c2a",
            LogLevel::Info,
            "Dequeued job type=thumbnail_resize",
        ),
        ("worker-4c2a", LogLevel::Info, "Job completed in 340ms"),
        (
            "worker-4c2a",
            LogLevel::Debug,
            "Heartbeat sent to coordinator",
        ),
        ("worker-9e1b", LogLevel::Debug, "Polling queue — 0 messages"),
        (
            "worker-9e1b",
            LogLevel::Info,
            "Dequeued job type=data_export",
        ),
        ("worker-9e1b", LogLevel::Info, "Job completed in 12300ms"),
        (
            "worker-9e1b",
            LogLevel::Debug,
            "Heartbeat sent to coordinator",
        ),
        (
            "worker-9e1b",
            LogLevel::Info,
            "Dequeued job type=notification",
        ),
        ("worker-9e1b", LogLevel::Info, "Job completed in 190ms"),
    ];
    const WARN: &[Template] = &[
        (
            "api-7f9b4d",
            LogLevel::Warn,
            "Slow query: SELECT * FROM orders WHERE … (1240ms, threshold 500ms)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Warn,
            "Connection pool near capacity (18/20)",
        ),
        (
            "api-7f9b4d",
            LogLevel::Warn,
            "Response time p99 exceeded SLO: 2.4s > 2.0s",
        ),
        (
            "worker-4c2a",
            LogLevel::Warn,
            "Memory pressure: heap 78% (threshold 75%)",
        ),
        (
            "worker-4c2a",
            LogLevel::Warn,
            "Job queue backlog growing: 47 pending",
        ),
        (
            "worker-9e1b",
            LogLevel::Warn,
            "Disk I/O wait elevated: 340ms avg",
        ),
    ];
    const ERROR: &[Template] = &[
        (
            "api-7f9b4d",
            LogLevel::Error,
            "Request timeout: GET /api/v1/search after 30s",
        ),
        (
            "api-7f9b4d",
            LogLevel::Error,
            "Upstream error: POST http://payments:8080 503 Service Unavailable",
        ),
        (
            "worker-9e1b",
            LogLevel::Error,
            "Connection refused: redis:6379 — retrying in 5s",
        ),
        (
            "worker-9e1b",
            LogLevel::Error,
            "Job failed after 3 attempts: type=data_export",
        ),
    ];
    const FATAL: &[Template] = &[(
        "worker-9e1b",
        LogLevel::Fatal,
        "panic: runtime error: index out of range [3] with length 3",
    )];

    // Weights: normal=85%, warn=9%, error=5%, fatal=1%
    // Map index mod 100 → category
    let pick = |i: u64| -> &Template {
        let slot = i % 100;
        if slot < 85 {
            &NORMAL[(i as usize) % NORMAL.len()]
        } else if slot < 94 {
            &WARN[(i as usize) % WARN.len()]
        } else if slot < 99 {
            &ERROR[(i as usize) % ERROR.len()]
        } else {
            &FATAL[(i as usize) % FATAL.len()]
        }
    };

    const TOTAL: u64 = 1_200;
    // Spread entries evenly over the last 60 minutes
    let interval_ms = 60 * 60 * 1000 / TOTAL as i64;

    (0..TOTAL)
        .map(|i| {
            let (producer, level, base_msg) = pick(i);
            let ts = now - Duration::milliseconds((TOTAL - i) as i64 * interval_ms);

            // Inject a unique token every ~25 lines so entries aren't identical
            let message = if i % 25 == 0 {
                format!("{} [req={}]", base_msg, i * 7919 % 99991)
            } else {
                base_msg.to_string()
            };

            LogEntry {
                seq: i + 1,
                raw: message.clone(),
                ts,
                level: Some(*level),
                source: FeedKind::Docker,
                producer: producer.to_string(),
                fields: Default::default(),
                message: Some(message),
            }
        })
        .collect()
}
