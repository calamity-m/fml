use std::{sync::Arc, time::Duration};

use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{
    event::SearchEvent,
    log::{LogEntry, SourceId},
    search::{EmitOutcome, emit_error, emit_results},
    store::LogStore,
};

/// Starts the background worker for a tail-oriented search request.
///
/// The worker re-emits the full tail window (up to `tail_size` entries) each
/// time `LogStore::bounds().1` advances. Emissions share `request_id` so that
/// `handle_search_event` discards results from superseded queries. Cancellation
/// is prompt because every iteration yields at `ticker.tick().await` or inside
/// `emit_results`.
pub fn start_tail_search(
    sources: Vec<SourceId>,
    tail_size: usize,
    poll_interval: Duration,
    store: Arc<dyn LogStore>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "spawned tail search - sources: {:?}, tail_size: {}, poll_interval: {:?}",
            sources, tail_size, poll_interval
        );

        let mut last_sent_high: Option<u64> = None;
        let mut ticker = tokio::time::interval(poll_interval);

        loop {
            ticker.tick().await;

            let (low, high) = store.bounds();
            if last_sent_high == Some(high) {
                continue;
            }

            let entries = if high == 0 {
                Vec::new()
            } else {
                let lower = high.saturating_sub(tail_size as u64 - 1).max(low);
                let mut pool: Vec<Arc<LogEntry>> = Vec::new();
                if let Err(e) = store.fetch_range(lower, high, &mut pool) {
                    let _ = emit_error(e.to_string(), &tx).await;
                    return;
                }
                pool.into_iter()
                    .filter(|entry| sources.is_empty() || sources.contains(&entry.source.id))
                    .collect()
            };

            match emit_results(entries, request_id, true, &tx).await {
                EmitOutcome::Sent => {}
                EmitOutcome::ReceiverGone => return,
            }

            last_sent_high = Some(high);
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        event::SearchHit,
        log::{LogLevel, NewLogEntry, Source},
        store::RingBufferStore,
    };

    fn test_store_config() -> StoreConfig {
        StoreConfig {
            capacity: 64,
            writer_log_internal: 100,
            channel_capacity: 64,
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

    async fn populate(
        tx: &mpsc::Sender<NewLogEntry>,
        store: &Arc<dyn LogStore>,
        entries: &[(&str, &str)],
        expected_high: u64,
    ) {
        for (msg, source) in entries {
            tx.send(make_entry(msg, source)).await.unwrap();
        }
        for _ in 0..500 {
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
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
    async fn seeds_with_last_n_entries() {
        let (store, store_tx) = RingBufferStore::new(test_store_config());
        populate(
            &store_tx,
            &store,
            &[
                ("a", "s1"),
                ("b", "s1"),
                ("c", "s1"),
                ("d", "s1"),
                ("e", "s1"),
            ],
            5,
        )
        .await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(vec![], 3, Duration::from_millis(10), store.clone(), 1, tx);

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 1);
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![3, 4, 5]);

        handle.abort();
    }

    #[tokio::test]
    async fn re_emits_full_window_on_new_entries() {
        let (store, store_tx) = RingBufferStore::new(test_store_config());
        populate(
            &store_tx,
            &store,
            &[("a", "s1"), ("b", "s1"), ("c", "s1")],
            3,
        )
        .await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(vec![], 5, Duration::from_millis(10), store.clone(), 7, tx);

        let (seed, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert_eq!(
            seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        populate(&store_tx, &store, &[("d", "s1"), ("e", "s1")], 5).await;

        let (update, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 7);
        assert!(complete);
        assert_eq!(
            update.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn seeds_empty_then_emits_after_population() {
        let (store, store_tx) = RingBufferStore::new(test_store_config());

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(vec![], 4, Duration::from_millis(10), store.clone(), 42, tx);

        let (seed, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 42);
        assert!(complete);
        assert!(seed.is_empty());

        populate(&store_tx, &store, &[("a", "s1"), ("b", "s1")], 2).await;

        let (update, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert_eq!(
            update.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 2]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn filters_by_source_id() {
        let (store, store_tx) = RingBufferStore::new(test_store_config());
        populate(
            &store_tx,
            &store,
            &[
                ("a", "s1"),
                ("b", "s2"),
                ("c", "s1"),
                ("d", "s3"),
            ],
            4,
        )
        .await;

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            vec!["s1".to_string()],
            10,
            Duration::from_millis(10),
            store.clone(),
            1,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![1, 3]);

        handle.abort();
    }
}
