use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::debug;

use crate::{
    error::FmlError,
    log::{LogEntry, SourceId},
    search::{EmitOutcome, SearchContext, emit_error, emit_results},
    store::LogStore,
};

/// Starts the background worker for a history window search.
///
/// Resolves up to `buffer` entries on each side of `middle_seq_id` that match
/// the optional source filter. `buffer` is a count of *matching* context
/// entries — when a filter is active, the worker walks outward in chunks until
/// it has `buffer` matches per side or it reaches the store's retained bounds.
/// The anchor (`middle_seq_id`) lands at the tail of the left side when
/// retained and matching, so the total result is at most `2 * buffer` entries.
///
/// The worker re-evaluates the window every `ctx.tick_rate` and re-emits when
/// the store's retained `(low, high)` bounds advance — newly ingested entries
/// or eviction will both trigger a fresh emission. Entering history mode
/// before the buffer is saturated is therefore safe: subsequent inserts will
/// flow through to the receiver instead of being lost when the worker exits.
pub fn start_history_search(
    ctx: SearchContext,
    middle_seq_id: u64,
    buffer: u64,
) -> JoinHandle<()> {
    let SearchContext {
        target,
        query,
        sources,
        request_id,
        tick_rate,
        store,
        tx,
    } = ctx;

    tokio::spawn(async move {
        debug!(
            "spawned history search - middle_seq_id: {}, buffer: {}, sources: {:?}, tick_rate: {:?}",
            middle_seq_id, buffer, sources, tick_rate
        );

        let mut last_emitted_bounds: Option<(u64, u64)> = None;
        let mut ticker = tokio::time::interval(tick_rate);

        loop {
            ticker.tick().await;

            let bounds = store.bounds();
            if last_emitted_bounds == Some(bounds) {
                continue;
            }

            let entries = match collect_window(&store, middle_seq_id, buffer, &sources) {
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
    middle: u64,
    buffer: u64,
    sources: &[SourceId],
) -> Result<Vec<Arc<LogEntry>>, FmlError> {
    let (store_low, store_high) = store.bounds();
    if store_high == 0 || buffer == 0 {
        return Ok(Vec::new());
    }

    let matches = |entry: &Arc<LogEntry>| sources.is_empty() || sources.contains(&entry.source.id);
    let target = buffer as usize;
    let chunk_size = buffer * 4;

    // Left scan: entries with seq ≤ middle, collected nearest-first then
    // reversed to restore ascending order.
    let mut left: Vec<Arc<LogEntry>> = Vec::new();
    let left_start = middle.min(store_high);
    if left_start >= store_low {
        let mut cursor_upper = left_start;
        loop {
            let cursor_lower = cursor_upper.saturating_sub(chunk_size - 1).max(store_low);
            let mut pool: Vec<Arc<LogEntry>> = Vec::new();
            store.fetch_range(cursor_lower, cursor_upper, &mut pool)?;
            for entry in pool.into_iter().rev() {
                if matches(&entry) {
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

    // Right scan: entries with seq > middle, ascending.
    let mut right: Vec<Arc<LogEntry>> = Vec::new();
    if middle < store_high {
        let mut cursor_lower = middle.saturating_add(1).max(store_low);
        while cursor_lower <= store_high {
            let cursor_upper = cursor_lower.saturating_add(chunk_size - 1).min(store_high);
            let mut pool: Vec<Arc<LogEntry>> = Vec::new();
            store.fetch_range(cursor_lower, cursor_upper, &mut pool)?;
            for entry in pool {
                if matches(&entry) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        event::{Query, SearchEvent, SearchHit, SearchTarget},
        log::{LogLevel, NewLogEntry, Source},
        search::SearchContext,
        store::RingBufferStore,
    };

    fn store_config(capacity: usize) -> StoreConfig {
        StoreConfig { capacity }
    }

    fn make_entry(msg: &str, source_id: &str) -> NewLogEntry {
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
            fields: HashMap::new(),
        }
    }

    async fn recv_result(rx: &mut mpsc::Receiver<SearchEvent>) -> (Vec<SearchHit>, u64, bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
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
            } => (results, request_id, complete),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
            SearchEvent::Cancel { .. } => panic!("unexpected SearchEvent::Cancel"),
        }
    }

    fn fast_poll() -> Duration {
        Duration::from_millis(10)
    }

    fn start_test_history_search(
        middle_seq_id: u64,
        buffer: u64,
        sources: Vec<SourceId>,
        store: Arc<dyn LogStore>,
        request_id: u64,
        tx: mpsc::Sender<SearchEvent>,
    ) -> JoinHandle<()> {
        start_history_search(
            SearchContext {
                target: SearchTarget::LogPane,
                query: Query::History {
                    middle_seq_id,
                    buffer,
                },
                sources,
                request_id,
                tick_rate: fast_poll(),
                store,
                tx,
            },
            middle_seq_id,
            buffer,
        )
    }

    #[tokio::test]
    async fn collects_buffer_per_side_single_chunk() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=10 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(5, 2, vec![], store.clone(), 1, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 1);
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![4, 5, 6, 7]);

        handle.abort();
    }

    #[tokio::test]
    async fn filter_aware_buffer_with_mixed_sources() {
        let store = RingBufferStore::new(store_config(128));
        // Alternate s1 / s2 for seqs 1..=20.
        for i in 1..=20 {
            let src = if i % 2 == 1 { "s1" } else { "s2" };
            store.insert(make_entry(&format!("e{i}"), src));
        }

        // middle=10 (s2). s1 entries: 1,3,5,7,9,11,13,15,17,19. Left ≤ 10 matches:
        // 9, 7. Right > 10 matches: 11, 13.
        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            start_test_history_search(10, 2, vec!["s1".to_string()], store.clone(), 42, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 42);
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![7, 9, 11, 13]);

        handle.abort();
    }

    #[tokio::test]
    async fn sparse_filter_expands_across_chunks() {
        // s1 every 300 entries; chunk_size = buffer * 4 = 8. At this density
        // each chunk is far too small to hold a match, forcing the worker to
        // walk many chunks per side before hitting its target.
        let store = RingBufferStore::new(store_config(2048));
        for i in 1..=1800u64 {
            let src = if i % 300 == 0 { "s1" } else { "s2" };
            store.insert(make_entry(&format!("e{i}"), src));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            start_test_history_search(1000, 2, vec!["s1".to_string()], store.clone(), 7, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![600, 900, 1200, 1500]);

        handle.abort();
    }

    #[tokio::test]
    async fn middle_beyond_retained_collects_from_top() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=5 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(100, 3, vec![], store.clone(), 1, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![3, 4, 5]);

        handle.abort();
    }

    #[tokio::test]
    async fn empty_store_yields_empty_result() {
        let store = RingBufferStore::new(store_config(64));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(10, 3, vec![], store.clone(), 99, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 99);
        assert!(complete);
        assert!(results.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn no_matching_sources_yields_empty_result() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=5 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(3, 2, vec!["s2".to_string()], store.clone(), 1, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert!(results.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn re_emits_on_bounds_advance() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=4 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(2, 3, vec![], store.clone(), 5, tx);

        let (seed, rid, _) = recv_result(&mut rx).await;
        assert_eq!(rid, 5);
        let seqs: Vec<u64> = seed.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4]);

        for i in 5..=8 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (update, _, _) = recv_result(&mut rx).await;
        let seqs: Vec<u64> = update.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);

        handle.abort();
    }

    #[tokio::test]
    async fn does_not_re_emit_when_bounds_unchanged() {
        let store = RingBufferStore::new(store_config(64));
        for i in 1..=4 {
            store.insert(make_entry(&format!("e{i}"), "s1"));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_history_search(2, 3, vec![], store.clone(), 1, tx);

        let _seed = recv_result(&mut rx).await;

        let result = tokio::time::timeout(Duration::from_millis(80), rx.recv()).await;
        assert!(
            result.is_err(),
            "expected no further emission when bounds are unchanged, got {result:?}"
        );

        handle.abort();
    }
}
