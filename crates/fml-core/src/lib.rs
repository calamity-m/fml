//! fml-core — Feed Me Logs core library.
//!
//! This crate exposes the four architectural pipeline layers as public modules,
//! plus the shared types used across all layers.
//!
//! # Architecture
//!
//! ```text
//! Ingestor ──► Store ──► Search ──► UI
//!    │           │
//!    └───────────┴──► Export
//! ```
//!
//! All inter-layer communication uses `tokio` channels. The UI drives the
//! main thread; everything else runs on background tasks.

pub mod config;
pub mod export;
pub mod normalizer;
pub mod search;
pub mod store;
pub mod types;

pub use types::{FeedKind, LogEntry, LogLevel};
