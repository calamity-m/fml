use crate::{config::search::SearchConfig, error::FmlError};

pub struct SearchState {
    pub running_handle: Option<tokio::task::JoinHandle<()>>,
    pub latest_request_id: u64,
}

impl SearchState {
    pub fn new(config: &SearchConfig) -> Result<Self, FmlError> {
        Ok(Self {
            running_handle: None,
            latest_request_id: 0,
        })
    }
}
