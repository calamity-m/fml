use std::collections::HashMap;

use tracing::trace;

use crate::log::{NewLogEntry, Source};

mod json;
mod logfmt;
mod pattern;

#[derive(Clone, Copy, Default)]
pub struct Normalizer;

impl Normalizer {
    pub fn new() -> Self {
        Normalizer
    }

    pub fn normalize(&self, raw: &str, source: Source) -> NewLogEntry {
        let parsed = json::try_parse_json(raw, &source)
            .or_else(|| logfmt::try_parse_logfmt(raw, &source))
            .or_else(|| pattern::try_parse_patterns(raw, &source))
            .unwrap_or_else(|| NewLogEntry {
                msg: raw.to_string(),
                ts: chrono::Utc::now(),
                level: None,
                source,
                fields: HashMap::new(),
            });

        trace!("normalized log entry - {:?}", parsed);

        parsed
    }
}
