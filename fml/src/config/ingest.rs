//! Startup backfill policy shared by the real (file/docker/kubernetes)
//! producers. The demo producer is live-only and ignores these settings.

use serde::{Deserialize, Serialize};

/// Bounded startup-history policy applied when a real producer first tracks
/// a live source during this process.
///
/// `Copy` on purpose: producers receive this by value at construction so they
/// stay decoupled from the full [`Config`](crate::config::Config).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct IngestConfig {
    /// Provider-side time window (seconds) for backends with server-side time
    /// filtering (docker `since`, kubernetes `since_seconds`). Plain files
    /// have no trustworthy per-line timestamps, so this window is not applied
    /// to file producers — only the line cap is.
    #[serde(default = "default_backfill_window_secs")]
    pub backfill_window_secs: u64,

    /// Hard safety cap on backfilled lines per source. `0` disables startup
    /// backfill for all producers while leaving live following intact.
    #[serde(default = "default_backfill_max_lines_per_source")]
    pub backfill_max_lines_per_source: usize,
}

impl IngestConfig {
    /// Whether startup backfill is enabled (`backfill_max_lines_per_source > 0`).
    pub fn backfill_enabled(&self) -> bool {
        self.backfill_max_lines_per_source > 0
    }
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            backfill_window_secs: default_backfill_window_secs(),
            backfill_max_lines_per_source: default_backfill_max_lines_per_source(),
        }
    }
}

fn default_backfill_window_secs() -> u64 {
    1800
}

fn default_backfill_max_lines_per_source() -> usize {
    5000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_thirty_minutes_and_five_thousand_lines() {
        let config = IngestConfig::default();

        assert_eq!(config.backfill_window_secs, 1800);
        assert_eq!(config.backfill_max_lines_per_source, 5000);
        assert!(config.backfill_enabled());
    }

    #[test]
    fn zero_line_cap_disables_backfill() {
        let config = IngestConfig {
            backfill_max_lines_per_source: 0,
            ..IngestConfig::default()
        };

        assert!(!config.backfill_enabled());
    }
}
