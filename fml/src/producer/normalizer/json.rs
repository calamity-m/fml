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

    Some(NewLogEntry {
        msg: todo!(), // TOOD - assign to "msg" or "message" field, whichever is present
        ts,
        level,
        source: source.clone(),
        fields: fields,
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
