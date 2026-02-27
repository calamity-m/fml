#![allow(unused)]
//! File ingestor integration harness.
//!
//! # What this covers
//!
//! - **Rotation**: when a log file is rotated (old file renamed, new file created
//!   at the original path), the ingestor must detect the new inode and continue
//!   tailing without missing lines or duplicating lines across the rotation.
//! - **Truncation**: when a file is truncated (e.g. `> file.log`), the ingestor
//!   must seek to offset 0 and re-read from the beginning.
//! - **Glob expansion**: `paths` config with glob patterns like `~/logs/**/*.log`
//!   must discover matching files and tail all of them simultaneously.
//! - **Property: all written lines received**: for any sequence of writes,
//!   all lines written to a file before it is closed must appear in the store.
//!   Verified with proptest over random line content and write batch sizes.
//!
//! # What this does NOT cover
//!
//! - NFS / network filesystem tail (undefined inotify behaviour)
//! - Files larger than available memory
//!
//! # Running
//!
//! ```sh
//! cargo test --test file_harness
//! ```

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Basic tailing
// ---------------------------------------------------------------------------

/// Lines written to a file after the ingestor starts must appear in the store.
#[test]
#[ignore = "not yet implemented"]
fn appended_lines_are_ingested() {
    todo!("write lines to tempfile; assert all arrive in store in order")
}

/// Lines already in the file when the ingestor starts (backfill) must be
/// ingested before new lines.
#[test]
#[ignore = "not yet implemented"]
fn existing_lines_are_backfilled() {
    todo!("pre-populate tempfile; start ingestor; assert existing lines arrive first")
}

/// Producer name for file ingestor is the file path (or basename, per config).
#[test]
#[ignore = "not yet implemented"]
fn producer_is_file_path() {
    todo!("ingest a file; assert entry.producer matches the file path")
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

/// When the file at the watched path is renamed and a new file is created,
/// the ingestor must switch to the new file and continue tailing.
#[test]
#[ignore = "not yet implemented"]
fn file_rotation_is_detected() {
    todo!("rename tempfile; create new file at same path; assert ingestor follows new inode")
}

/// Lines written to the old file after rotation but before the ingestor
/// detects the rotation must still arrive (drain the old file before switching).
#[test]
#[ignore = "not yet implemented"]
fn old_file_is_drained_before_rotation() {
    todo!("write to old file post-rotation; assert those lines arrive before new-file lines")
}

/// No lines are duplicated across a file rotation.
#[test]
#[ignore = "not yet implemented"]
fn no_duplicates_across_rotation() {
    todo!("count total lines before and after rotation; assert no duplicates in store")
}

// ---------------------------------------------------------------------------
// Truncation
// ---------------------------------------------------------------------------

/// When a file is truncated (size drops below current read offset), the
/// ingestor must seek to 0 and re-read.
#[test]
#[ignore = "not yet implemented"]
fn truncated_file_is_re_read_from_start() {
    todo!("truncate tempfile; write new content; assert new content arrives from offset 0")
}

// ---------------------------------------------------------------------------
// Glob
// ---------------------------------------------------------------------------

/// A glob pattern that matches multiple files tails all of them simultaneously.
#[test]
#[ignore = "not yet implemented"]
fn glob_matches_multiple_files() {
    todo!("create 3 tempfiles matching glob; assert all 3 produce entries")
}

/// A file created after the ingestor starts that matches the glob pattern is
/// automatically picked up.
#[test]
#[ignore = "not yet implemented"]
fn glob_discovers_new_files() {
    todo!("create a matching file after ingestor starts; assert it is discovered")
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

/// Property: for any sequence of line writes, all lines written to a file
/// before it is closed appear in the store exactly once.
#[test]
#[ignore = "not yet implemented"]
fn prop_all_written_lines_received() {
    todo!("proptest: random lines and write batches; assert store contains all lines exactly once")
}
