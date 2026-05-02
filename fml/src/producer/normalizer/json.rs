//! JSON log line parser.
//!
//! Attempts to parse the raw line as a JSON object.

use std::collections::{BTreeMap, HashMap};

use crate::log::{LogLevel, NewLogEntry, Source};

/// Well-known keys for log level detection.
const LEVEL_KEYS: &[&str] = &["level", "severity", "lvl", "log.level"];

/// Well-known keys for timestamp detection.
const TIMESTAMP_KEYS: &[&str] = &["ts", "timestamp", "time", "@timestamp", "t"];

const MESSAGE_KEYS: &[&str] = &["msg", "message"];

/// Try to parse `raw` as a JSON object. Returns extracted fields on success.
///
/// All top-level keys are promoted to `fields`. Well-known keys for level and
/// timestamp are detected and mapped into the parsed metadata.
pub fn try_parse_json(raw: &str, source: &Source) -> Option<NewLogEntry> {
    let obj: serde_json::Map<String, serde_json::Value> = serde_json::from_str(raw).ok()?;

    let mut fields = HashMap::new();
    let mut level = None;
    let mut ts = chrono::Utc::now();
    let mut ts_parsed = false;

    for (key, value) in &obj {
        let lower_key = key.to_lowercase();

        // Detect level
        if level.is_none()
            && LEVEL_KEYS.contains(&lower_key.as_str())
            && let Some(s) = value.as_str()
        {
            level = LogLevel::parse_level(s);
        }

        // Detect timestamp
        if !ts_parsed
            && TIMESTAMP_KEYS.contains(&lower_key.as_str())
            && let Some(s) = value.as_str()
            && let Some(parsed) = try_parse_timestamp(s)
        {
            ts = parsed;
            ts_parsed = true;
        }

        // All top-level keys go into fields
        fields.insert(lower_key, value.clone());
    }

    let msg = MESSAGE_KEYS
        .iter()
        .find_map(|k| obj.get(*k)?.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string());

    Some(NewLogEntry {
        msg,
        ts,
        level,
        source: source.clone(),
        fields,
    })
}

/// Try to parse a timestamp string into a `DateTime<Utc>`.
pub fn try_parse_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, NaiveDateTime, Utc};

    // Try RFC 3339 / ISO 8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try common formats
    let formats = [
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
    ];

    for fmt in &formats {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(ndt.and_utc());
        }
    }

    None
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
    fn msg_field_used_as_msg() {
        let entry = try_parse_json(r#"{"level":"info","msg":"hello world"}"#, &source()).unwrap();
        assert_eq!(entry.msg, "hello world");
    }

    #[test]
    fn message_field_used_as_msg() {
        let entry =
            try_parse_json(r#"{"level":"info","message":"hello world"}"#, &source()).unwrap();
        assert_eq!(entry.msg, "hello world");
    }

    #[test]
    fn msg_wins_over_message() {
        let entry =
            try_parse_json(r#"{"msg":"from msg","message":"from message"}"#, &source()).unwrap();
        assert_eq!(entry.msg, "from msg");
    }

    #[test]
    fn falls_back_to_raw_when_no_msg_key() {
        let raw = r#"{"level":"info","data":"no message key here"}"#;
        let entry = try_parse_json(raw, &source()).unwrap();
        assert_eq!(entry.msg, raw);
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(try_parse_json("not json at all", &source()).is_none());
    }
}
