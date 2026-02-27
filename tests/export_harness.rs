#![allow(unused)]
//! Export layer integration harness.
//!
//! # What this covers
//!
//! - **3 formats × 4 scopes = 12 combinations**, each snapshot-tested with
//!   `insta` so unintentional format changes are caught.
//! - **Formats**: `raw` (plain newline-delimited log lines), `jsonl`
//!   (one JSON object per line), `csv` (key-value fields as columns).
//! - **Scopes**:
//!   - `entire_store` — all entries in the ring buffer
//!   - `active_filter` — entries matching the current query
//!   - `selected_producer` — entries from one producer
//!   - `selected_producer_with_filter` — entries from one producer matching query
//! - **UI responsiveness**: export runs in the background; the test verifies the
//!   UI (simulated by checking the channel) remains unblocked during export.
//! - **Editor invocation**: after export to temp file, the configured editor
//!   command is invoked with the file path.
//! - **Empty export**: exporting an empty store or an empty filter result must
//!   produce an empty file (not panic).
//!
//! # What this does NOT cover
//!
//! - Network export destinations
//! - Export progress reporting in the UI
//!
//! # Running
//!
//! ```sh
//! cargo test --test export_harness
//! # Update snapshots after intentional format changes:
//! cargo insta review
//! ```

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Format: raw
// ---------------------------------------------------------------------------

/// Raw export of entire store produces one line per entry, matching the
/// original `LogEntry::raw` field.
#[test]
#[ignore = "not yet implemented"]
fn raw_entire_store() {
    todo!("export entire store as raw; snapshot output; assert line count == store.len()")
}

/// Raw export of active filter produces only matching entries.
#[test]
#[ignore = "not yet implemented"]
fn raw_active_filter() {
    todo!("apply error filter; export as raw; snapshot; assert only error lines present")
}

/// Raw export scoped to a single producer produces only that producer's lines.
#[test]
#[ignore = "not yet implemented"]
fn raw_selected_producer() {
    todo!("export producer=api-7f9b4d as raw; snapshot; assert all lines from that producer")
}

/// Raw export scoped to selected producer + active filter.
#[test]
#[ignore = "not yet implemented"]
fn raw_selected_producer_with_filter() {
    todo!("filter errors from one producer; export; snapshot")
}

// ---------------------------------------------------------------------------
// Format: jsonl
// ---------------------------------------------------------------------------

/// JSONL export of entire store produces valid JSON objects, one per line.
#[test]
#[ignore = "not yet implemented"]
fn jsonl_entire_store() {
    todo!("export as jsonl; parse each line as JSON; snapshot; assert all fields present")
}

/// JSONL export of active filter.
#[test]
#[ignore = "not yet implemented"]
fn jsonl_active_filter() {
    todo!("apply filter; export as jsonl; snapshot")
}

/// JSONL export of selected producer.
#[test]
#[ignore = "not yet implemented"]
fn jsonl_selected_producer() {
    todo!("export producer as jsonl; snapshot")
}

/// JSONL export of selected producer + active filter.
#[test]
#[ignore = "not yet implemented"]
fn jsonl_selected_producer_with_filter() {
    todo!("export with both scope and filter as jsonl; snapshot")
}

// ---------------------------------------------------------------------------
// Format: csv
// ---------------------------------------------------------------------------

/// CSV export produces a header row followed by one data row per entry.
/// All key-value fields from `LogEntry::fields` become columns.
#[test]
#[ignore = "not yet implemented"]
fn csv_entire_store() {
    todo!("export as csv; parse; snapshot; assert header + data rows")
}

/// CSV export of active filter.
#[test]
#[ignore = "not yet implemented"]
fn csv_active_filter() {
    todo!("apply filter; export as csv; snapshot")
}

/// CSV export of selected producer.
#[test]
#[ignore = "not yet implemented"]
fn csv_selected_producer() {
    todo!("export producer as csv; snapshot")
}

/// CSV export of selected producer + active filter.
#[test]
#[ignore = "not yet implemented"]
fn csv_selected_producer_with_filter() {
    todo!("export with scope and filter as csv; snapshot")
}

// ---------------------------------------------------------------------------
// UI responsiveness
// ---------------------------------------------------------------------------

/// The UI channel must not be blocked while a large export (100 000 entries) is
/// in progress. This simulates that export runs on a background task.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn export_does_not_block_ui_channel() {
    todo!("start export of 100k entries in background; assert UI channel remains responsive")
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Exporting an empty store produces an empty file (not a panic).
#[test]
#[ignore = "not yet implemented"]
fn empty_store_produces_empty_file() {
    todo!("export empty store; assert output file is empty")
}

/// Exporting with a filter that matches nothing produces an empty file.
#[test]
#[ignore = "not yet implemented"]
fn empty_filter_result_produces_empty_file() {
    todo!("apply non-matching filter; export; assert empty output")
}
