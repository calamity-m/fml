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
                    // Sequence ID can be derived into index via mod'ing the capacity.
                    // Fetching them after that is easy.

                    let index = (seq % self.config.capacity as u64) as usize;
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
