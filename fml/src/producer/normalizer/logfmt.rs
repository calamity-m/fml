//! Logfmt log line parser.
//!
//! Parses lines in the logfmt format: space-separated `key=value` pairs,
//! with double-quoted values for strings containing spaces.

use std::collections::HashMap;

use crate::log::{LogLevel, NewLogEntry, Source};

const LEVEL_KEYS: &[&str] = &["level", "severity", "lvl", "log.level"];
const TIMESTAMP_KEYS: &[&str] = &["ts", "timestamp", "time", "@timestamp", "t"];
const MESSAGE_KEYS: &[&str] = &["msg", "message"];

/// Try to parse `raw` as a logfmt line. Returns `None` if no `key=value`
/// pairs (with an explicit `=`) are found.
pub fn try_parse_logfmt(raw: &str, source: &Source) -> Option<NewLogEntry> {
    let pairs = parse_pairs(raw)?;

    let mut fields = HashMap::new();
    let mut level = None;
    let mut ts = chrono::Utc::now();
    let mut ts_parsed = false;
    let mut msg_value: Option<String> = None;

    for (key, value) in &pairs {
        let lower_key = key.to_lowercase();

        match value {
            None => {
                // Bare level word (e.g. "INFO" at the start of the line)
                if level.is_none() {
                    level = LogLevel::parse_level(key);
                }
                fields.insert(lower_key, serde_json::Value::Bool(true));
            }
            Some(v) => {
                if level.is_none() && LEVEL_KEYS.contains(&lower_key.as_str()) {
                    level = LogLevel::parse_level(v);
                }

                if !ts_parsed && TIMESTAMP_KEYS.contains(&lower_key.as_str()) {
                    if let Some(parsed) = super::json::try_parse_timestamp(v) {
                        ts = parsed;
                        ts_parsed = true;
                    }
                }

                if msg_value.is_none() && MESSAGE_KEYS.contains(&lower_key.as_str()) {
                    msg_value = Some(v.clone());
                }

                fields.insert(lower_key, serde_json::Value::String(v.clone()));
            }
        }
    }

    let msg = msg_value.unwrap_or_else(|| raw.to_string());

    Some(NewLogEntry {
        msg,
        ts,
        level,
        source: source.clone(),
        fields,
    })
}

/// Parse logfmt key/value pairs from `input`.
///
/// Returns `None` if no pairs with an explicit `=` were found (line is not
/// logfmt). Bare keys (no `=`) have a `None` value; `key=` has `Some("")`.
fn parse_pairs(mut input: &str) -> Option<Vec<(String, Option<String>)>> {
    let mut pairs: Vec<(String, Option<String>)> = Vec::new();

    loop {
        input = input.trim_start();
        if input.is_empty() {
            break;
        }

        // Read key: everything up to '=' or whitespace
        let key_end = input
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(input.len());
        let key = &input[..key_end];
        if key.is_empty() {
            break;
        }
        input = &input[key_end..];

        if input.starts_with('=') {
            input = &input[1..]; // consume '='

            if input.starts_with('"') {
                input = &input[1..]; // consume opening '"'
                let mut value = String::new();
                loop {
                    match input.find(|c: char| c == '\\' || c == '"') {
                        Some(pos) => {
                            value.push_str(&input[..pos]);
                            if input[pos..].starts_with("\\\"") {
                                value.push('"');
                                input = &input[pos + 2..];
                            } else if input[pos..].starts_with('\\') {
                                value.push('\\');
                                input = &input[pos + 1..];
                            } else {
                                // unescaped '"' → end of value
                                input = &input[pos + 1..];
                                break;
                            }
                        }
                        None => {
                            // unterminated quote: consume remainder
                            value.push_str(input);
                            input = "";
                            break;
                        }
                    }
                }
                pairs.push((key.to_string(), Some(value)));
            } else {
                // unquoted value: up to next whitespace
                let val_end = input
                    .find(|c: char| c.is_ascii_whitespace())
                    .unwrap_or(input.len());
                let value = input[..val_end].to_string();
                input = &input[val_end..];
                pairs.push((key.to_string(), Some(value)));
            }
        } else {
            // bare key (no '=')
            pairs.push((key.to_string(), None));
        }
    }

    // Require at least one key=value pair; bare-key-only lines are not logfmt
    if pairs.iter().any(|(_, v)| v.is_some()) {
        Some(pairs)
    } else {
        None
    }
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
    fn typical_line_parsed() {
        let raw =
            "level=info msg=\"starting server\" host=0.0.0.0 port=8080 ts=2024-01-01T00:00:00Z";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.msg, "starting server");
        assert_eq!(entry.level, Some(LogLevel::Info));
        assert_eq!(
            entry.fields.get("host"),
            Some(&serde_json::Value::String("0.0.0.0".into()))
        );
        assert_eq!(
            entry.fields.get("port"),
            Some(&serde_json::Value::String("8080".into()))
        );
    }

    #[test]
    fn quoted_value_with_spaces() {
        let raw = r#"msg="hello world" level=debug"#;
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.msg, "hello world");
    }

    #[test]
    fn escaped_quote_in_quoted_value() {
        let raw = r#"msg="say \"hi\"" level=info"#;
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.msg, r#"say "hi""#);
    }

    #[test]
    fn bare_key_stored_as_bool_true() {
        let raw = "level=info debug";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(
            entry.fields.get("debug"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn empty_value_stored_as_empty_string() {
        let raw = "key= level=info";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(
            entry.fields.get("key"),
            Some(&serde_json::Value::String("".into()))
        );
    }

    #[test]
    fn no_equals_sign_returns_none() {
        assert!(try_parse_logfmt("just a plain message", &source()).is_none());
    }

    #[test]
    fn only_bare_keys_returns_none() {
        assert!(try_parse_logfmt("foo bar baz", &source()).is_none());
    }

    #[test]
    fn timestamp_parsed() {
        let raw = "ts=2024-06-01T12:00:00Z level=info msg=ok";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(
            entry.ts,
            chrono::DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        );
    }

    #[test]
    fn level_warn_parsed() {
        let raw = "level=warn msg=careful";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Warn));
    }

    #[test]
    fn falls_back_to_raw_when_no_msg_key() {
        let raw = "level=error code=500";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.msg, raw);
    }

    #[test]
    fn message_key_used_as_msg() {
        let raw = r#"message="alternative key" level=info"#;
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.msg, "alternative key");
    }

    #[test]
    fn keys_are_lowercased() {
        let raw = "Level=INFO Msg=hello";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert!(entry.fields.contains_key("level"));
        assert!(entry.fields.contains_key("msg"));
        assert_eq!(entry.level, Some(LogLevel::Info));
        assert_eq!(entry.msg, "hello");
    }

    #[test]
    fn bare_level_prefix_detected() {
        let raw = "WARN cache stale key=session-0 age_ms=9000";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Warn));
    }

    #[test]
    fn bare_error_level_detected() {
        let raw = "ERROR upstream timeout peer=payments timeout_ms=2500";
        let entry = try_parse_logfmt(raw, &source()).unwrap();
        assert_eq!(entry.level, Some(LogLevel::Error));
    }
}
