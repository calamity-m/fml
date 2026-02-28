//! fml-feeds — log feed source adapters for fml.
//!
//! Each feed adapter connects to a log source, reads raw bytes, and pushes
//! normalised [`fml_core::LogEntry`] structs onto an async channel for the store.

pub mod docker;
pub mod file;
pub mod kubernetes;
pub mod stdin;

/// Trait implemented by each log feed source.
/// Full definition comes in Phase 4.
pub trait FeedHandle: Send + Sync {}
