//! fml — Feed Me Logs
//!
//! Terminal TUI for log aggregation, triage, and searching. This crate exposes
//! the four architectural layers as public modules so that integration tests
//! and the MCP server can import them directly.
//!
//! # Architecture
//!
//! ```text
//! Ingestor ──► Store ──► Search ──► UI
//!    │           │
//!    └───────────┴──► Export
//! ```
//!
//! All inter-layer communication uses `tokio` channels. The UI drives the
//! main thread; everything else runs on background tasks.

pub mod export;
pub mod ingestor;
pub mod normalizer;
pub mod search;
pub mod store;
pub mod ui;

/// A normalised log entry produced by the ingestor and stored in the ring buffer.
///
/// Every field is optional except `raw` and `ts`. The normalizer populates as
/// many fields as it can from the raw line; the remainder are left as `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
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
    /// The human-readable message field, if one could be identified.
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
