//! Internal event types passed between the app loop, TUI, and search work.
//!
//! These values are sent over channels to keep terminal input, rendering,
//! shutdown, and background search processing loosely coupled.

use crate::log::SourceId;

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
    /// A user input key event was received.
    Input(crossterm::event::KeyEvent),
    /// An error occurred in the event stream.
    Error(String),
}

/// A single field match within a search result.
///
/// Stores the field key plus the character offsets of the matched span.
#[derive(Debug)]
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
#[derive(Debug)]
pub struct SearchHit {
    /// Sequence id of the matching log entry.
    pub seq_id: u64,
    /// Individual field matches found within the entry.
    pub matches: Vec<Match>,
}

#[derive(Debug)]
pub enum Query {
    Tail,
    History { middle_seq_id: u64, buffer: u64 },
    Fuzzy(String),
}

/// Messages exchanged with the search subsystem.
pub enum SearchEvent {
    /// Request execution of a search query, optionally restricted to sources.
    Search {
        query: Query,
        sources: Vec<SourceId>,
    },
    /// Completed search results for a query.
    Result {
        results: Vec<SearchHit>,
        request_id: u64,
        complete: bool,
    },
    /// An error occurred while executing a search request.
    Error(String),
}
