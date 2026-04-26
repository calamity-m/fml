use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SearchConfig {
    #[serde(default = "default_tail_size")]
    pub tail_size: usize,

    #[serde(default = "default_tail_poll_interval_ms")]
    pub tail_poll_interval_ms: u64,

    #[serde(default = "default_history_poll_interval_ms")]
    pub history_poll_interval_ms: u64,

    #[serde(default = "default_fuzzy_tick_rate_ms")]
    pub fuzzy_tick_rate_ms: u64,

    #[serde(default = "default_fuzzy_result_limit")]
    pub fuzzy_result_limit: usize,

    #[serde(default = "default_fuzzy_max_typos")]
    pub fuzzy_max_typos: Option<u16>,
}

fn default_tail_size() -> usize {
    500
}

fn default_tail_poll_interval_ms() -> u64 {
    150
}

fn default_history_poll_interval_ms() -> u64 {
    150
}

fn default_fuzzy_tick_rate_ms() -> u64 {
    150
}

fn default_fuzzy_result_limit() -> usize {
    20_000
}

fn default_fuzzy_max_typos() -> Option<u16> {
    None
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tail_size: default_tail_size(),
            tail_poll_interval_ms: default_tail_poll_interval_ms(),
            history_poll_interval_ms: default_history_poll_interval_ms(),
            fuzzy_tick_rate_ms: default_fuzzy_tick_rate_ms(),
            fuzzy_result_limit: default_fuzzy_result_limit(),
            fuzzy_max_typos: default_fuzzy_max_typos(),
        }
    }
}
