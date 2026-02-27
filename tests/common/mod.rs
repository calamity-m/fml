//! Shared test utilities for fml integration harnesses.
//!
//! Import everything you need via `mod common; use common::*;` at the top of
//! each harness file. All helpers are designed to be zero-allocation where
//! possible and deterministic with `tokio::time::pause()`.

pub mod assertions;
pub mod builders;
pub mod fake_docker_api;
pub mod fake_process;
pub mod fixtures;

pub use assertions::*;
pub use builders::*;
pub use fixtures::*;
