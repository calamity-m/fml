use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
};

use tracing::warn;

use crate::{
    config::store::StoreConfig,
    error::FmlError,
    log::{LogEntry, NewLogEntry},
};

/// Read/write interface for the log store.
///
/// All access — including writes — goes through this trait. Concrete
/// implementations expose a constructor that returns `Arc<dyn LogStore>`,
/// and synchronization is handled internally (e.g. `RwLock`), so callers
/// never manage locks. Inserts are serialized by the application's
/// producer event loop, which is the sole writer in normal operation.
pub trait LogStore: Send + Sync {
    /// Append a new entry and return the assigned monotonic sequence id.
    fn insert(&self, entry: NewLogEntry) -> u64;

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

    /// Fetch up to `limit` entries whose `source.id` is in `sources`, walking
    /// from the newest retained entry backward. Results are appended to `out`
    /// in ascending sequence order.
    ///
    /// An empty `sources` slice matches every entry, behaving as a plain
    /// tail of the last `limit` retained entries. A `limit` of zero is a
    /// no-op. The scan is bounded by the retained-entry count, so cost is
    /// O(retained) in the worst case (no matches) and O(matches) when the
    /// focused source is dense near the tail.
    fn fetch_tail_filtered(
        &self,
        sources: &[String],
        limit: usize,
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError>;

    /// Retrieve the monotonic id bounds of the store, (low, high)
    fn bounds(&self) -> (u64, u64);

    /// Retrieve retained-entry count, capacity, and sequence bounds.
    fn stats(&self) -> StoreStats;
}

/// Point-in-time capacity and retention information for the log store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    /// Number of entries currently retained in the store.
    pub retained: usize,
    /// Maximum number of entries this store can retain.
    pub capacity: usize,
    /// Monotonic id bounds of retained entries, `(oldest, newest)`.
    pub bounds: (u64, u64),
}

struct RingBuffer {
    entries: VecDeque<Arc<LogEntry>>,
    capacity: usize,
    next_seq: u64,
}

pub struct RingBufferStore {
    rb: RwLock<RingBuffer>,
}

impl RingBufferStore {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(config: StoreConfig) -> Arc<dyn LogStore> {
        Arc::new(Self {
            rb: RwLock::new(RingBuffer {
                entries: VecDeque::with_capacity(config.capacity),
                capacity: config.capacity,
                next_seq: 1,
            }),
        })
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
        self.stats().bounds
    }

    fn stats(&self) -> StoreStats {
        let state = self.read_state();
        let bounds = match (state.entries.front(), state.entries.back()) {
            (Some(first), Some(last)) => (first.seq, last.seq),
            _ => (0, 0),
        };

        StoreStats {
            retained: state.entries.len(),
            capacity: state.capacity,
            bounds,
        }
    }

    fn fetch_tail_filtered(
        &self,
        sources: &[String],
        limit: usize,
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError> {
        if limit == 0 {
            return Ok(());
        }

        let state = self.read_state();
        let match_all = sources.is_empty();

        // Walk newest -> oldest under a single read lock, collecting up to
        // `limit` matches, then reverse for ascending order on append.
        let mut collected: Vec<Arc<LogEntry>> = Vec::with_capacity(limit);
        for entry in state.entries.iter().rev() {
            if match_all || sources.iter().any(|s| s == &entry.source.id) {
                collected.push(entry.clone());
                if collected.len() == limit {
                    break;
                }
            }
        }
        out.extend(collected.into_iter().rev());

        Ok(())
    }

    fn fetch_requested(
        &self,
        requested: &[u64],
        out: &mut Vec<Arc<LogEntry>>,
    ) -> Result<(), FmlError> {
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

    fn populate(store: &Arc<dyn LogStore>, entries: &[(&str, &str)]) {
        for (msg, source_id) in entries {
            store.insert(make_entry(msg, source_id));
        }
    }

    // --- bounds ---

    #[test]
    fn empty_store_has_zero_bounds() {
        let store = RingBufferStore::new(test_config(8));
        assert_eq!(store.bounds(), (0, 0));
    }

    #[test]
    fn empty_store_reports_stats() {
        let store = RingBufferStore::new(test_config(8));

        assert_eq!(
            store.stats(),
            StoreStats {
                retained: 0,
                capacity: 8,
                bounds: (0, 0),
            }
        );
    }

    #[test]
    fn bounds_reflect_inserted_entries() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test"), ("c", "test")]);
        assert_eq!(store.bounds(), (1, 3));
    }

    #[test]
    fn partially_filled_store_reports_stats() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test"), ("c", "test")]);

        assert_eq!(
            store.stats(),
            StoreStats {
                retained: 3,
                capacity: 8,
                bounds: (1, 3),
            }
        );
    }

