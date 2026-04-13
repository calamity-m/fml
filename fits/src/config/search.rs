use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SearchConfig {}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {}
    }
}
