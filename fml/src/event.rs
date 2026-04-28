//! Internal event types passed between the app loop, TUI, and search work.
//!
//! These values are sent over channels to keep terminal input, rendering,
//! shutdown, and background search processing loosely coupled.

use std::sync::Arc;

use crate::log::{LogEntry, NewLogEntry, Source, SourceId};

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
    /// A scroll action was requested in the given direction.
    Scroll(ratatui::widgets::ScrollDirection),
    /// A request to jump the scroll position to the start of the buffer.
    ScrollHead,
    /// A request to jump the scroll position to the end of the buffer.
    ScrollTail,
    /// Dispatch a log-pane search after applying the current source filter.
    DispatchLogPaneSearch(Query),
    /// Redispatch the active log-pane search after source-filter changes.
    RedispatchLogPaneSearch,
    /// A user input key event was received.
    Input(crossterm::event::KeyEvent),
    /// An error occurred in the event stream.
    Error(String),
    /// The log pane selected entry changed.
    NewSelectedEntry(Option<SelectedEntry>),
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

/// Snapshot of the log entry currently selected by the log pane.
#[derive(Debug, Clone)]
pub struct SelectedEntry {
    pub entry: Arc<LogEntry>,
    pub matches: Vec<Match>,
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Query {
    Tail,
    History { middle_seq_id: u64, buffer: u64 },
    Surrounding { middle_seq_id: u64, buffer: u64 },
    Fuzzy(String),
}

/// Routing key identifying which search engine instance a [`SearchEvent`]
/// belongs to.
///
/// Each variant corresponds to a distinct, independently-running search engine
/// (one per pane). The reason this is an enum rather than a unit-less event
/// stream is that engines are addressable: every [`SearchEvent::Search`] and
/// [`SearchEvent::Cancel`] must be dispatched to exactly one engine, and every
/// [`SearchEvent::Result`] must be attributable back to the engine that
/// produced it so consumers can update the correct pane.
///
/// Engines are independent: starting or cancelling work for one target has no
/// effect on the other. Within a single target, a new `Search` cancels that
/// target's previous in-flight query (one query at a time per engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchTarget {
    LogPane,
    PreviewPane,
}

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
        results: Vec<SearchHit>,
        request_id: u64,
        complete: bool,
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
