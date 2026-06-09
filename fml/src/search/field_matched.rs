use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::debug;

use crate::{
    error::FmlError,
    event::FieldPredicate,
    log::LogEntry,
    search::{EmitOutcome, SearchContext, emit_error, emit_results},
    store::LogStore,
};

/// Starts the background worker for field-matched preview searches.
///
/// The worker scans retained logs around `anchor_seq_id` and returns up to
/// `buffer` entries per side whose fields exactly equal every predicate. Source
/// filters are intentionally ignored so request ids or trace parents can be
/// followed across sources.
pub fn start_field_matched_search(
    ctx: SearchContext,
    anchor_seq_id: u64,
    buffer: u64,
    predicates: Vec<FieldPredicate>,
) -> JoinHandle<()> {
    let SearchContext {
        target,
        query,
        sources: _,
        request_id,
        tick_rate,
        store,
        tx,
    } = ctx;

    tokio::spawn(async move {
        debug!(
            "spawned field-matched search - anchor_seq_id: {}, buffer: {}, predicates: {:?}, tick_rate: {:?}",
            anchor_seq_id, buffer, predicates, tick_rate
        );

        let mut last_emitted_bounds: Option<(u64, u64)> = None;
        let mut ticker = tokio::time::interval(tick_rate);

        loop {
            ticker.tick().await;

            let bounds = store.bounds();
            if last_emitted_bounds == Some(bounds) {
                continue;
            }

            let entries = match collect_window(&store, anchor_seq_id, buffer, &predicates) {
                Ok(entries) => entries,
                Err(e) => {
                    let _ = emit_error(e.to_string(), &tx).await;
                    return;
                }
            };

            match emit_results(target, query.clone(), entries, request_id, true, &tx).await {
                EmitOutcome::Sent => {}
                EmitOutcome::ReceiverGone => return,
            }

            last_emitted_bounds = Some(bounds);
        }
    })
}

pub(crate) fn collect_window(
    store: &Arc<dyn LogStore>,
    anchor_seq_id: u64,
    buffer: u64,
    predicates: &[FieldPredicate],
) -> Result<Vec<Arc<LogEntry>>, FmlError> {
    let (store_low, store_high) = store.bounds();
    if store_high == 0 || buffer == 0 || predicates.is_empty() {
        return Ok(Vec::new());
    }
    if anchor_seq_id < store_low || anchor_seq_id > store_high {
        return Ok(Vec::new());
    }

    let target = buffer as usize;
    let chunk_size = buffer * 4;

    let mut left: Vec<Arc<LogEntry>> = Vec::new();
    let left_start = anchor_seq_id;
    if left_start >= store_low {
        let mut cursor_upper = left_start;
        loop {
            let cursor_lower = cursor_upper.saturating_sub(chunk_size - 1).max(store_low);
            let mut pool: Vec<Arc<LogEntry>> = Vec::new();
            store.fetch_range(cursor_lower, cursor_upper, &mut pool)?;
            for entry in pool.into_iter().rev() {
                if matches_predicates(&entry, predicates) {
                    left.push(entry);
                    if left.len() >= target {
                        break;
                    }
                }
            }
            if left.len() >= target || cursor_lower == store_low {
                break;
            }
            cursor_upper = cursor_lower - 1;
        }
    }
    left.reverse();

    let mut right: Vec<Arc<LogEntry>> = Vec::new();
    if anchor_seq_id < store_high {
        let mut cursor_lower = anchor_seq_id.saturating_add(1).max(store_low);
        while cursor_lower <= store_high {
            let cursor_upper = cursor_lower.saturating_add(chunk_size - 1).min(store_high);
            let mut pool: Vec<Arc<LogEntry>> = Vec::new();
            store.fetch_range(cursor_lower, cursor_upper, &mut pool)?;
            for entry in pool {
                if matches_predicates(&entry, predicates) {
                    right.push(entry);
                    if right.len() >= target {
                        break;
                    }
                }
            }
            if right.len() >= target || cursor_upper == store_high {
                break;
            }
            cursor_lower = cursor_upper + 1;
        }
    }

    let mut out = left;
    out.extend(right);
    Ok(out)
}

