//! JSON log line parser.
//!
//! Attempts to parse the raw line as a JSON object.

use std::collections::HashMap;

use crate::log::{LogLevel, NewLogEntry, Source, TsSource};

/// Well-known top-level keys for string log level detection.
const LEVEL_KEYS: &[&str] = &[
    "level",
    "severity",
    "lvl",
    "log.level",
    "level_name",
    "levelname",
    "severity_text",
    "severitytext",
];

/// Well-known top-level keys for OpenTelemetry numeric severity detection.
const OTEL_SEVERITY_NUMBER_KEYS: &[&str] = &["severity_number", "severitynumber"];

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
    let mut top_level_string_level = None;
    let mut otel_severity_number_level = None;
    let mut ts = chrono::Utc::now();
    let mut ts_parsed = false;

    for (key, value) in &obj {
        let lower_key = key.to_lowercase();

        // Detect level
        if top_level_string_level.is_none()
            && LEVEL_KEYS.contains(&lower_key.as_str())
            && let Some(s) = value.as_str()
        {
            top_level_string_level = LogLevel::parse_level(s);
        }

        if otel_severity_number_level.is_none()
            && OTEL_SEVERITY_NUMBER_KEYS.contains(&lower_key.as_str())
        {
            otel_severity_number_level = parse_otel_severity_number(value);
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

    let mut level = top_level_string_level.or(otel_severity_number_level);
    if level.is_none() {
        level = nested_string(&obj, &["log", "level"]).and_then(LogLevel::parse_level);
    }

    let msg = MESSAGE_KEYS
        .iter()
        .find_map(|k| obj.get(*k)?.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string());

    let raw = (msg != raw).then(|| raw.to_string());

    Some(NewLogEntry {
        msg,
        ts,
        ts_source: if ts_parsed {
            TsSource::Parsed
        } else {
            TsSource::Ingest
        },
        raw,
        level,
        source: source.clone(),
        fields,
    })
}

fn nested_string<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    path: &[&str],
) -> Option<&'a str> {
    let (first, rest) = path.split_first()?;
    let mut value = obj.get(*first)?;

    for key in rest {
        value = value.as_object()?.get(*key)?;
    }

    value.as_str()
}

fn parse_otel_severity_number(value: &serde_json::Value) -> Option<LogLevel> {
    let n = value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))?;

    match n {
        1..=4 => Some(LogLevel::Trace),
        5..=8 => Some(LogLevel::Debug),
        9..=12 => Some(LogLevel::Info),
        13..=16 => Some(LogLevel::Warn),
        17..=20 => Some(LogLevel::Error),
        21..=24 => Some(LogLevel::Fatal),
        _ => None,
    }
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

    #[test]
    fn dotted_log_level_field_sets_level() {
        let entry = try_parse_json(r#"{"log.level":"error","message":"boom"}"#, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Error));
    }

    #[test]
    fn nested_ecs_log_level_sets_level_without_flattening_fields() {
        let entry =
            try_parse_json(r#"{"log":{"level":"warn"},"message":"careful"}"#, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Warn));
        assert_eq!(entry.msg, "careful");
        assert!(entry.fields.contains_key("log"));
        assert!(!entry.fields.contains_key("log.level"));
    }

    #[test]
    fn top_level_level_wins_over_nested_ecs_log_level() {
        let entry = try_parse_json(
            r#"{"level":"error","log":{"level":"debug"},"message":"boom"}"#,
            &source(),
        )
        .unwrap();
        assert_eq!(entry.level, Some(LogLevel::Error));
    }

    #[test]
    fn log4j_logback_level_aliases_set_level() {
        let level_name_entry =
            try_parse_json(r#"{"level_name":"ERROR","message":"boom"}"#, &source()).unwrap();
        let levelname_entry =
            try_parse_json(r#"{"levelname":"WARN","message":"careful"}"#, &source()).unwrap();

        assert_eq!(level_name_entry.level, Some(LogLevel::Error));
        assert_eq!(levelname_entry.level, Some(LogLevel::Warn));
    }

    #[test]
    fn opentelemetry_severity_text_aliases_set_level() {
        let camel_case_entry =
            try_parse_json(r#"{"severityText":"WARN","message":"careful"}"#, &source()).unwrap();
        let snake_case_entry =
            try_parse_json(r#"{"severity_text":"ERROR","message":"boom"}"#, &source()).unwrap();

        assert_eq!(camel_case_entry.level, Some(LogLevel::Warn));
        assert_eq!(snake_case_entry.level, Some(LogLevel::Error));
    }

    #[test]
    fn opentelemetry_severity_number_sets_level() {
        let warn_entry =
            try_parse_json(r#"{"severityNumber":13,"message":"careful"}"#, &source()).unwrap();
        let fatal_entry =
            try_parse_json(r#"{"severity_number":24,"message":"fatal"}"#, &source()).unwrap();

        assert_eq!(warn_entry.level, Some(LogLevel::Warn));
        assert_eq!(fatal_entry.level, Some(LogLevel::Fatal));
    }

    #[test]
    fn opentelemetry_severity_text_wins_over_severity_number() {
        let entry = try_parse_json(
            r#"{"severityNumber":17,"severityText":"WARN","message":"careful"}"#,
            &source(),
        )
        .unwrap();

        assert_eq!(entry.level, Some(LogLevel::Warn));
    }

    #[test]
    fn google_cloud_severity_field_sets_level() {
        let entry = try_parse_json(r#"{"severity":"ERROR","message":"boom"}"#, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Error));
    }

    #[test]
    fn parsed_timestamp_marks_ts_source_parsed() {
        let entry =
            try_parse_json(r#"{"ts":"2024-06-01T12:00:00Z","msg":"hi"}"#, &source()).unwrap();
        assert_eq!(entry.ts_source, TsSource::Parsed);
        assert_eq!(
            entry.ts,
            chrono::DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z").unwrap()
        );
    }

    #[test]
    fn missing_or_unparseable_timestamp_marks_ts_source_ingest() {
        let missing = try_parse_json(r#"{"msg":"hi"}"#, &source()).unwrap();
        assert_eq!(missing.ts_source, TsSource::Ingest);

        let unparseable =
            try_parse_json(r#"{"ts":"yesterday-ish","msg":"hi"}"#, &source()).unwrap();
        assert_eq!(unparseable.ts_source, TsSource::Ingest);
    }

    #[test]
    fn raw_preserved_only_when_msg_differs_from_line() {
        let line = r#"{"msg":"hello"}"#;
        let extracted = try_parse_json(line, &source()).unwrap();
        assert_eq!(extracted.raw.as_deref(), Some(line));

        let fallback_line = r#"{"level":"info"}"#;
        let fallback = try_parse_json(fallback_line, &source()).unwrap();
        assert_eq!(fallback.msg, fallback_line);
        assert_eq!(fallback.raw, None);
    }
}
