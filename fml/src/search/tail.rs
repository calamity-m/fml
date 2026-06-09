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
/// time a *matching* entry advances the tail. Filtering is delegated to
/// `LogStore::fetch_tail_filtered`, so the window stays full even when an
/// unrelated hot source dominates ingestion. The re-emit gate tracks the
/// max sequence id in the last emitted set, suppressing redundant emits
/// while only non-matching entries arrive. Cancellation is prompt because
/// every iteration yields at `ticker.tick().await` or inside `emit_results`.
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

        let mut last_sent_match_high: Option<u64> = None;
        let mut seeded = false;
        let mut ticker = tokio::time::interval(tick_rate);

        loop {
            ticker.tick().await;

            let mut entries: Vec<Arc<LogEntry>> = Vec::new();
            if let Err(e) = store.fetch_tail_filtered(&sources, tail_size, &mut entries) {
                let _ = emit_error(e.to_string(), &tx).await;
                return;
            }

            let current_match_high = entries.last().map(|e| e.seq);
            // Skip re-emit when no new matching entry has advanced the tail.
            // Always emit at least once so consumers see the seed (possibly
            // empty) state.
            if seeded && current_match_high == last_sent_match_high {
                continue;
            }

            match emit_results(target, query.clone(), entries, request_id, true, &tx).await {
                EmitOutcome::Sent => {}
                EmitOutcome::ReceiverGone => return,
            }

            last_sent_match_high = current_match_high;
            seeded = true;
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
        event::{Query, SearchEvent, SearchHit},
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
                target: crate::event::PaneId(1),
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
                target: crate::event::PaneId(1),
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
                target: crate::event::PaneId(1),
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
    async fn fills_tail_window_with_matching_entries_when_hot_source_dominates() {
        // Issue #13 repro: hot source dominates ingestion, slow source is the
        // focused filter. The tail window must walk back past hot entries
        // to fill `tail_size` matching slow entries.
        let store = RingBufferStore::new(test_store_config());
        let mut entries: Vec<(&str, &str)> = Vec::new();
        // Pattern: [9 hot, 1 slow] x 5 followed by 9 trailing hot entries.
        // Slow entries land at seqs 10, 20, 30, 40, 50; high = 59.
        let slow_msgs = ["s0", "s1", "s2", "s3", "s4"];
        for &slow_msg in &slow_msgs {
            for _ in 0..9 {
                entries.push(("h", "hot"));
            }
            entries.push((slow_msg, "slow"));
        }
        for _ in 0..9 {
            entries.push(("h", "hot"));
        }
        populate(&store, &entries);

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: crate::event::PaneId(1),
                query: Query::Tail,
                sources: vec!["slow".to_string()],
                request_id: 1,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            3,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        // Slow entries land at seqs 10, 20, 30, 40, 50; last 3 ascending:
        assert_eq!(
            seqs,
            vec![30, 40, 50],
            "tail window must contain last 3 slow entries by seq, ascending"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn does_not_re_emit_when_only_non_matching_entries_arrive() {
        // Hot inserts should not trigger re-emit when the focused source has
        // produced nothing new. Today the gate uses global high, so this fails.
        let store = RingBufferStore::new(test_store_config());
        populate(&store, &[("a", "slow"), ("b", "hot"), ("c", "slow")]);

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_tail_search(
            SearchContext {
                target: crate::event::PaneId(1),
                query: Query::Tail,
                sources: vec!["slow".to_string()],
                request_id: 1,
                tick_rate: Duration::from_millis(10),
                store: store.clone(),
                tx,
            },
            5,
        );

        // Drain the seed emission.
        let (seed, _, _) = recv_result(&mut rx).await;
        assert_eq!(
            seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 3]
        );

        // Insert many hot-only entries; matching set does not change.
        for _ in 0..50 {
            store.insert(make_entry("h", "hot"));
        }

        // Wait several ticks; no further Result event should arrive.
        let res = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
        assert!(
            res.is_err(),
            "unexpected re-emit when no new matching entries arrived: {:?}",
            res
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
                target: crate::event::PaneId(1),
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
