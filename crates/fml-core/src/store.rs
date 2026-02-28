//! Store — in-memory ring buffer of [`LogEntry`](crate::LogEntry) values with indexed metadata.
//!
//! The store is the single source of truth; the UI reads from it, never from the feed directly.
