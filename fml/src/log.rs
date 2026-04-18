use std::collections::HashMap;

use serde::Serialize;

pub type SourceId = String;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Source {
    pub id: SourceId,
}

/// Log severity level, normalised across all feed types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

impl LogLevel {
    pub fn parse_level(s: &str) -> Option<LogLevel> {
        match s.to_uppercase().as_str() {
            "TRACE" => Some(LogLevel::Trace),
            "DEBUG" => Some(LogLevel::Debug),
            "INFO" => Some(LogLevel::Info),
            "WARN" | "WARNING" => Some(LogLevel::Warn),
            "ERROR" | "ERR" => Some(LogLevel::Error),
            "FATAL" | "CRITICAL" | "CRIT" => Some(LogLevel::Fatal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogEntry {
    /// Monotonically increasing sequence number assigned by the store on insert.
    /// Used for ordering and deduplication.
    pub seq: u64,
    /// Parsed display message of the log line
    pub msg: String,
    /// Ingest timestamp (UTC) or log entry's parsed timestamp
    pub ts: chrono::DateTime<chrono::Utc>,
    /// Log level if normaliser finds one
    pub level: Option<LogLevel>,
    /// Source that produced this log entry
    pub source: Source,
    /// Ordered map of values found from the raw log entry, besides level/msg/ts
    pub fields: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NewLogEntry {
    /// Parsed display message of the log line
    pub msg: String,
    /// Ingest timestamp (UTC) or log entry's parsed timestamp
    pub ts: chrono::DateTime<chrono::Utc>,
    /// Log level if normaliser finds one
    pub level: Option<LogLevel>,
    /// Source that produced this log entry
    pub source: Source,
    /// Map of values found from the raw log entry, besides level/msg/ts
    pub fields: HashMap<String, serde_json::Value>,
}