fn matches_predicates(entry: &LogEntry, predicates: &[FieldPredicate]) -> bool {
    predicates
        .iter()
        .all(|predicate| entry.fields.get(&predicate.key) == Some(&predicate.value))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use chrono::Utc;
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        event::{Query, SearchEvent, SearchHit},
        log::{LogLevel, NewLogEntry, Source},
        store::RingBufferStore,
    };

    fn store_config(capacity: usize) -> StoreConfig {
        StoreConfig { capacity }
    }

    fn make_entry(
        msg: &str,
        source_id: &str,
        fields: HashMap<String, serde_json::Value>,
    ) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: source_id.to_string(),
                display_name: source_id.to_string(),
                group: None,
            },
            fields,
        }
    }

    fn predicate(key: &str, value: serde_json::Value) -> FieldPredicate {
        FieldPredicate {
            key: key.to_string(),
            value,
        }
    }

    fn seqs(entries: Vec<Arc<LogEntry>>) -> Vec<u64> {
        entries.into_iter().map(|entry| entry.seq).collect()
    }

    async fn recv_result(rx: &mut mpsc::Receiver<SearchEvent>) -> (Vec<SearchHit>, u64, bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out awaiting SearchEvent")
            .expect("channel closed before delivering SearchEvent");
        match evt {
            SearchEvent::Result {
                target: _,
                query: _,
                results,
                request_id,
                complete,
                progress: _,
            } => (results, request_id, complete),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
            SearchEvent::Cancel { .. } => panic!("unexpected SearchEvent::Cancel"),
        }
    }

    #[test]
    fn exact_json_value_equality_does_not_coerce_types() {
        let store = RingBufferStore::new(store_config(16));
        store.insert(make_entry(
            "string",
            "s1",
            HashMap::from([("status".to_string(), json!("500"))]),
        ));
        store.insert(make_entry(
            "number",
            "s1",
            HashMap::from([("status".to_string(), json!(500))]),
        ));

        let entries = collect_window(&store, 2, 4, &[predicate("status", json!(500))])
            .expect("field matched entries");

        assert_eq!(seqs(entries), vec![2]);
    }

    #[test]
    fn multiple_predicates_use_and_semantics() {
        let store = RingBufferStore::new(store_config(16));
        store.insert(make_entry(
            "request only",
            "s1",
            HashMap::from([("request_id".to_string(), json!("abc"))]),
        ));
        store.insert(make_entry(
            "both",
            "s1",
            HashMap::from([
                ("request_id".to_string(), json!("abc")),
                ("status".to_string(), json!(500)),
            ]),
        ));
        store.insert(make_entry(
            "status only",
            "s1",
            HashMap::from([("status".to_string(), json!(500))]),
        ));

        let entries = collect_window(
            &store,
            2,
            4,
            &[
                predicate("request_id", json!("abc")),
                predicate("status", json!(500)),
            ],
        )
        .expect("field matched entries");

        assert_eq!(seqs(entries), vec![2]);
    }

    #[test]
    fn matches_across_sources_and_sparse_chunks() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=20 {
            let fields = if matches!(i, 1 | 9 | 10 | 19) {
                HashMap::from([("trace_id".to_string(), json!("t1"))])
            } else {
                HashMap::new()
            };
            let source = if i % 2 == 0 { "s2" } else { "s1" };
            store.insert(make_entry(&format!("e{i}"), source, fields));
        }

        let entries = collect_window(&store, 10, 2, &[predicate("trace_id", json!("t1"))])
            .expect("field matched entries");

        assert_eq!(seqs(entries), vec![9, 10, 19]);
    }

    #[test]
    fn anchor_eviction_yields_no_matches_when_anchor_is_outside_retained_bounds() {
        let store = RingBufferStore::new(store_config(3));
        for i in 1..=5 {
            store.insert(make_entry(
                &format!("e{i}"),
                "s1",
                HashMap::from([("trace_id".to_string(), json!("t1"))]),
            ));
        }

        let entries = collect_window(&store, 2, 2, &[predicate("trace_id", json!("t1"))])
            .expect("field matched entries");

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn worker_emits_matching_hits_with_request_id() {
        let store = RingBufferStore::new(store_config(16));
        store.insert(make_entry(
            "left",
            "s1",
            HashMap::from([("trace_id".to_string(), json!("t1"))]),
        ));
        store.insert(make_entry("skip", "s1", HashMap::new()));
        store.insert(make_entry(
            "anchor",
            "s2",
            HashMap::from([("trace_id".to_string(), json!("t1"))]),
        ));

        let (tx, mut rx) = mpsc::channel(8);
        let query = Query::FieldMatched {
            anchor_seq_id: 3,
            buffer: 4,
            predicates: vec![predicate("trace_id", json!("t1"))],
        };
        let handle = start_field_matched_search(
            SearchContext {
                target: crate::event::PaneId(1),
                query,
                sources: vec!["ignored".to_string()],
                request_id: 42,
                tick_rate: Duration::from_millis(10),
                store,
                tx,
            },
            3,
            4,
            vec![predicate("trace_id", json!("t1"))],
        );

        let (results, request_id, complete) = recv_result(&mut rx).await;
        handle.abort();

        assert_eq!(request_id, 42);
        assert!(complete);
        assert_eq!(
            results
                .into_iter()
                .map(|result| result.seq_id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }
}
