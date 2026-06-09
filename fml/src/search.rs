//! Search event handling and background task orchestration.
//!
//! This module is responsible for coordinating the application's search
//! subsystem. It accepts [`SearchEvent`] messages from the main event loop,
//! dispatches the appropriate search strategy for each [`Query`], and routes
//! asynchronous search results back into [`AppState`].
//!
//! Searches are latest-wins per target: issuing a new request for one target
//! cancels only that target's superseded in-flight work, and result messages
//! are correlated with the active request for the same target. A target is a
//! pane id; results addressed to a pane that no longer exists are dropped
//! and the orphaned worker is cancelled.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{
    event::{Match, Query, SearchEvent, SearchHit, SearchProgress, SearchTarget},
    log::{LogEntry, SourceId},
    state::AppState,
    store::LogStore,
};

pub mod field_matched;
pub mod fuzzy;
pub mod history;
pub mod tail;

pub(crate) enum EmitOutcome {
    Sent,
    ReceiverGone,
}

/// Shared plumbing every search worker needs.
///
/// Bundles the routing/identity bits (`target`, `request_id`, `query`) that
/// tag every emission, the per-call filter/cadence (`sources`, `tick_rate`),
/// and the I/O handles (`store`, `tx`). Each `start_*_search` worker takes
/// this as its first argument so strategy-specific parameters stay short and
/// every worker shares the same uniform call shape.
///
/// `tick_rate` is the worker's wake-up cadence: in tail/history it gates how
/// often retained-bounds are re-checked; in fuzzy it doubles as the partial-
/// emission cadence during an in-flight scan.
pub struct SearchContext {
    pub target: SearchTarget,
    pub query: Query,
    pub sources: Vec<SourceId>,
    pub request_id: u64,
    pub tick_rate: Duration,
    pub store: Arc<dyn LogStore>,
    pub tx: mpsc::Sender<SearchEvent>,
}

