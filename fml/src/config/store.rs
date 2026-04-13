use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StoreConfig {
    #[serde(default = "default_capacity")]
    pub capacity: usize,
    #[serde(default = "default_writer_log_interval")]
    pub writer_log_internal: u64,
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            capacity: default_capacity(),
            writer_log_internal: default_writer_log_interval(),
            channel_capacity: default_channel_capacity(),
        }
    }
}

fn default_capacity() -> usize {
    1_000_000
}

fn default_writer_log_interval() -> u64 {
    10
}

fn default_channel_capacity() -> usize {
    8_192
}
