use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SearchConfig {
    #[serde(default = "default_tail_size")]
    pub tail_size: usize,

    #[serde(default = "default_tail_poll_interval_ms")]
    pub tail_poll_interval_ms: u64,
}

fn default_tail_size() -> usize {
    500
}

fn default_tail_poll_interval_ms() -> u64 {
    150
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tail_size: default_tail_size(),
            tail_poll_interval_ms: default_tail_poll_interval_ms(),
        }
    }
}
