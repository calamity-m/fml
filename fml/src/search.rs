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
    event::{Query, SearchEvent, SearchHit, TuiEvent},
    log::LogEntry,
    state::{
        AppState,
        tui_state::log_pane_state::{LogPaneUpdate, SearchKind},
    },
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
    complete: bool,
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
            complete,
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

/// Sends pre-built `SearchHit`s as a single `SearchEvent::Result`. Used by
/// workers (fuzzy) that populate per-field `Match` data; the entry-only
/// `emit_results` helper hard-codes an empty matches vec.
pub(crate) async fn emit_hits(
    hits: Vec<SearchHit>,
    request_id: u64,
    complete: bool,
    tx: &mpsc::Sender<SearchEvent>,
) -> EmitOutcome {
    match tx
        .send(SearchEvent::Result {
            results: hits,
            request_id,
            complete,
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

pub(crate) async fn emit_error(message: String, tx: &mpsc::Sender<SearchEvent>) -> EmitOutcome {
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
    match event {
        SearchEvent::Search { query, sources } => {
            debug!(
                "received search query event - query: {:?}, sources: {:?}",
                query, sources
            );

            if let Some(handle) = &state.search.running_handle {
                handle.abort();
            }

            let request_id = &state.search.latest_request_id + 1;
            state.tui.log_pane.on_search_started(&query);

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
                    Duration::from_millis(state.config.search.history_poll_interval_ms),
                    state.store.clone(),
                    request_id,
                    state.event_bus.search_event_tx.clone(),
                ),
                Query::Fuzzy(term) => fuzzy::start_fuzzy_search(
                    term,
                    sources,
                    fuzzy::FuzzySearchOptions {
                        result_limit: state.config.search.fuzzy_result_limit,
                        tick_rate: Duration::from_millis(state.config.search.fuzzy_tick_rate_ms),
                        matcher_kind: state.config.search.fuzzy_matcher,
                        max_typos: state.config.search.fuzzy_max_typos,
                    },
                    state.store.clone(),
                    request_id,
                    state.event_bus.search_event_tx.clone(),
                ),
            };

            state.search.running_handle = Some(new_handle);
            state.search.latest_request_id = request_id;
            state
        }
        SearchEvent::Result {
            results,
            request_id,
            complete,
        } => {
            debug!(
                "received search result - request_id: {}, complete: {}, results: {:?}",
                request_id, complete, results
            );

            // Ignore stale request_id responses
            if request_id != state.search.latest_request_id {
                warn!(
                    "received result for stale request - expected request_id {}, but got result for request_id {}",
                    state.search.latest_request_id, request_id
                );
                return state;
            }

            let kind = state.tui.log_pane.active_query;
            let retained_bounds = state.store.bounds();
            let matches_by_seq = (kind == SearchKind::Fuzzy).then(|| {
                results
                    .iter()
                    .map(|hit| (hit.seq_id, hit.matches.clone()))
                    .collect()
            });
            let seq_ids: Vec<u64> = results.into_iter().map(|hit| hit.seq_id).collect();
            let mut entries: Vec<Arc<LogEntry>> = Vec::with_capacity(seq_ids.len());
            if let Err(err) = state.store.fetch_requested(&seq_ids, &mut entries) {
                warn!("failed to fetch search result entries from store: {err}");
            }
            let update = match kind {
                SearchKind::Tail => LogPaneUpdate::Tail {
                    entries,
                    retained_bounds,
                },
                SearchKind::History => LogPaneUpdate::History {
                    entries,
                    retained_bounds,
                },
                SearchKind::Fuzzy => LogPaneUpdate::Fuzzy {
                    best_first_entries: entries,
                    retained_bounds,
                    matches_by_seq: matches_by_seq.unwrap_or_default(),
                },
            };
            state
                .tui
                .log_pane
                .apply_update(update, &mut state.tui.log_pane_cursor_row);
            if let Err(err) = state
                .event_bus
                .tui_event_tx
                .send(TuiEvent::NewSelectedEntry(
                    state.tui.log_pane.selected_entry(),
                ))
            {
                warn!("failed to send selected entry event after search result: {err}");
            }

            state
        }

        SearchEvent::Error(_) => state,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::*;
    use crate::{
        config::Config,
        event::{Match, ProducerEvent},
        log::{LogLevel, NewLogEntry, Source},
        producer,
        state::AppState,
    };

    fn state_with_entries(count: u64) -> AppState {
        let mut state = AppState::new(Config::default()).expect("app state");
        for seq in 1..=count {
            state = producer::handle_producer_event(
                ProducerEvent::StoreEvent(NewLogEntry {
                    msg: format!("entry {seq}"),
                    ts: Utc::now(),
                    level: Some(LogLevel::Info),
                    source: Source {
                        producer: "fake".to_string(),
                        id: "src-a".to_string(),
                        display_name: "src-a".to_string(),
                        group: None,
                    },
                    fields: HashMap::new(),
                }),
                state,
            );
        }
        state.search.latest_request_id = 1;
        state
    }

    fn take_selected_entry_event(state: &mut AppState) -> Option<crate::event::SelectedEntry> {
        match state
            .event_bus
            .tui_event_rx
            .try_recv()
            .expect("selected entry event")
        {
            TuiEvent::NewSelectedEntry(selected_entry) => selected_entry,
            event => panic!("expected selected entry event, got {event:?}"),
        }
    }

    #[test]
    fn tail_result_emits_selected_entry_event() {
        let state = state_with_entries(3);
        let mut state = handle_search_event(
            SearchEvent::Result {
                results: vec![1, 2, 3]
                    .into_iter()
                    .map(|seq_id| SearchHit {
                        seq_id,
                        matches: Vec::new(),
                    })
                    .collect(),
                request_id: 1,
                complete: true,
            },
            state,
        );

        let selected = take_selected_entry_event(&mut state).expect("selected entry");

        assert_eq!(selected.entry.seq, 3);
        assert!(selected.matches.is_empty());
    }

    #[test]
    fn fuzzy_result_emits_selected_entry_with_matches() {
        let mut state = state_with_entries(2);
        state
            .tui
            .log_pane
            .on_search_started(&Query::Fuzzy("entry".to_string()));

        let mut state = handle_search_event(
            SearchEvent::Result {
                results: vec![SearchHit {
                    seq_id: 2,
                    matches: vec![Match {
                        key: "msg".to_string(),
                        indices: vec![0, 1],
                    }],
                }],
                request_id: 1,
                complete: true,
            },
            state,
        );

        let selected = take_selected_entry_event(&mut state).expect("selected entry");

        assert_eq!(selected.entry.seq, 2);
        assert_eq!(selected.matches[0].key, "msg");
    }

    #[test]
    fn empty_result_emits_clear_selected_entry_event() {
        let state = state_with_entries(2);
        let mut state = handle_search_event(
            SearchEvent::Result {
                results: Vec::new(),
                request_id: 1,
                complete: true,
            },
            state,
        );

        assert!(take_selected_entry_event(&mut state).is_none());
    }
}