    #[test]
    fn bounds_advance_after_eviction() {
        let store = RingBufferStore::new(test_config(3));
        // Insert 5 entries into a capacity-3 buffer: entries 1,2 get evicted
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        let (lo, hi) = store.bounds();
        assert_eq!(lo, 3);
        assert_eq!(hi, 5);
    }

    #[test]
    fn full_store_reports_capacity_and_shifted_bounds() {
        let store = RingBufferStore::new(test_config(3));
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        assert_eq!(
            store.stats(),
            StoreStats {
                retained: 3,
                capacity: 3,
                bounds: (3, 5),
            }
        );
    }

    // --- clear ---

    #[test]
    fn clear_empties_the_store() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test")]);

        store.clear().unwrap();
        assert_eq!(store.bounds(), (0, 0));
    }

    // --- fetch_range ---

    #[test]
    fn fetch_range_returns_all_entries() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test"), ("c", "test")]);

        let mut out = Vec::new();
        store.fetch_range(1, 3, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "a");
        assert_eq!(out[1].msg, "b");
        assert_eq!(out[2].msg, "c");
    }

    #[test]
    fn fetch_range_subset() {
        let store = RingBufferStore::new(test_config(8));
        populate(
            &store,
            &[("a", "test"), ("b", "test"), ("c", "test"), ("d", "test")],
        );

        let mut out = Vec::new();
        store.fetch_range(2, 3, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].msg, "b");
        assert_eq!(out[1].msg, "c");
    }

    #[test]
    fn fetch_range_clamps_to_retained() {
        let store = RingBufferStore::new(test_config(3));
        // Insert 5 entries, only seq 3,4,5 remain
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        // Request range 1..5 — should clamp to 3..5
        let mut out = Vec::new();
        store.fetch_range(1, 5, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "c");
        assert_eq!(out[1].msg, "d");
        assert_eq!(out[2].msg, "e");
    }

    #[test]
    fn fetch_range_entirely_outside_returns_empty() {
        let store = RingBufferStore::new(test_config(3));
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        // Request range that was evicted
        let mut out = Vec::new();
        store.fetch_range(1, 2, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_range_inverted_bounds_returns_empty() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test")]);

        let mut out = Vec::new();
        store.fetch_range(5, 1, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_range_empty_store_returns_empty() {
        let store = RingBufferStore::new(test_config(8));

        let mut out = Vec::new();
        store.fetch_range(1, 10, &mut out).unwrap();
        assert!(out.is_empty());
    }

    // --- fetch_requested ---

    #[test]
    fn fetch_requested_returns_entries_by_seq() {
        let store = RingBufferStore::new(test_config(8));
        populate(
            &store,
            &[("a", "test"), ("b", "test"), ("c", "test"), ("d", "test")],
        );

        let mut out = Vec::new();
        store.fetch_requested(&[2, 4], &mut out).unwrap();
        assert_eq!(out.len(), 2);
        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert!(seqs.contains(&2));
        assert!(seqs.contains(&4));
    }

    #[test]
    fn fetch_requested_skips_evicted_seqs() {
        let store = RingBufferStore::new(test_config(3));
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        // Seq 1 and 2 were evicted — requesting them should filter them out
        let mut out = Vec::new();
        store.fetch_requested(&[1, 2, 4], &mut out).unwrap();
        // Only seq 4 is in range (3..5)
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, 4);
    }

    #[test]
    fn fetch_requested_empty_store() {
        let store = RingBufferStore::new(test_config(8));

        let mut out = Vec::new();
        store.fetch_requested(&[1, 2, 3], &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_requested_empty_request() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "test"), ("b", "test")]);

        let mut out = Vec::new();
        store.fetch_requested(&[], &mut out).unwrap();
        assert!(out.is_empty());
    }

    // --- fetch_tail_filtered ---

    #[test]
    fn fetch_tail_filtered_empty_store_returns_empty() {
        let store = RingBufferStore::new(test_config(8));

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["s1".to_string()], 10, &mut out)
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_tail_filtered_no_matches_returns_empty() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "hot"), ("b", "hot"), ("c", "hot")]);

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["slow".to_string()], 10, &mut out)
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_tail_filtered_fewer_matches_than_limit_returns_all_ascending() {
        let store = RingBufferStore::new(test_config(16));
        populate(
            &store,
            &[
                ("a", "hot"),
                ("b", "slow"),
                ("c", "hot"),
                ("d", "slow"),
                ("e", "hot"),
            ],
        );

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["slow".to_string()], 10, &mut out)
            .unwrap();

        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 4]);
    }

    #[test]
    fn fetch_tail_filtered_more_matches_returns_last_n_ascending() {
        // Bug-proving case: hot dominates ingestion, slow is sparse but plentiful
        // in absolute terms. Asking for last 3 slow entries must walk back past
        // many hot entries to fill the window.
        let store = RingBufferStore::new(test_config(64));
        let mut entries: Vec<(&str, &str)> = Vec::new();
        // Interleave 1 slow per 10 hot, 5 slow entries total
        for i in 0..5 {
            for _ in 0..10 {
                entries.push(("h", "hot"));
            }
            // Stamp the slow entry's message with its index so we can assert ordering
            entries.push(match i {
                0 => ("s0", "slow"),
                1 => ("s1", "slow"),
                2 => ("s2", "slow"),
                3 => ("s3", "slow"),
                _ => ("s4", "slow"),
            });
        }
        populate(&store, &entries);

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["slow".to_string()], 3, &mut out)
            .unwrap();

        let msgs: Vec<&str> = out.iter().map(|e| e.msg.as_str()).collect();
        assert_eq!(msgs, vec!["s2", "s3", "s4"]);
        for w in out.windows(2) {
            assert!(w[0].seq < w[1].seq, "results must be ascending by seq");
        }
    }

    #[test]
    fn fetch_tail_filtered_limit_zero_returns_empty() {
        let store = RingBufferStore::new(test_config(8));
        populate(&store, &[("a", "slow"), ("b", "slow")]);

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["slow".to_string()], 0, &mut out)
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fetch_tail_filtered_empty_sources_returns_last_n_overall() {
        // Unfiltered behavior: empty sources slice matches any entry.
        let store = RingBufferStore::new(test_config(16));
        populate(
            &store,
            &[("a", "x"), ("b", "y"), ("c", "x"), ("d", "y"), ("e", "x")],
        );

        let mut out = Vec::new();
        store.fetch_tail_filtered(&[], 3, &mut out).unwrap();

        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![3, 4, 5]);
    }

    #[test]
    fn fetch_tail_filtered_multiple_sources_union() {
        let store = RingBufferStore::new(test_config(16));
        populate(
            &store,
            &[
                ("a", "hot"),
                ("b", "slow1"),
                ("c", "hot"),
                ("d", "slow2"),
                ("e", "hot"),
                ("f", "slow1"),
            ],
        );

        let mut out = Vec::new();
        store
            .fetch_tail_filtered(&["slow1".to_string(), "slow2".to_string()], 10, &mut out)
            .unwrap();

        let seqs: Vec<u64> = out.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 4, 6]);
    }

    // --- sequence numbering ---

    #[test]
    fn sequence_ids_are_monotonic() {
        let store = RingBufferStore::new(test_config(8));
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        let mut out = Vec::new();
        store.fetch_range(1, 5, &mut out).unwrap();
        for w in out.windows(2) {
            assert_eq!(w[1].seq, w[0].seq + 1, "sequence ids must be consecutive");
        }
    }

    #[test]
    fn insert_returns_assigned_seq() {
        let store = RingBufferStore::new(test_config(8));
        assert_eq!(store.insert(make_entry("a", "test")), 1);
        assert_eq!(store.insert(make_entry("b", "test")), 2);
        assert_eq!(store.insert(make_entry("c", "test")), 3);
    }

    // --- eviction ---

    #[test]
    fn eviction_preserves_newest_entries() {
        let store = RingBufferStore::new(test_config(3));
        populate(
            &store,
            &[
                ("a", "test"),
                ("b", "test"),
                ("c", "test"),
                ("d", "test"),
                ("e", "test"),
            ],
        );

        let mut out = Vec::new();
        let (lo, hi) = store.bounds();
        store.fetch_range(lo, hi, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].msg, "c");
        assert_eq!(out[1].msg, "d");
        assert_eq!(out[2].msg, "e");
    }
}
