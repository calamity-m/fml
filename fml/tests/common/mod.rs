#![cfg(feature = "integration")]
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::Utc;
use ratatui::buffer::Buffer;

use fml::log::{LogLevel, NewLogEntry, Source};

pub fn make_entry(msg: &str, source_id: &str) -> NewLogEntry {
    make_entry_with_source_display(msg, source_id, source_id)
}

pub fn make_entry_with_source_display(
    msg: &str,
    source_id: &str,
    display_name: &str,
) -> NewLogEntry {
    NewLogEntry {
        msg: msg.to_string(),
        ts: Utc::now(),
        level: Some(LogLevel::Info),
        source: Source {
            id: source_id.to_string(),
            display_name: display_name.to_string(),
            group: None,
        },
        fields: HashMap::new(),
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
