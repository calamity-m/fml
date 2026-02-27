#![allow(unused)]
//! Docker ingestor integration harness.
//!
//! # What this covers
//!
//! - **Multiplexed frame decoding**: the Docker log API wraps stdout and stderr in
//!   an 8-byte frame header (`[stream_type, 0, 0, 0, size_big_endian_u32]`).
//!   The ingestor must decode these frames correctly.
//! - **Compose project naming**: containers with the
//!   `com.docker.compose.project` label should show a two-level tree
//!   (`project/service`). Unlabelled containers appear at the top level.
//! - **stderr tagging**: lines from stderr (frame type = 2) must carry a
//!   synthetic field `stream=stderr`; stdout lines get `stream=stdout`.
//! - **Container exit and removal**: when a container exits or is removed, the
//!   log stream closes gracefully without panicking.
//!
//! # What this does NOT cover
//!
//! - Real Docker daemon interaction (uses `FakeDockerApi` Unix socket)
//! - Swarm / overlay network containers
//!
//! # Running
//!
//! ```sh
//! cargo test --test docker_harness
//! ```

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Multiplexed frame decoding
// ---------------------------------------------------------------------------

/// Raw Docker multiplexed frames must be decoded into clean log lines.
/// The 8-byte header must be stripped before the line is stored.
#[test]
#[ignore = "not yet implemented"]
fn multiplexed_frames_are_decoded() {
    todo!("construct raw Docker frame bytes; feed through ingestor; assert header stripped")
}

/// A frame with stream_type=1 (stdout) must tag the entry with `stream=stdout`.
#[test]
#[ignore = "not yet implemented"]
fn stdout_frame_tagged_as_stdout() {
    todo!("send frame with type=1; assert entry.fields[stream] == stdout")
}

/// A frame with stream_type=2 (stderr) must tag the entry with `stream=stderr`.
#[test]
#[ignore = "not yet implemented"]
fn stderr_frame_tagged_as_stderr() {
    todo!("send frame with type=2; assert entry.fields[stream] == stderr")
}

/// Interleaved stdout and stderr frames must be decoded correctly without
/// cross-contamination between streams.
#[test]
#[ignore = "not yet implemented"]
fn interleaved_stdout_stderr_decoded_correctly() {
    todo!("send alternating stdout/stderr frames; assert stream field correct on each")
}

// ---------------------------------------------------------------------------
// Compose project naming
// ---------------------------------------------------------------------------

/// A container with `com.docker.compose.project=myapp` and
/// `com.docker.compose.service=api` must appear under `myapp/api` in the
/// producer tree.
#[test]
#[ignore = "not yet implemented"]
fn compose_container_producer_includes_project_and_service() {
    todo!("add compose-labelled container; assert producer == project/service")
}

/// A container without compose labels must appear at the top level with just
/// its container name.
#[test]
#[ignore = "not yet implemented"]
fn unlabelled_container_producer_is_container_name() {
    todo!("add unlabelled container; assert producer == container name")
}

/// Multiple services within the same compose project are grouped correctly.
#[test]
#[ignore = "not yet implemented"]
fn multiple_compose_services_grouped_under_project() {
    todo!("add api, worker, db containers under same project; verify grouping")
}

// ---------------------------------------------------------------------------
// Container lifecycle
// ---------------------------------------------------------------------------

/// When a container exits, the log stream closes without error.
#[test]
#[ignore = "not yet implemented"]
fn container_exit_closes_stream_cleanly() {
    todo!("close FakeDockerApi stream; assert ingestor shuts down without panic")
}

/// When a container is removed while being tailed, the entry is removed from
/// the producer tree and no further lines arrive for that container.
#[test]
#[ignore = "not yet implemented"]
fn removed_container_disappears_from_producer_tree() {
    todo!("remove container from FakeDockerApi; assert no further entries arrive")
}

// ---------------------------------------------------------------------------
// Fake API integration
// ---------------------------------------------------------------------------

/// /containers/json response is correctly mapped to the available producer list.
#[test]
#[ignore = "not yet implemented"]
fn container_list_populates_producer_tree() {
    todo!("start FakeDockerApi with 3 containers; verify all 3 appear as selectable producers")
}
