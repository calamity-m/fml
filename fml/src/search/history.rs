use std::sync::Arc;

use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{
    error::FmlError,
    event::SearchEvent,
    log::{LogEntry, SourceId},
    search::{EmitOutcome, emit_error, emit_results},
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
pub fn start_history_search(
    middle_seq_id: u64,
    buffer: u64,
    sources: Vec<SourceId>,
    store: Arc<dyn LogStore>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "spawned history search - middle_seq_id: {}, buffer: {}, sources: {:?}",
            middle_seq_id, buffer, sources
        );

        let entries = match collect_window(&store, middle_seq_id, buffer, &sources) {
            Ok(entries) => entries,
            Err(e) => {
                let _ = emit_error(e.to_string(), &tx).await;
                return;
            }
        };

        match emit_results(entries, request_id, true, &tx).await {
            EmitOutcome::Sent | EmitOutcome::ReceiverGone => {}
        }
    })
}

fn collect_window(
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
        event::SearchHit,
        log::{LogLevel, NewLogEntry, Source},
        store::RingBufferStore,
    };

    fn store_config(capacity: usize, channel_capacity: usize) -> StoreConfig {
        StoreConfig {
            capacity,
            writer_log_internal: 10_000,
            channel_capacity,
        }
    }

    fn make_entry(msg: &str, source_id: &str) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                id: source_id.to_string(),
            },
            fields: HashMap::new(),
        }
    }

    async fn wait_for_bounds(store: &Arc<dyn LogStore>, expected_high: u64) {
        for _ in 0..50_000 {
            if store.bounds().1 >= expected_high {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "store did not reach expected upper bound {} (got {:?})",
            expected_high,
            store.bounds()
        );
    }

    async fn recv_result(rx: &mut mpsc::Receiver<SearchEvent>) -> (Vec<SearchHit>, u64, bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out awaiting SearchEvent")
            .expect("channel closed before delivering SearchEvent");
        match evt {
            SearchEvent::Result {
                results,
                request_id,
                complete,
            } => (results, request_id, complete),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
        }
    }

    #[tokio::test]
    async fn collects_buffer_per_side_single_chunk() {
        let (store, store_tx) = RingBufferStore::new(store_config(64, 64));
        for i in 1..=10 {
            store_tx
                .send(make_entry(&format!("e{i}"), "s1"))
                .await
                .unwrap();
        }
        wait_for_bounds(&store, 10).await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_history_search(5, 2, vec![], store.clone(), 1, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 1);
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![4, 5, 6, 7]);

        handle.abort();
    }

    #[tokio::test]
    async fn filter_aware_buffer_with_mixed_sources() {
        let (store, store_tx) = RingBufferStore::new(store_config(128, 128));
        // Alternate s1 / s2 for seqs 1..=20.
        for i in 1..=20 {
            let src = if i % 2 == 1 { "s1" } else { "s2" };
            store_tx
                .send(make_entry(&format!("e{i}"), src))
                .await
                .unwrap();
        }
        wait_for_bounds(&store, 20).await;

        // middle=10 (s2). s1 entries: 1,3,5,7,9,11,13,15,17,19. Left ≤ 10 matches:
        // 9, 7. Right > 10 matches: 11, 13.
        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            start_history_search(10, 2, vec!["s1".to_string()], store.clone(), 42, tx);

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
        let (store, store_tx) = RingBufferStore::new(store_config(2048, 2048));
        for i in 1..=1800u64 {
            let src = if i % 300 == 0 { "s1" } else { "s2" };
            store_tx
                .send(make_entry(&format!("e{i}"), src))
                .await
                .unwrap();
        }
        wait_for_bounds(&store, 1800).await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            start_history_search(1000, 2, vec!["s1".to_string()], store.clone(), 7, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![600, 900, 1200, 1500]);

        handle.abort();
    }

    #[tokio::test]
    async fn middle_beyond_retained_collects_from_top() {
        let (store, store_tx) = RingBufferStore::new(store_config(64, 64));
        for i in 1..=5 {
            store_tx
                .send(make_entry(&format!("e{i}"), "s1"))
                .await
                .unwrap();
        }
        wait_for_bounds(&store, 5).await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_history_search(100, 3, vec![], store.clone(), 1, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![3, 4, 5]);

        handle.abort();
    }

    #[tokio::test]
    async fn empty_store_yields_empty_result() {
        let (store, _store_tx) = RingBufferStore::new(store_config(64, 64));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_history_search(10, 3, vec![], store.clone(), 99, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 99);
        assert!(complete);
        assert!(results.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn no_matching_sources_yields_empty_result() {
        let (store, store_tx) = RingBufferStore::new(store_config(64, 64));
        for i in 1..=5 {
            store_tx
                .send(make_entry(&format!("e{i}"), "s1"))
                .await
                .unwrap();
        }
        wait_for_bounds(&store, 5).await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            start_history_search(3, 2, vec!["s2".to_string()], store.clone(), 1, tx);

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert!(results.is_empty());

        handle.abort();
    }
}
