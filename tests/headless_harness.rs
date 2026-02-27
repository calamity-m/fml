#![allow(unused)]
//! Headless mode process-level integration harness.
//!
//! # What this covers
//!
//! This harness exercises `fml` as a compiled binary via
//! [`std::process::Command`]. It validates the contract of headless mode from
//! the outside — what a user or another CLI tool would observe.
//!
//! - **All headless flags**: `--headless`, `--query`, `--greed`, `--tail`,
//!   `--duration`, `--format`, `--no-metadata`.
//! - **Exit codes**: clean exit = 0; bad flags = non-zero; feed error = non-zero.
//! - **TTY detection**: when stdout is a pipe, colourisation is suppressed.
//!   When stdout is a TTY (simulated with a pty), ANSI codes are present.
//! - **Output format validation**: raw, jsonl, and csv output is parsed and
//!   validated against the known input.
//! - **Duration flag**: `--duration 100ms` must cause the process to exit
//!   within a bounded time window.
//!
//! # What this does NOT cover
//!
//! - TUI rendering (that requires a real terminal)
//! - MCP server mode (separate harness)
//!
//! # Running
//!
//! ```sh
//! cargo test --test headless_harness
//! # These tests compile and run the fml binary, so they require:
//! cargo build  # build the binary first
//! cargo test --test headless_harness
//! ```

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use std::process::{Command, Stdio};

fn fml_binary() -> Command {
    // Use the debug build. In CI, cargo test builds it automatically.
    let binary = env!("CARGO_BIN_EXE_fml");
    Command::new(binary)
}

// ---------------------------------------------------------------------------
// Basic headless operation
// ---------------------------------------------------------------------------

/// `fml --headless --feed stdin` with lines on stdin exits with code 0.
#[test]
#[ignore = "not yet implemented"]
fn headless_stdin_exits_zero() {
    todo!("pipe lines to fml --headless --feed stdin; assert exit code 0")
}

/// `fml --headless --feed stdin --tail 5` with 100 lines on stdin prints
/// exactly 5 lines to stdout.
#[test]
#[ignore = "not yet implemented"]
fn headless_tail_n_lines() {
    todo!("pipe 100 lines; --tail 5; count stdout lines; assert == 5")
}

/// `fml --headless` with no feed flag prints an error and exits non-zero.
#[test]
#[ignore = "not yet implemented"]
fn headless_no_feed_exits_nonzero() {
    todo!("run fml --headless with no --feed; assert exit code != 0")
}

// ---------------------------------------------------------------------------
// Query and greed flags
// ---------------------------------------------------------------------------

/// `--query level:error` filters output to error lines only.
#[test]
#[ignore = "not yet implemented"]
fn headless_query_filters_output() {
    todo!("pipe mixed-level lines; --query level:error; assert only ERROR lines in output")
}

/// `--greed 7` enables semantic expansion (more lines matched than greed 0).
#[test]
#[ignore = "not yet implemented"]
fn headless_greed_flag_enables_expansion() {
    todo!("pipe lines with synonyms; --greed 7 vs --greed 0; assert greed 7 returns more lines")
}

// ---------------------------------------------------------------------------
// Output format flags
// ---------------------------------------------------------------------------

/// `--format jsonl` produces valid JSON objects, one per line.
#[test]
#[ignore = "not yet implemented"]
fn headless_format_jsonl_is_valid() {
    todo!("run with --format jsonl; parse every output line as JSON; assert no parse errors")
}

/// `--format csv` produces a CSV with a header row.
#[test]
#[ignore = "not yet implemented"]
fn headless_format_csv_has_header() {
    todo!("run with --format csv; assert first line is CSV header")
}

/// `--format raw` (default) produces plain text lines.
#[test]
#[ignore = "not yet implemented"]
fn headless_format_raw_is_plain_text() {
    todo!("run with --format raw; assert output has no JSON delimiters or CSV commas")
}

// ---------------------------------------------------------------------------
// Metadata flags
// ---------------------------------------------------------------------------

/// `--no-metadata` suppresses the injected `source`, `producer`, and `ts` fields
/// in jsonl output.
#[test]
#[ignore = "not yet implemented"]
fn headless_no_metadata_suppresses_synthetic_fields() {
    todo!("run --format jsonl --no-metadata; parse output; assert no source/producer/ts keys")
}

// ---------------------------------------------------------------------------
// Duration flag
// ---------------------------------------------------------------------------

/// `--duration 100ms` causes the process to exit within a bounded window
/// (between 50ms and 500ms, to allow for CI timing variance).
#[test]
#[ignore = "not yet implemented"]
fn headless_duration_flag_causes_exit() {
    todo!("run fml --headless --feed stdin --duration 100ms with infinite stdin; assert exits within 500ms")
}

// ---------------------------------------------------------------------------
// TTY detection
// ---------------------------------------------------------------------------

/// When stdout is a pipe (not a TTY), no ANSI escape codes appear in output.
#[test]
#[ignore = "not yet implemented"]
fn headless_piped_output_has_no_ansi_codes() {
    todo!("pipe fml output through cat; assert no ESC[ sequences in output bytes")
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

/// Unknown flag returns exit code 2 (standard convention for CLI usage errors).
#[test]
#[ignore = "not yet implemented"]
fn unknown_flag_exits_with_code_2() {
    todo!("run fml --unknown-flag; assert exit code 2")
}
