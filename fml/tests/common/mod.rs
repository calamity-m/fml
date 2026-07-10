#![cfg(feature = "integration")]
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use ratatui::buffer::Buffer;

use fml::log::{LogLevel, NewLogEntry, Source, TsSource};

pub fn make_entry(msg: &str, source_id: &str) -> NewLogEntry {
    make_entry_with_source_display(msg, source_id, source_id)
}

pub fn make_entry_with_source_display(
    msg: &str,
    source_id: &str,
    display_name: &str,
) -> NewLogEntry {
    make_entry_with_source_display_and_fields(msg, source_id, display_name, HashMap::new())
}

pub fn make_entry_with_fields(
    msg: &str,
    source_id: &str,
    fields: HashMap<String, serde_json::Value>,
) -> NewLogEntry {
    make_entry_with_source_display_and_fields(msg, source_id, source_id, fields)
}

pub fn make_entry_with_source_display_and_fields(
    msg: &str,
    source_id: &str,
    display_name: &str,
    fields: HashMap<String, serde_json::Value>,
) -> NewLogEntry {
    NewLogEntry {
        msg: msg.to_string(),
        ts: Utc
            .with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
            .single()
            .expect("fixed timestamp"),
        ts_source: TsSource::Ingest,
        raw: None,
        level: Some(LogLevel::Info),
        source: Source {
            producer: "fake".to_string(),
            id: source_id.to_string(),
            display_name: display_name.to_string(),
            group: None,
        },
        fields,
    }
}

/// Render the buffer as a multi-line string of cell symbols. Strips styles —
/// snapshots assert on visible glyphs only, which is what the README mockup
/// describes anyway.
pub fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area();
    let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