/// Maps already-selected log entries into `SearchHit`s and sends them as a
/// single `SearchEvent::Result`. Workers own their own selection logic
/// (tail window, history buffer, fuzzy matches) and delegate emission here.
pub(crate) async fn emit_results(
    target: SearchTarget,
    query: Query,
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
            target,
            query,
            results,
            hit_seqs: None,
            request_id,
            complete,
            progress: None,
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
/// `emit_results` helper hard-codes an empty matches vec. `hit_seqs`
/// carries the full uncapped match list when `hits` is display-capped.
pub(crate) async fn emit_hits(
    target: SearchTarget,
    query: Query,
    hits: Vec<SearchHit>,
    hit_seqs: Option<Vec<u64>>,
    request_id: u64,
    complete: bool,
    progress: Option<SearchProgress>,
    tx: &mpsc::Sender<SearchEvent>,
) -> EmitOutcome {
    match tx
        .send(SearchEvent::Result {
            target,
            query,
            results: hits,
            hit_seqs,
            request_id,
            complete,
            progress,
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
/// - start or replace in-flight work for a target when a new
///   [`SearchEvent::Search`] request arrives,
/// - accept [`SearchEvent::Result`] messages produced by background workers,
///   fetch the referenced entries, and route them to the owning pane, and
/// - handle [`SearchEvent::Error`] messages without destabilizing the rest of
///   the application loop.
///
/// Search requests are target/request scoped so stale responses can be
/// discarded without one pane cancelling another pane's work.
pub fn handle_search_event(event: SearchEvent, mut state: AppState) -> AppState {
    match event {
        SearchEvent::Search {
            target,
            query,
            sources,
        } => {
            debug!(
                "received search query event - target: {:?}, query: {:?}, sources: {:?}",
                target, query, sources
            );

            {
                let client = state.search.client_mut(target);
                if let Some(handle) = client.running_handle.take() {
                    handle.abort();
                }
            }

            let request_id = state.search.latest_request_id(target) + 1;

            let tick_rate = match &query {
                Query::Tail => Duration::from_millis(state.config.search.tail_poll_interval_ms),
                Query::History { .. } | Query::Surrounding { .. } | Query::FieldMatched { .. } => {
                    Duration::from_millis(state.config.search.history_poll_interval_ms)
                }
                Query::Fuzzy(_) => Duration::from_millis(state.config.search.fuzzy_tick_rate_ms),
            };
            let ctx = SearchContext {
                target,
                query: query.clone(),
                sources,
                request_id,
                tick_rate,
                store: state.store.clone(),
                tx: state.event_bus.search_event_tx.clone(),
            };

            let new_handle = match &query {
                Query::Tail => tail::start_tail_search(ctx, state.config.search.tail_size),
                Query::History {
                    middle_seq_id,
                    buffer,
                }
                | Query::Surrounding {
                    middle_seq_id,
                    buffer,
                } => history::start_history_search(ctx, *middle_seq_id, *buffer),
                Query::FieldMatched {
                    anchor_seq_id,
                    buffer,
                    predicates,
                } => field_matched::start_field_matched_search(
                    ctx,
                    *anchor_seq_id,
                    *buffer,
                    predicates.clone(),
                ),
                Query::Fuzzy(_) => fuzzy::start_fuzzy_search(
                    ctx,
                    fuzzy::FuzzySearchOptions {
                        result_limit: state.config.search.fuzzy_result_limit,
                        matcher_kind: state.config.search.fuzzy_matcher,
                        max_typos: state.config.search.fuzzy_max_typos,
                    },
                ),
            };

            let client = state.search.client_mut(target);
            client.running_handle = Some(new_handle);
            client.latest_request_id = request_id;
            state
        }
        SearchEvent::Cancel { target } => {
            state.search.cancel(target);
            state
        }
        SearchEvent::Result {
            target,
            query,
            results,
            hit_seqs,
            request_id,
            complete,
            progress,
        } => {
            debug!(
                "received search result - target: {:?}, query: {:?}, request_id: {}, complete: {}, progress: {:?}, results: {}",
                target,
                query,
                request_id,
                complete,
                progress,
                results.len()
            );

            // Ignore stale request_id responses
            let latest_request_id = state.search.latest_request_id(target);
            if request_id != latest_request_id {
                warn!(
                    "received result for stale request - expected request_id {}, but got result for request_id {}",
                    latest_request_id, request_id
                );
                return state;
            }

            let matches_by_seq: HashMap<u64, Vec<Match>> = results
                .iter()
                .filter(|hit| !hit.matches.is_empty())
                .map(|hit| (hit.seq_id, hit.matches.clone()))
                .collect();
            let seq_ids: Vec<u64> = results.into_iter().map(|hit| hit.seq_id).collect();
            let mut entries: Vec<Arc<LogEntry>> = Vec::with_capacity(seq_ids.len());
            if let Err(err) = state.store.fetch_requested(&seq_ids, &mut entries) {
                warn!("failed to fetch search result entries from store: {err}");
            }
            let retained_bounds = state.store.bounds();

            let Some(pane) = state.workspace.pane_mut(target) else {
                // The pane was closed while this worker was in flight.
                debug!("dropping result for closed pane {target}");
                state.search.cancel(target);
                return state;
            };
            pane.apply_result(
                &query,
                entries,
                matches_by_seq,
                hit_seqs,
                progress,
                retained_bounds,
            );
            state
        }

        SearchEvent::Error(_) => state,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::{
        config::Config,
        event::{FieldPredicate, Match, PaneId, ProducerEvent},
        log::{LogLevel, NewLogEntry, Source},
        producer,
        tui::pane::View,
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
        state
    }

    fn focused_pane_id(state: &AppState) -> PaneId {
        state.workspace.tab().focused
    }

    fn hit(seq_id: u64) -> SearchHit {
        SearchHit {
            seq_id,
            matches: Vec::new(),
        }
    }

    fn result_event(
        target: PaneId,
        query: Query,
        results: Vec<SearchHit>,
        request_id: u64,
    ) -> SearchEvent {
        SearchEvent::Result {
            target,
            query,
            results,
            hit_seqs: None,
            request_id,
            complete: true,
            progress: None,
        }
    }

    #[test]
    fn tail_result_routes_to_pane_and_pins_cursor() {
        let mut state = state_with_entries(3);
        let pane_id = focused_pane_id(&state);
        state.search.client_mut(pane_id).latest_request_id = 1;
        state.workspace.focused_pane_mut().active_query = Some(Query::Tail);

        let state = handle_search_event(
            result_event(pane_id, Query::Tail, vec![hit(1), hit(2), hit(3)], 1),
            state,
        );

        let pane = state.workspace.focused_pane();
        assert_eq!(pane.cursor_seq, Some(3));
        assert_eq!(pane.view.entries().len(), 3);
    }

    #[test]
    fn stale_request_id_is_dropped() {
        let mut state = state_with_entries(3);
        let pane_id = focused_pane_id(&state);
        state.search.client_mut(pane_id).latest_request_id = 2;
        state.workspace.focused_pane_mut().active_query = Some(Query::Tail);

        let state = handle_search_event(result_event(pane_id, Query::Tail, vec![hit(1)], 1), state);

        assert!(state.workspace.focused_pane().view.entries().is_empty());
    }

    #[test]
    fn result_for_closed_pane_is_dropped_and_engine_cancelled() {
        let mut state = state_with_entries(3);
        let ghost = PaneId(999);
        state.search.client_mut(ghost).latest_request_id = 1;

        let state = handle_search_event(result_event(ghost, Query::Tail, vec![hit(1)], 1), state);

        // Nothing routed anywhere; the focused pane is untouched.
        assert!(state.workspace.focused_pane().view.entries().is_empty());
    }

    #[test]
    fn fuzzy_result_carries_match_spans_to_pane() {
        let mut state = state_with_entries(2);
        let pane_id = focused_pane_id(&state);
        state.search.client_mut(pane_id).latest_request_id = 1;
        let query = Query::Fuzzy("entry".to_string());
        state.workspace.focused_pane_mut().active_query = Some(query.clone());

        let state = handle_search_event(
            result_event(
                pane_id,
                query,
                vec![SearchHit {
                    seq_id: 2,
                    matches: vec![Match {
                        key: "msg".to_string(),
                        indices: vec![0, 1],
                    }],
                }],
                1,
            ),
            state,
        );

        let pane = state.workspace.focused_pane();
        match &pane.view {
            View::Results {
                entries, matches, ..
            } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(matches.get(&2).map(|m| m[0].key.as_str()), Some("msg"));
            }
            View::Stream { .. } => panic!("expected results view"),
        }
        assert_eq!(pane.cursor_seq, Some(2));
    }

    #[tokio::test]
    async fn field_matched_search_event_routes_to_worker() {
        let mut state = AppState::new(Config::default()).expect("app state");
        for (seq, source_id, fields) in [
            (
                1,
                "src-a",
                HashMap::from([("trace_id".to_string(), json!("t1"))]),
            ),
            (2, "src-a", HashMap::new()),
            (
                3,
                "src-b",
                HashMap::from([("trace_id".to_string(), json!("t1"))]),
            ),
        ] {
            state = producer::handle_producer_event(
                ProducerEvent::StoreEvent(NewLogEntry {
                    msg: format!("entry {seq}"),
                    ts: Utc::now(),
                    level: Some(LogLevel::Info),
                    source: Source {
                        producer: "fake".to_string(),
                        id: source_id.to_string(),
                        display_name: source_id.to_string(),
                        group: None,
                    },
                    fields,
                }),
                state,
            );
        }
        let pane_id = focused_pane_id(&state);

        let mut state = handle_search_event(
            SearchEvent::Search {
                target: pane_id,
                query: Query::FieldMatched {
                    anchor_seq_id: 3,
                    buffer: 4,
                    predicates: vec![FieldPredicate {
                        key: "trace_id".to_string(),
                        value: json!("t1"),
                    }],
                },
                sources: vec!["ignored".to_string()],
            },
            state,
        );

        let event = tokio::time::timeout(
            Duration::from_secs(2),
            state.event_bus.search_event_rx.recv(),
        )
        .await
        .expect("timed out awaiting field-matched result")
        .expect("search event");
        state.search.cancel(pane_id);

        match event {
            SearchEvent::Result {
                target,
                query,
                results,
                hit_seqs: _,
                request_id,
                complete,
                progress,
            } => {
                assert_eq!(target, pane_id);
                assert!(matches!(
                    query,
                    Query::FieldMatched {
                        anchor_seq_id: 3,
                        ..
                    }
                ));
                assert_eq!(request_id, 1);
                assert!(complete);
                assert_eq!(progress, None);
                assert_eq!(
                    results
                        .into_iter()
                        .map(|result| result.seq_id)
                        .collect::<Vec<_>>(),
                    vec![1, 3]
                );
            }
            event => panic!("expected field-matched result, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn new_search_for_same_target_bumps_request_id() {
        let state = state_with_entries(3);
        let pane_id = focused_pane_id(&state);

        let state = handle_search_event(
            SearchEvent::Search {
                target: pane_id,
                query: Query::Tail,
                sources: Vec::new(),
            },
            state,
        );
        let mut state = handle_search_event(
            SearchEvent::Search {
                target: pane_id,
                query: Query::Tail,
                sources: Vec::new(),
            },
            state,
        );

        assert_eq!(state.search.latest_request_id(pane_id), 2);
        state.search.cancel(pane_id);
    }
}
