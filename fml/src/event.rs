//! Internal event types passed between the app loop, TUI, and search work.
//!
//! These values are sent over channels to keep terminal input, rendering,
//! shutdown, and background search processing loosely coupled.

use crate::log::{NewLogEntry, Source, SourceId};

/// Marker event used to request application shutdown.
///
/// The quit channel carries this zero-sized payload when the app should exit.
pub struct QuitEvent {}

/// Events produced by the terminal loop and consumed by the TUI state machine.
#[derive(Debug)]
pub enum TuiEvent {
    /// A render tick has been requested.
    Render,
    /// The terminal gained focus.
    FocusGained,
    /// The terminal lost focus.
    FocusLost,
    /// A mouse action occurred.
    Mouse(crossterm::event::MouseEvent),
    /// The terminal was resized to `(columns, rows)`.
    Resize(u16, u16),
    /// Text was pasted from the clipboard.
    Paste(String),
    /// A user input key event was received.
    Input(crossterm::event::KeyEvent),
    /// An error occurred in the event stream.
    Error(String),
}

/// A single field match within a search result.
///
/// Stores the field key plus the character offsets of the matched span.
#[derive(Debug, Clone)]
pub struct Match {
    /// Field name that contains the matched text.
    pub key: String,
    /// Character indice highlights.
    pub indices: Vec<u32>,
}

/// A search result for one stored log entry.
///
/// The hit is keyed by the entry sequence id and may contain multiple field
/// matches for that entry.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Sequence id of the matching log entry.
    pub seq_id: u64,
    /// Individual field matches found within the entry.
    pub matches: Vec<Match>,
}

/// Progress through a finite background search scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchProgress {
    /// Number of candidate entries inspected so far.
    pub scanned: usize,
    /// Total candidate entries in the current scan snapshot.
    pub total: usize,
}

/// Exact key/value predicate used by field-matched searches.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FieldPredicate {
    /// Field key to compare.
    pub key: String,
    /// JSON value that must match exactly.
    pub value: serde_json::Value,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Query {
    Tail,
    History {
        middle_seq_id: u64,
        buffer: u64,
    },
    Surrounding {
        middle_seq_id: u64,
        buffer: u64,
    },
    /// Query for entries whose fields exactly match all predicates.
    FieldMatched {
        anchor_seq_id: u64,
        buffer: u64,
        predicates: Vec<FieldPredicate>,
    },
    /// Frozen timestamp-window query centered on an anchor entry.
    TimeWindow {
        anchor_seq_id: u64,
        /// Timestamp retained independently of the ring-buffer anchor entry.
        anchor_ts: chrono::DateTime<chrono::Utc>,
        /// Half-width of the timestamp window in seconds.
        window_secs: u64,
        /// Snapshot bound captured at dispatch.
        until_seq: u64,
        /// Optional exact field predicates applied to every candidate.
        predicates: Vec<FieldPredicate>,
    },
    /// Fuzzy text search. `until_seq` bounds the scan: `None` is a live
    /// query that keeps re-ranking new entries forever; `Some(seq)` is a
    /// frozen snapshot that scores only entries with `seq <= until_seq`,
    /// emits `complete = true` once, and then stops. The bound is part of
    /// the query identity — same term with different bounds are distinct
    /// queries and must not accept each other's emissions.
    Fuzzy {
        term: String,
        until_seq: Option<u64>,
    },
}

/// Identity of a single workspace pane.
///
/// Ids are allocated monotonically by the workspace and never reused within
/// a run, so a stale search result addressed to a closed pane can simply be
/// dropped during routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

impl std::fmt::Display for PaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Routing key identifying which search engine instance a [`SearchEvent`]
/// belongs to.
///
/// Every pane owns exactly one search engine slot, so the routing key *is*
/// the pane id. Engines are independent: starting or cancelling work for one
/// pane has no effect on any other. Within a single pane, a new `Search`
/// cancels that pane's previous in-flight query (one query at a time per
/// engine), and every [`SearchEvent::Result`] is attributed back to the pane
/// that requested it.
pub type SearchTarget = PaneId;

/// Messages exchanged with the search subsystem.
#[derive(Debug)]
pub enum SearchEvent {
    /// Request execution of a search query, optionally restricted to sources.
    Search {
        target: SearchTarget,
        query: Query,
        sources: Vec<SourceId>,
    },
    /// Explicitly cancel in-flight search work for one target.
    ///
    /// Starting a new search for the same target also cancels that target's
    /// previous worker. Use this event when a target should become inactive
    /// without immediately replacing its search.
    Cancel { target: SearchTarget },
    /// Completed search results for a query.
    Result {
        target: SearchTarget,
        query: Query,
        /// Hits to display, capped at the configured result limit and
        /// carrying highlight indices.
        results: Vec<SearchHit>,
        /// Every matching seq id (ascending, uncapped) for fuzzy queries,
        /// so hit navigation can walk matches that aren't displayed.
        /// `None` for queries where `results` already is the full set.
        hit_seqs: Option<Vec<u64>>,
        request_id: u64,
        complete: bool,
        progress: Option<SearchProgress>,
    },
    /// An error occurred while executing a search request.
    Error(String),
}

#[derive(Debug)]
pub enum ProducerEvent {
    SourceFound(Source),
    SourceLost(SourceId),
    StoreEvent(NewLogEntry),
}
