use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::debug;

use crate::{
    log::LogEntry,
    search::{EmitOutcome, SearchContext, emit_error, emit_results},
};

/// Starts the background worker for a tail-oriented search request.
///
/// The worker re-emits the full tail window (up to `tail_size` entries) each
/// time `LogStore::bounds().1` advances. Emissions share `ctx.target` and
/// `ctx.request_id` so that `handle_search_event` discards results from
/// superseded queries. Cancellation is prompt because every iteration yields
/// at `ticker.tick().await` or inside `emit_results`.
pub fn start_tail_search(ctx: SearchContext, tail_size: usize) -> JoinHandle<()> {
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
            "spawned tail search - sources: {:?}, tail_size: {}, tick_rate: {:?}",
            sources, tail_size, tick_rate
        );

        let mut last_sent_high: Option<u64> = None;
        let mut ticker = tokio::time::interval(tick_rate);

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

            match emit_results(target, query.clone(), entries, request_id, true, &tx).await {
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
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        event::{Query, SearchEvent, SearchHit, SearchTarget},
        log::{LogLevel, NewLogEntry, Source},
        search::SearchContext,
        store::{LogStore, RingBufferStore},
    };

    fn test_store_config() -> StoreConfig {
        StoreConfig { capacity: 64 }
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

    fn populate(store: &Arc<dyn LogStore>, entries: &[(&str, &str)]) {
        for (msg, source) in entries {
            store.insert(make_entry(msg, source));
        }
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

    #[tokio::test]
    async fn seeds_with_last_n_entries() {
        let store = RingBufferStore::new(test_store_config());
        populate(
            &store,
            &[
                ("a", "s1"),
                ("b", "s1"),
                ("c", "s1"),
                ("d", "s1"),
                ("e", "s1"),
            ],
        );

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: SearchTarget::LogPane,
                query: Query::Tail,
                sources: vec![],
                request_id: 1,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            3,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 1);
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![3, 4, 5]);

        handle.abort();
    }

    #[tokio::test]
    async fn re_emits_full_window_on_new_entries() {
        let store = RingBufferStore::new(test_store_config());
        populate(&store, &[("a", "s1"), ("b", "s1"), ("c", "s1")]);

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: SearchTarget::LogPane,
                query: Query::Tail,
                sources: vec![],
                request_id: 7,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            5,
        );

        let (seed, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert_eq!(
            seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        populate(&store, &[("d", "s1"), ("e", "s1")]);

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
        let store = RingBufferStore::new(test_store_config());

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: SearchTarget::LogPane,
                query: Query::Tail,
                sources: vec![],
                request_id: 42,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            4,
        );

        let (seed, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 42);
        assert!(complete);
        assert!(seed.is_empty());

        populate(&store, &[("a", "s1"), ("b", "s1")]);

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
        let store = RingBufferStore::new(test_store_config());
        populate(
            &store,
            &[("a", "s1"), ("b", "s2"), ("c", "s1"), ("d", "s3")],
        );

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: SearchTarget::LogPane,
                query: Query::Tail,
                sources: vec!["s1".to_string()],
                request_id: 1,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            10,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        assert_eq!(seqs, vec![1, 3]);

        handle.abort();
    }
}
