use tokio::sync::mpsc;
use tracing::debug;

use crate::{
    event::{Query, SearchEvent},
    log::SourceId,
    state::{AppState, events_bus},
};

pub fn handle_search_event(event: SearchEvent, state: AppState) -> AppState {
    let new_state = match event {
        SearchEvent::Search { query, filter } => {
            debug!(
                "received search query event - query: {:?}, filter: {:?}",
                query, filter
            );

            // TODO figure out a way to pass a handler from app state to the search functions, so that they
            // cancel each other out - only the latest search keeps going, issuing a new search event ends the
            // old one.
            //
            // stop_current_search(...)
            //
            // We probably want to modify the start_*_search functions to return a tokio join/task
            // handle or something, which we can use to stop the async tokio task with. We can just
            // store it into app state, under a new SearchState or something.

            match query {
                Query::Tail => start_tail_search(filter, state.event_bus.search_event_tx.clone()),
                Query::History {
                    middle_seq_id,
                    buffer,
                } => start_history_search(
                    middle_seq_id,
                    buffer,
                    filter,
                    state.event_bus.search_event_tx.clone(),
                ),
                Query::Fuzzy(term) => {
                    start_fuzzy_search(term, filter, state.event_bus.search_event_tx.clone())
                }
            }

            state
        }
        SearchEvent::Result { results } => state,
        SearchEvent::Error(_) => state,
    };

    new_state
}

fn start_tail_search(sources: Vec<SourceId>, tx: mpsc::Sender<SearchEvent>) {}

fn start_history_search(
    middle_seq_id: u64,
    buffer: u64,
    sources: Vec<SourceId>,
    tx: mpsc::Sender<SearchEvent>,
) {
}

fn start_fuzzy_search(term: String, sources: Vec<SourceId>, tx: mpsc::Sender<SearchEvent>) {}
