use std::collections::HashMap;

use crate::event::SearchTarget;

#[derive(Default)]
pub struct SearchClientState {
    pub running_handle: Option<tokio::task::JoinHandle<()>>,
    pub latest_request_id: u64,
}

pub struct SearchState {
    clients: HashMap<SearchTarget, SearchClientState>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn client_mut(&mut self, target: SearchTarget) -> &mut SearchClientState {
        self.clients.entry(target).or_default()
    }

    pub fn latest_request_id(&self, target: SearchTarget) -> u64 {
        self.clients
            .get(&target)
            .map(|client| client.latest_request_id)
            .unwrap_or_default()
    }

    pub fn cancel(&mut self, target: SearchTarget) {
        if let Some(client) = self.clients.get_mut(&target)
            && let Some(handle) = client.running_handle.take()
        {
            handle.abort();
        }
    }
}
