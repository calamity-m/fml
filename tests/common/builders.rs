//! Test builders — ergonomic constructors for `LogEntry`, `Store`, and queries.
//!
//! These builders are designed for readability in test assertions, not for
//! production use. They panic on invalid input rather than returning `Result`.

use fml_core::{FeedKind, LogEntry, LogLevel};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// LogEntryBuilder
// ---------------------------------------------------------------------------

/// Fluent builder for [`LogEntry`] test fixtures.
///
/// # Example
///
/// ```rust
/// let entry = LogEntryBuilder::new("timeout connecting to db")
///     .level(LogLevel::Error)
///     .producer("api-7f9b4d")
///     .source(FeedKind::Kubernetes)
///     .field("request_id", "req-abc123")
///     .build();
/// ```
pub struct LogEntryBuilder {
    raw: String,
    ts: chrono::DateTime<chrono::Utc>,
    level: Option<LogLevel>,
    source: FeedKind,
    producer: String,
    fields: HashMap<String, serde_json::Value>,
    message: Option<String>,
}

impl LogEntryBuilder {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            raw: raw.clone(),
            ts: chrono::Utc::now(),
            level: None,
            source: FeedKind::Stdin,
            producer: "test-producer".to_string(),
            fields: HashMap::new(),
            message: Some(raw),
        }
    }

    pub fn level(mut self, level: LogLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn source(mut self, source: FeedKind) -> Self {
        self.source = source;
        self
    }

    pub fn producer(mut self, producer: impl Into<String>) -> Self {
        self.producer = producer.into();
        self
    }

    pub fn ts(mut self, ts: chrono::DateTime<chrono::Utc>) -> Self {
        self.ts = ts;
        self
    }

    pub fn field(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn build(self) -> LogEntry {
        LogEntry {
            raw: self.raw,
            ts: self.ts,
            level: self.level,
            source: self.source,
            producer: self.producer,
            fields: self.fields,
            message: self.message,
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

/// Build an INFO entry.
pub fn info_entry(message: &str) -> LogEntry {
    LogEntryBuilder::new(message).level(LogLevel::Info).build()
}

/// Build an ERROR entry.
pub fn error_entry(message: &str) -> LogEntry {
    LogEntryBuilder::new(message).level(LogLevel::Error).build()
}

/// Build a Kubernetes pod entry.
pub fn k8s_entry(pod: &str, message: &str, level: LogLevel) -> LogEntry {
    LogEntryBuilder::new(message)
        .source(FeedKind::Kubernetes)
        .producer(pod)
        .level(level)
        .build()
}

/// Build a Docker container entry.
pub fn docker_entry(container: &str, message: &str, level: LogLevel) -> LogEntry {
    LogEntryBuilder::new(message)
        .source(FeedKind::Docker)
        .producer(container)
        .level(level)
        .build()
}

// ---------------------------------------------------------------------------
// Corpus helpers
// ---------------------------------------------------------------------------

/// Parse a raw log line string into a `LogEntry` via the normalizer (once
/// implemented). Until then, wraps it in a minimal `LogEntryBuilder`.
pub fn entry_from_raw(raw: &str) -> LogEntry {
    LogEntryBuilder::new(raw).build()
}

/// Build a corpus of `n` `LogEntry` values alternating INFO/WARN/ERROR.
pub fn build_corpus(n: usize) -> Vec<LogEntry> {
    (0..n)
        .map(|i| {
            let level = match i % 10 {
                0 => LogLevel::Error,
                1 | 2 => LogLevel::Warn,
                _ => LogLevel::Info,
            };
            LogEntryBuilder::new(format!("log line {i}"))
                .level(level)
                .producer(format!("producer-{}", i % 3))
                .build()
        })
        .collect()
}
