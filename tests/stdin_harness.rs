#![allow(unused)]
//! Stdin ingestor integration harness.
//!
//! # What this covers
//!
//! - **Finite input**: when stdin closes (EOF), the ingestor must terminate
//!   cleanly and the store must contain exactly the lines that were sent.
//! - **Burst**: a large number of lines sent in rapid succession must all arrive
//!   in the store without loss.
//! - **Headless exit**: in `--headless` mode with `--tail N`, the process must
//!   exit with code 0 after printing exactly N lines (or fewer if the store
//!   has fewer).
//! - **Empty input**: an immediate EOF must result in an empty store and a
//!   clean exit.
//!
//! # What this does NOT cover
//!
//! - Binary input (fml is UTF-8; non-UTF-8 is lossily converted)
//! - Interactive terminal input (use the TUI harness)
//!
//! # Running
//!
//! ```sh
//! cargo test --test stdin_harness
//! ```

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Finite input
// ---------------------------------------------------------------------------

/// When stdin reaches EOF, the ingestor terminates and the store contains
/// exactly the lines that were provided.
#[test]
#[ignore = "not yet implemented"]
fn eof_terminates_ingestor() {
    todo!("pipe finite bytes through fake stdin; assert store has exact line count on EOF")
}

/// Empty stdin (immediate EOF) produces an empty store and a clean exit.
#[test]
#[ignore = "not yet implemented"]
fn empty_stdin_produces_empty_store() {
    todo!("send EOF immediately; assert store.len() == 0")
}

/// A single line on stdin is ingested correctly.
#[test]
#[ignore = "not yet implemented"]
fn single_line_is_ingested() {
    todo!("pipe one line; assert store.len() == 1 and content matches")
}

// ---------------------------------------------------------------------------
// Burst
// ---------------------------------------------------------------------------

/// 10 000 lines sent in a single burst are all received in the store.
#[test]
#[ignore = "not yet implemented"]
fn burst_of_10k_lines_all_received() {
    todo!("pipe 10k lines rapidly; assert store.len() == 10_000")
}

/// Lines are received in the order they were sent (no reordering).
#[test]
#[ignore = "not yet implemented"]
fn lines_are_ordered() {
    todo!("pipe numbered lines; assert entries are in sequence order")
}

// ---------------------------------------------------------------------------
// Headless mode
// ---------------------------------------------------------------------------

/// In headless mode with --tail 5, exactly 5 lines are printed to stdout.
#[test]
#[ignore = "not yet implemented"]
fn headless_tail_n_prints_exactly_n_lines() {
    todo!("run fml --headless --tail 5 with 100 lines on stdin; count stdout lines")
}

/// In headless mode, the process exits with code 0 after printing tail output.
#[test]
#[ignore = "not yet implemented"]
fn headless_exits_cleanly() {
    todo!("run fml --headless; assert exit code 0")
}

/// In headless mode with --tail N where N > store size, all available lines are
/// printed (no panic, no hanging).
#[test]
#[ignore = "not yet implemented"]
fn headless_tail_larger_than_store_prints_all() {
    todo!("pipe 3 lines; --tail 100; assert 3 lines in output")
}

// ---------------------------------------------------------------------------
// Producer
// ---------------------------------------------------------------------------

/// The stdin ingestor sets producer to an empty string or a configurable name,
/// never to a pod/container name.
#[test]
#[ignore = "not yet implemented"]
fn stdin_producer_field_is_set() {
    todo!("ingest via stdin; assert entry.source == FeedKind::Stdin")
}
