//! Normalizer — parses raw log bytes into structured [`LogEntry`](crate::LogEntry) values.
//!
//! Parsing is attempted in order: JSON → logfmt → common-pattern regexes → fallback.
