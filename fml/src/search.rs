//! Search event handling and background task orchestration.
//!
//! This module is responsible for coordinating the application's search
//! subsystem. It accepts [`SearchEvent`] messages from the main event loop,
//! dispatches the appropriate search strategy for each [`Query`], and routes
//! asynchronous search results back into [`AppState`].
//!
//! Searches are intended to behave as latest-wins work: issuing a new request
//! should cancel any superseded in-flight search, and result messages should be
//! correlated with the active request so outdated responses do not overwrite
//! newer state.

use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{
    event::{Query, SearchEvent, SearchHit},
    log::LogEntry,
    state::AppState,
};

pub mod fuzzy;
pub mod history;
pub mod tail;

pub(crate) enum EmitOutcome {
    Sent,
    ReceiverGone,
}

/// Maps already-selected log entries into `SearchHit`s and sends them as a
/// single `SearchEvent::Result`. Workers own their own selection logic
/// (tail window, history buffer, fuzzy matches) and delegate emission here.
pub(crate) async fn emit_results(
    entries: Vec<Arc<LogEntry>>,
    request_id: u64,
    tx: &mpsc::Sender<SearchEvent>,
) -> EmitOutcome {
    let results: Vec<SearchHit> = entries
        .into_iter()
        .map(|entry| SearchHit {
            seq_id: entry.seq,
            matches: Vec::new(),
        })
        .collect();

    match tx
        .send(SearchEvent::Result {
            results,
            request_id,
        })
        .await
    {
        Ok(()) => EmitOutcome::Sent,
        Err(e) => {
            warn!("search worker failed to deliver result: {e}");
            EmitOutcome::ReceiverGone
        }
    }
}

pub(crate) async fn emit_error(
    message: String,
    tx: &mpsc::Sender<SearchEvent>,
) -> EmitOutcome {
    match tx.send(SearchEvent::Error(message)).await {
        Ok(()) => EmitOutcome::Sent,
        Err(_) => {
            warn!("search worker failed to deliver error: receiver gone");
            EmitOutcome::ReceiverGone
        }
    }
}

/// Applies a single [`SearchEvent`] to the application state.
///
/// This is the search subsystem's main reducer entry point. It is intended to:
///
/// - start or replace in-flight work when a new [`SearchEvent::Search`] request
///   arrives,
/// - accept [`SearchEvent::Result`] messages produced by background workers and
///   merge them into the active search state, and
/// - handle [`SearchEvent::Error`] messages without destabilizing the rest of
///   the application loop.
///
/// Search requests are expected to be request-scoped so that results can be
/// matched to the currently active query and stale responses can be discarded.
pub fn handle_search_event(event: SearchEvent, mut state: AppState) -> AppState {
    let new_state = match event {
        SearchEvent::Search { query, sources } => {
            debug!(
                "received search query event - query: {:?}, sources: {:?}",
                query, sources
            );

            if let Some(handle) = &state.search.running_handle {
                handle.abort();
            }

            let request_id = &state.search.latest_request_id + 1;

            let new_handle = match query {
                Query::Tail => tail::start_tail_search(
                    sources,
                    state.config.search.tail_size,
                    Duration::from_millis(state.config.search.tail_poll_interval_ms),
                    state.store.clone(),
                    request_id,
                    state.event_bus.search_event_tx.clone(),
                ),
                Query::History {
                    middle_seq_id,
                    buffer,
                } => history::start_history_search(
                    middle_seq_id,
                    buffer,
                    sources,
                    request_id,
                    state.event_bus.search_event_tx.clone(),
                ),
                Query::Fuzzy(term) => fuzzy::start_fuzzy_search(
                    term,
                    sources,
                    request_id,
                    state.event_bus.search_event_tx.clone(),
                ),
            };

            state.search.running_handle = Some(new_handle);
            state
        }
        SearchEvent::Result {
            results,
            request_id,
        } => {
            debug!(
                "received search result - request_id: {}, results: {:?}",
                request_id, results
            );

            // Ignore stale request_id responses
            if request_id != state.search.latest_request_id {
                warn!(
                    "received result for stale request - expected request_id {}, but got result for request_id {}",
                    state.search.latest_request_id, request_id
                );
                return state;
            }

            state
        }

        SearchEvent::Error(_) => state,
    };

    new_state
}
