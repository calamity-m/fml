#![allow(unused)]
//! Store layer integration harness.
//!
//! # What this covers
//!
//! - **Ring eviction**: when the store is at capacity, the oldest entry is
//!   evicted to make room for the newest. After N inserts where N > capacity,
//!   `store.len() == capacity`.
//! - **Sequence monotonicity**: every entry has a monotonically increasing
//!   sequence number. No two entries share a sequence number.
//! - **Concurrent reads/writes**: multiple readers and writers operating
//!   concurrently must not deadlock, panic, or corrupt data.
//! - **Property: len == min(n, capacity)**: for any n inserts into a store with
//!   capacity c, `store.len() == min(n, c)`. Verified with proptest.
//! - **Producer filter**: querying by producer returns only entries with that
//!   producer, in sequence order.
//! - **Level filter**: querying by level returns only entries at that level or
//!   above.
//!
//! # What this does NOT cover
//!
//! - Persistence / serialization of the store to disk
//! - Memory-mapped storage
//!
//! # Running
//!
//! ```sh
//! cargo test --test store_harness
//! ```

mod common;
use common::*;
use fml::LogLevel;

// ---------------------------------------------------------------------------
// Ring buffer eviction
// ---------------------------------------------------------------------------

/// Inserting more than `capacity` entries evicts the oldest entries. After
/// `capacity + 1` inserts, `store.len() == capacity` and the first entry is gone.
#[test]
#[ignore = "not yet implemented"]
fn ring_evicts_oldest_on_overflow() {
    todo!("insert capacity+1 entries; assert store.len() == capacity; assert first entry evicted")
}

/// After eviction, the remaining entries are the most recently inserted ones,
/// in insertion order.
#[test]
#[ignore = "not yet implemented"]
fn evicted_entries_are_oldest() {
    todo!("insert capacity+N entries; assert the first N entries are gone")
}

// ---------------------------------------------------------------------------
// Sequence numbers
// ---------------------------------------------------------------------------

/// Every inserted entry has a sequence number greater than the previous entry.
#[test]
#[ignore = "not yet implemented"]
fn sequence_numbers_are_monotonic() {
    todo!("insert 100 entries; assert each entry.seq > previous entry.seq")
}

/// No two entries in the store share a sequence number.
#[test]
#[ignore = "not yet implemented"]
fn sequence_numbers_are_unique() {
    todo!("insert 1000 entries; collect seq numbers into a HashSet; assert len == 1000")
}

// ---------------------------------------------------------------------------
// Concurrent access
// ---------------------------------------------------------------------------

/// Multiple concurrent writers and readers must not deadlock, panic, or
/// corrupt entry data.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn concurrent_reads_and_writes_are_safe() {
    todo!("spawn 5 writer tasks and 5 reader tasks; run for 1s; assert no panic or deadlock")
}

/// A reader that iterates over the store while a writer is inserting must see
/// a consistent snapshot (no partial entries, no torn reads).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn reader_sees_consistent_snapshot() {
    todo!("iterate store during concurrent writes; assert every returned entry is valid")
}

// ---------------------------------------------------------------------------
// Producer filter
// ---------------------------------------------------------------------------

/// `store.by_producer("api-7f9b4d")` returns only entries from that producer.
#[test]
#[ignore = "not yet implemented"]
fn producer_filter_returns_correct_entries() {
    todo!("insert entries from 3 producers; filter by one; assert only that producer's entries returned")
}

/// Producer filter returns entries in sequence order.
#[test]
#[ignore = "not yet implemented"]
fn producer_filter_preserves_order() {
    todo!(
        "insert interleaved entries from 2 producers; filter one; assert sequence order preserved"
    )
}

// ---------------------------------------------------------------------------
// Level filter
// ---------------------------------------------------------------------------

/// Filtering by `LogLevel::Error` returns only ERROR and FATAL entries.
#[test]
#[ignore = "not yet implemented"]
fn level_filter_excludes_lower_levels() {
    todo!("insert entries at all levels; filter >= Error; assert only Error/Fatal returned")
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

/// Property: for any n inserts into a store with capacity c,
/// `store.len() == min(n, c)`.
#[test]
#[ignore = "not yet implemented"]
fn prop_len_equals_min_n_capacity() {
    todo!("proptest: random n and c; assert store.len() == n.min(c) after n inserts")
}

/// Property: producer filter ⊆ all entries (no entries appear that were never
/// inserted for that producer).
#[test]
#[ignore = "not yet implemented"]
fn prop_producer_filter_subset_of_all() {
    todo!("proptest: random entries; assert filtered results are subset of inserted entries")
}
