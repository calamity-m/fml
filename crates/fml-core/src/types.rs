//! Core types for fml-core — Feed Me Logs.
//!
//! This module defines the fundamental data structures shared across all
//! architectural layers: the normalised [`LogEntry`], its [`LogLevel`], and
//! the [`FeedKind`] discriminant.

/// A normalised log entry produced by the ingestor and stored in the ring buffer.
///
/// Non-optional fields: `seq`, `raw`, `ts`, `source`, `producer`. The normalizer
/// populates the remaining fields as best-effort; they are `None` / empty when
/// the information could not be extracted from the raw line.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// Monotonically increasing sequence number assigned by the store on insert.
    /// Unique within a session; used for ordering and deduplication.
    pub seq: u64,
    /// Raw log line as received from the feed (UTF-8 lossy converted).
    pub raw: String,
    /// Ingest timestamp (UTC). May be overridden by a parsed timestamp from
    /// the log line itself if the normalizer detects one.
    pub ts: chrono::DateTime<chrono::Utc>,
    /// Log level, best-effort parsed from the line.
    pub level: Option<LogLevel>,
    /// Feed type that produced this entry.
    pub source: FeedKind,
    /// Producer name (pod, container, filename, …).
    pub producer: String,
    /// Structured fields extracted during normalisation. Keys are lowercase.
    /// For JSON logs these are all top-level keys; for logfmt they are the
    /// parsed key-value pairs.
    pub fields: std::collections::HashMap<String, serde_json::Value>,
    /// Human-readable message text, if one could be identified. Populated by
    /// the normalizer when a recognised message key is found (e.g. `"message"`,
    /// `"msg"` in JSON or logfmt). Falls back to the full raw line for
    /// unstructured input where no message key exists.
    pub message: Option<String>,
}

/// Log severity level, normalised across all feed types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "TRACE"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Fatal => write!(f, "FATAL"),
        }
    }
}

/// Which feed produced a log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedKind {
    Kubernetes,
    Docker,
    File,
    Stdin,
}

impl std::fmt::Display for FeedKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedKind::Kubernetes => write!(f, "kubernetes"),
            FeedKind::Docker => write!(f, "docker"),
            FeedKind::File => write!(f, "file"),
            FeedKind::Stdin => write!(f, "stdin"),
        }
    }
}
