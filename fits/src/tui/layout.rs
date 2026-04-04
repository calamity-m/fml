use std::collections::HashMap;

use ratatui::layout::{Constraint, Layout, Rect};

/// A named region of the terminal layout that a widget can target.
///
/// Also used as the focus target — [`Slot::focusable()`] returns the ordered
/// list of slots that can receive keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// Primary content area.
    Main,
    /// Single-line query/search input below the main content area.
    QueryBox,
    /// Upper sidebar pane for detailed information about the selected row.
    InfoPane,
    /// Lower sidebar pane for preview content.
    PreviewPane,
    /// Bottom row — keyboard hints, status text, etc.
    StatusBar,
}

impl Slot {
    /// Ordered list of slots that can receive keyboard focus.
    /// Tab cycles through these in order.
    pub fn focusable() -> &'static [Slot] {
        &[Slot::QueryBox, Slot::Main]
    }
}
