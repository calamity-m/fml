//! Domain-specific assertion macros for fml harnesses.
//!
//! These wrap `pretty_assertions` and add context-rich failure messages that
//! make it clear *what* fml invariant was violated and *where* in the log
//! pipeline the violation occurred.

use fml_core::{FeedKind, LogEntry, LogLevel};

// ---------------------------------------------------------------------------
// Field assertions
// ---------------------------------------------------------------------------

/// Assert that a `LogEntry` has a specific field with an expected value.
///
/// ```rust
/// assert_has_field!(entry, "request_id", "req-abc123");
/// ```
#[macro_export]
macro_rules! assert_has_field {
    ($entry:expr, $key:expr, $value:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        let key: &str = $key;
        let expected = serde_json::json!($value);
        match entry.fields.get(key) {
            Some(actual) if *actual == expected => {}
            Some(actual) => panic!(
                "assert_has_field! failed:\n  entry.fields[{:?}]\n  expected: {}\n  actual:   {}",
                key, expected, actual
            ),
            None => panic!(
                "assert_has_field! failed: field {:?} not found in entry.\n  Available fields: {:?}",
                key,
                entry.fields.keys().collect::<Vec<_>>()
            ),
        }
    }};
}

/// Assert that a `LogEntry` contains a field key (any value).
#[macro_export]
macro_rules! assert_field_exists {
    ($entry:expr, $key:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        let key: &str = $key;
        if !entry.fields.contains_key(key) {
            panic!(
                "assert_field_exists! failed: field {:?} not found.\n  Available: {:?}",
                key,
                entry.fields.keys().collect::<Vec<_>>()
            );
        }
    }};
}

// ---------------------------------------------------------------------------
// Level assertions
// ---------------------------------------------------------------------------

/// Assert that a `LogEntry` has a specific log level.
///
/// ```rust
/// assert_level!(entry, LogLevel::Error);
/// ```
#[macro_export]
macro_rules! assert_level {
    ($entry:expr, $level:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        let expected: fml_core::LogLevel = $level;
        match entry.level {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!(
                "assert_level! failed:\n  expected: {:?}\n  actual:   {:?}\n  raw: {:?}",
                expected, actual, entry.raw
            ),
            None => panic!(
                "assert_level! failed: no level on entry.\n  raw: {:?}",
                entry.raw
            ),
        }
    }};
}

/// Assert that a `LogEntry` has *some* level set (not `None`).
#[macro_export]
macro_rules! assert_has_level {
    ($entry:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        if entry.level.is_none() {
            panic!(
                "assert_has_level! failed: level is None.\n  raw: {:?}",
                entry.raw
            );
        }
    }};
}

// ---------------------------------------------------------------------------
// Producer assertions
// ---------------------------------------------------------------------------

/// Assert that a `LogEntry` came from the expected producer.
///
/// ```rust
/// assert_producer!(entry, "api-7f9b4d");
/// ```
#[macro_export]
macro_rules! assert_producer {
    ($entry:expr, $producer:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        let expected: &str = $producer;
        if entry.producer != expected {
            panic!(
                "assert_producer! failed:\n  expected: {:?}\n  actual:   {:?}",
                expected, entry.producer
            );
        }
    }};
}

/// Assert that a `LogEntry` came from the expected feed source.
#[macro_export]
macro_rules! assert_source {
    ($entry:expr, $source:expr) => {{
        let entry: &fml_core::LogEntry = &$entry;
        let expected: fml_core::FeedKind = $source;
        if entry.source != expected {
            panic!(
                "assert_source! failed:\n  expected: {:?}\n  actual:   {:?}",
                expected, entry.source
            );
        }
    }};
}

// ---------------------------------------------------------------------------
// Store / search result assertions
// ---------------------------------------------------------------------------

/// Assert that a search result set contains at least one entry matching a
/// field predicate.
///
/// ```rust
/// assert_results_contain!(results, |e| e.level == Some(LogLevel::Error));
/// ```
#[macro_export]
macro_rules! assert_results_contain {
    ($results:expr, $pred:expr) => {{
        let results: &[fml_core::LogEntry] = &$results;
        let pred = $pred;
        if !results.iter().any(pred) {
            panic!(
                "assert_results_contain! failed: no entry matched predicate.\n  {} entries checked.",
                results.len()
            );
        }
    }};
}

/// Assert that every entry in a result set satisfies a predicate.
///
/// ```rust
/// assert_results_all!(results, |e| e.level == Some(LogLevel::Error));
/// ```
#[macro_export]
macro_rules! assert_results_all {
    ($results:expr, $pred:expr) => {{
        let results: &[fml_core::LogEntry] = &$results;
        let pred = $pred;
        let failing: Vec<_> = results.iter().filter(|e| !pred(e)).collect();
        if !failing.is_empty() {
            panic!(
                "assert_results_all! failed: {} of {} entries did not satisfy predicate.",
                failing.len(),
                results.len()
            );
        }
    }};
}

// ---------------------------------------------------------------------------
// Normalizer synthetic field invariant helpers
// ---------------------------------------------------------------------------

/// Assert that a normalised `LogEntry` has all mandatory synthetic fields.
///
/// Mandatory fields for every normalised entry: `source`, `producer`, `ts`
/// (as a top-level struct field, not inside `fields`).
pub fn assert_synthetic_fields(entry: &LogEntry) {
    // `source`, `producer`, and `ts` are struct fields, not HashMap entries,
    // so we just check they have non-empty/non-zero values.
    assert!(
        !entry.producer.is_empty(),
        "normalised entry must have a non-empty producer: {:?}",
        entry.raw
    );
    // source is an enum — any variant is valid; check it was set by seeing it
    // doesn't panic (it always exists as an enum field).
    let _ = entry.source;
    // ts is always set — just check it is not the default Unix epoch (which
    // would indicate the normalizer forgot to set it).
    assert!(
        entry.ts.timestamp() > 0,
        "normalised entry ts should not be Unix epoch: {:?}",
        entry.raw
    );
}
