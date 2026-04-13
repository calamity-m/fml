use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    config::store::StoreConfig,
    error::FmlError,
    log::{LogEntry, NewLogEntry},
};

/// Read interface for the log store.
///
/// Writes are not part of this trait. Concrete implementations expose a
/// `spawn()` constructor that returns `(Arc<dyn LogStore>, mpsc::Sender<NewLogEntry>)`.
/// The sender is the write path — ingest sources clone it and send [`NewLogEntry`]
/// values through the channel. A background task owned by the implementation
/// drains the receiver and inserts entries into the store, making it the sole
/// writer. This keeps lock contention off the ingest hot path entirely.
///
/// Readers (search engine, app) hold `Arc<dyn LogStore>` and call the methods
/// below directly. Synchronization is handled internally by the implementation
/// (e.g. `RwLock`), so callers never manage locks.
pub trait LogStore: Send + Sync {
    /// Clear the retained logs from this store
    fn clear(&self) -> Result<(), FmlError>;

    /// Fetch a continuous range from the log store
    fn fetch_range(
        &self,
        lower_bound: u64,
        upper_bound: u64,
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError>;

    /// Fetch requested sequence ids from the log store
    fn fetch_requested(
        &self,
        requested: &[u64],
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError>;

    /// Retrieve the monotonic id bounds of the store, (low, high)
    fn bounds(&self) -> (u64, u64);
}

struct RingBuffer {
    entries: VecDeque<Arc<LogEntry>>,
    capacity: usize,
    next_seq: u64,
}

pub struct RingBufferStore {
    config: StoreConfig,
    rb: RwLock<RingBuffer>,
}

impl RingBufferStore {
    pub fn new(config: StoreConfig) -> (Arc<dyn LogStore>, mpsc::Sender<NewLogEntry>) {
        let s = Arc::new(Self {
            config: config.clone(),

            rb: RwLock::new(RingBuffer {
                entries: VecDeque::with_capacity(config.capacity),
                capacity: config.capacity,
                next_seq: 1,
            }),
        });

        let (tx, rx) = mpsc::channel(config.channel_capacity);

        let writer_store = Arc::clone(&s);

        tokio::spawn(async move {
            Self::writer_loop(writer_store, rx, config).await;
        });

        (s as Arc<dyn LogStore>, tx)
    }

    async fn writer_loop(
        store: Arc<Self>,
        mut rx: mpsc::Receiver<NewLogEntry>,
        config: StoreConfig,
    ) {
        let mut inserted = 0_u64;
        debug!("ring log store writer task started");

        while let Some(entry) = rx.recv().await {
            let seq = store.insert(entry);
            inserted += 1;

            if inserted.is_multiple_of(config.writer_log_internal) {
                debug!(inserted, latest_seq = seq, "ring log store writer progress");
            }
        }

        info!(
            inserted,
            "ring log store writer exiting after channel closure"
        );
    }

    fn insert(&self, entry: NewLogEntry) -> u64 {
        let mut state = self.write_state();
        let seq = state.next_seq;
        state.next_seq = state.next_seq.saturating_add(1);

        if state.entries.len() == state.capacity {
            state.entries.pop_front();
        }

        state.entries.push_back(Arc::new(LogEntry {
            seq,
            msg: entry.msg,
            ts: entry.ts,
            level: entry.level,
            source: entry.source,
            fields: entry.fields,
        }));

        seq
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, RingBuffer> {
        self.rb.read().unwrap_or_else(|poisoned| {
            warn!("recovering from poisoned log store read lock");
            poisoned.into_inner()
        })
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, RingBuffer> {
        self.rb.write().unwrap_or_else(|poisoned| {
            warn!("recovering from poisoned log store write lock");
            poisoned.into_inner()
        })
    }
}

impl LogStore for RingBufferStore {
    fn clear(&self) -> Result<(), FmlError> {
        self.write_state().entries.clear();
        Ok(())
    }

    fn fetch_range(
        &self,
        lower_bound: u64,
        upper_bound: u64,
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError> {
        if lower_bound > upper_bound {
            return Ok(());
        }

        let state = self.read_state();
        let Some(first) = state.entries.front() else {
            // Empty store, return nothing
            return Ok(());
        };
        let Some(last) = state.entries.back() else {
            // Empty store, return nothing
            return Ok(());
        };

        let retained_lower = first.seq;
        let retained_upper = last.seq;

        if upper_bound < retained_lower || lower_bound > retained_upper {
            return Ok(());
        }

        let lower_bound = lower_bound.max(retained_lower);
        let upper_bound = upper_bound.min(retained_upper);

        let start = (lower_bound - retained_lower) as usize;
        let end = (upper_bound - retained_lower) as usize;

        out.extend(
            state
                .entries
                .iter()
                .skip(start)
                .take(end - start + 1)
                .cloned(),
        );

        Ok(())
    }

    fn bounds(&self) -> (u64, u64) {
        let state = self.read_state();
        match (state.entries.front(), state.entries.back()) {
            (Some(first), Some(last)) => (first.seq, last.seq),
            _ => (0, 0),
        }
    }

    fn fetch_requested(
        &self,
        requested: &[u64],
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError> {
        debug!(count = requested.len(), "fetching requested from log store");

        let state = self.read_state();
        let Some(first) = state.entries.front() else {
            // Empty store, return nothing
            return Ok(());
        };
        let Some(last) = state.entries.back() else {
            // Empty store, return nothing
            return Ok(());
        };

        let retained_lower = first.seq;
        let retained_upper = last.seq;

        // Add into the out vec the desired log entries
        out.extend(
            requested
                .iter()
                .filter(|&&seq| {
                    // Validate any seq against our retained sequence ids - if we're outside of
                    // bounds of them, there's no point in trying to find them as they're definitely
                    // missing.
                    //
                    let in_range = seq >= retained_lower && seq <= retained_upper;
                    if !in_range {
                        warn!(
                            requested_seq = seq,
                            retained_lower, retained_upper, "attempted to fetch out of bounds seq"
                        );
                    }
                    in_range
                })
                .filter_map(|&seq| {
                    // VecDeque pop_front() shifts logical indices, so index 0 is
                    // always the oldest entry. Offset from front, not mod capacity.
                    let index = (seq - retained_lower) as usize;
                    let entry = state.entries.get(index);
                    if entry.is_none() {
                        warn!(
                            requested_seq = seq,
                            retained_lower,
                            retained_upper,
                            index,
                            "idx {} was not found in log store",
                            index
                        );
                    }
                    entry.cloned()
                }),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;

    use super::*;
    use crate::config::store::StoreConfig;
    use crate::log::{LogLevel, NewLogEntry, Source};

    fn test_config(capacity: usize) -> StoreConfig {
        StoreConfig {
            capacity,
            writer_log_internal: 100,
            channel_capacity: 64,
        }
    }

    fn make_entry(msg: &str) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {},
            fields: HashMap::new(),
        }
    }

    /// Send entries through the channel and drop the sender so the writer
    /// task flushes everything before we read.
    async fn populate(
        tx: &mpsc::Sender<NewLogEntry>,
        msgs: &[&str],
    ) {
        for msg in msgs {
            tx.send(make_entry(msg)).await.unwrap();
        }
    }

    /// Convenience: wait for the writer task to drain by dropping the sender
    /// and yielding until the store reflects the expected count.
    async fn flush_and_wait(tx: mpsc::Sender<NewLogEntry>, store: &Arc<dyn LogStore>, expected_upper: u64) {
        drop(tx);
        // Give the writer task time to drain
        for _ in 0..100 {
            if store.bounds().1 >= expected_upper {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "store did not reach expected upper bound {} (got {:?})",
            expected_upper,
            store.bounds()
        );
    }

    // --- bounds ---

    #[tokio::test]
    async fn empty_store_has_zero_bounds() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        drop(tx);
        tokio::task::yield_now().await;
        assert_eq!(store.bounds(), (0, 0));
    }

    #[tokio::test]
    async fn bounds_reflect_inserted_entries() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b", "c"]).await;
        flush_and_wait(tx, &store, 3).await;

        assert_eq!(store.bounds(), (1, 3));
    }

    #[tokio::test]
    async fn bounds_advance_after_eviction() {
        let (store, tx) = RingBufferStore::new(test_config(3));
        // Insert 5 entries into a capacity-3 buffer: entries 1,2 get evicted
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        let (lo, hi) = store.bounds();
        assert_eq!(lo, 3);
        assert_eq!(hi, 5);
    }

    // --- clear ---

    #[tokio::test]
    async fn clear_empties_the_store() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b"]).await;
        flush_and_wait(tx, &store, 2).await;

        store.clear().unwrap();
        assert_eq!(store.bounds(), (0, 0));
    }

    // --- fetch_range ---

    #[tokio::test]
    async fn fetch_range_returns_all_entries() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b", "c"]).await;
        flush_and_wait(tx, &store, 3).await;

        let mut out = Vec::new();
        store.fetch_range(1, 3, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "a");
        assert_eq!(out[1].msg, "b");
        assert_eq!(out[2].msg, "c");
    }

    #[tokio::test]
    async fn fetch_range_subset() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b", "c", "d"]).await;
        flush_and_wait(tx, &store, 4).await;

        let mut out = Vec::new();
        store.fetch_range(2, 3, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].msg, "b");
        assert_eq!(out[1].msg, "c");
    }

    #[tokio::test]
    async fn fetch_range_clamps_to_retained() {
        let (store, tx) = RingBufferStore::new(test_config(3));
        // Insert 5 entries, only seq 3,4,5 remain
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        // Request range 1..5 — should clamp to 3..5
        let mut out = Vec::new();
        store.fetch_range(1, 5, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "c");
        assert_eq!(out[1].msg, "d");
        assert_eq!(out[2].msg, "e");
    }

    #[tokio::test]
    async fn fetch_range_entirely_outside_returns_empty() {
        let (store, tx) = RingBufferStore::new(test_config(3));
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        // Request range that was evicted
        let mut out = Vec::new();
        store.fetch_range(1, 2, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn fetch_range_inverted_bounds_returns_empty() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b"]).await;
        flush_and_wait(tx, &store, 2).await;

        let mut out = Vec::new();
        store.fetch_range(5, 1, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn fetch_range_empty_store_returns_empty() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        drop(tx);
        tokio::task::yield_now().await;

        let mut out = Vec::new();
        store.fetch_range(1, 10, &mut out).unwrap();
        assert!(out.is_empty());
    }

    // --- fetch_requested ---

    #[tokio::test]
    async fn fetch_requested_returns_entries_by_seq() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b", "c", "d"]).await;
        flush_and_wait(tx, &store, 4).await;

        let mut out = Vec::new();
        store.fetch_requested(&[2, 4], &mut out).unwrap();
        assert_eq!(out.len(), 2);
        // Verify we got the right entries by seq
        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert!(seqs.contains(&2));
        assert!(seqs.contains(&4));
    }

    #[tokio::test]
    async fn fetch_requested_skips_evicted_seqs() {
        let (store, tx) = RingBufferStore::new(test_config(3));
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        // Seq 1 and 2 were evicted — requesting them should filter them out
        let mut out = Vec::new();
        store.fetch_requested(&[1, 2, 4], &mut out).unwrap();
        // Only seq 4 is in range (3..5)
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, 4);
    }

    #[tokio::test]
    async fn fetch_requested_empty_store() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        drop(tx);
        tokio::task::yield_now().await;

        let mut out = Vec::new();
        store.fetch_requested(&[1, 2, 3], &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn fetch_requested_empty_request() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b"]).await;
        flush_and_wait(tx, &store, 2).await;

        let mut out = Vec::new();
        store.fetch_requested(&[], &mut out).unwrap();
        assert!(out.is_empty());
    }

    // --- sequence numbering ---

    #[tokio::test]
    async fn sequence_ids_are_monotonic() {
        let (store, tx) = RingBufferStore::new(test_config(8));
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        let mut out = Vec::new();
        store.fetch_range(1, 5, &mut out).unwrap();
        for w in out.windows(2) {
            assert_eq!(w[1].seq, w[0].seq + 1, "sequence ids must be consecutive");
        }
    }

    // --- eviction ---

    #[tokio::test]
    async fn eviction_preserves_newest_entries() {
        let (store, tx) = RingBufferStore::new(test_config(3));
        populate(&tx, &["a", "b", "c", "d", "e"]).await;
        flush_and_wait(tx, &store, 5).await;

        let mut out = Vec::new();
        let (lo, hi) = store.bounds();
        store.fetch_range(lo, hi, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "c");
        assert_eq!(out[1].msg, "d");
        assert_eq!(out[2].msg, "e");
    }
}
