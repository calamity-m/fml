use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StoreConfig {
    #[serde(default = "default_capacity")]
    pub capacity: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            capacity: default_capacity(),
        }
    }
}

fn default_capacity() -> usize {
    1_000_000
}
