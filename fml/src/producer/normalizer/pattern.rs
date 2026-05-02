//! Unstructured log line pattern detection.
//!
//! Uses regex patterns to detect log levels, timestamps, and request IDs
//! in unstructured plain-text log lines. Only returns `Some` if at least
//! a level or timestamp was detected.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::{collections::BTreeMap, hash::Hash};

use crate::log::{LogLevel, NewLogEntry, Source};

static LEVEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(TRACE|DEBUG|INFO|WARN(?:ING)?|ERROR|ERR|FATAL|CRIT(?:ICAL)?)\b").unwrap()
});

static TIMESTAMP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        \[?                                  # optional opening bracket
        (\d{4}-\d{2}-\d{2}                   # date part
        [T\x20]                               # separator (T or space)
        \d{2}:\d{2}:\d{2}                    # time part
        (?:\.\d{1,6})?                        # optional fractional seconds
        (?:Z|[+-]\d{2}:?\d{2})?)             # optional timezone
        \]?                                   # optional closing bracket
    ",
    )
    .unwrap()
});

static REQUEST_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(req-[a-zA-Z0-9]+|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\b",
    )
    .unwrap()
});

/// Try to detect structure in an unstructured log line via regex patterns.
/// Returns parsed fields if at least a level or timestamp was detected.
pub fn try_parse_patterns(raw: &str, source: &Source) -> Option<NewLogEntry> {
    let level = LEVEL_RE
        .find(raw)
        .and_then(|m| LogLevel::parse_level(m.as_str()));

    let ts = TIMESTAMP_RE
        .captures(raw)
        .and_then(|c| c.get(1))
        .and_then(|m| super::json::try_parse_timestamp(m.as_str()));

    let request_id = REQUEST_ID_RE.find(raw).map(|m| m.as_str().to_string());

    // Only return Some if we detected at least a level or timestamp
    if level.is_none() && ts.is_none() {
        return None;
    }

    let mut fields = HashMap::new();
    if let Some(ref req_id) = request_id {
        fields.insert(
            "request_id".to_string(),
            serde_json::Value::String(req_id.clone()),
        );
    }

    Some(NewLogEntry {
        msg: raw.to_string(),
        ts: ts.unwrap_or_else(chrono::Utc::now),
        level,
        source: source.clone(),
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            producer: "test".to_string(),
            id: "test-id".to_string(),
            display_name: "Test".to_string(),
            group: None,
        }
    }

    #[test]
    fn line_with_level_returns_full_raw_as_msg() {
        let raw = "2024-01-01T00:00:00Z INFO starting service";
        let entry = try_parse_patterns(raw, &source()).unwrap();
        assert_eq!(entry.msg, raw);
    }

    #[test]
    fn line_with_timestamp_returns_full_raw_as_msg() {
        let raw = "2024-01-01T00:00:00Z starting service";
        let entry = try_parse_patterns(raw, &source()).unwrap();
        assert_eq!(entry.msg, raw);
    }

    #[test]
    fn line_with_no_level_or_timestamp_returns_none() {
        assert!(
            try_parse_patterns("just a plain message with nothing parseable", &source()).is_none()
        );
    }
}
