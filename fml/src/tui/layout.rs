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

/// Build the slot-to-area mapping for the given terminal area.
///
/// ```text
/// +-------------+-------------+
/// |             |             |
/// |    Main     |  InfoPane   |
/// |  (Fill 1)   |   (30%)     |
/// |             +-------------+
/// | QueryBox    | PreviewPane |
/// | (Length 3)  |   (70%)     |
/// +-------------+-------------+
/// | StatusBar (Length 1)      |
/// +---------------------------+
/// ```
pub fn build_layout(area: Rect, sidebar_width_percent: u16) -> HashMap<Slot, Rect> {
    let vertical = Layout::vertical([
        Constraint::Fill(1),   // Content (Main/QueryBox + Sidebar)
        Constraint::Length(1), // StatusBar
    ])
    .split(area);

    let content = Layout::horizontal([
        Constraint::Fill(1), // Left column (Main + QueryBox)
        Constraint::Percentage(normalize_sidebar_width_percent(sidebar_width_percent)), // Sidebar
    ])
    .split(vertical[0]);

    let left_column = Layout::vertical([
        Constraint::Fill(1),   // Main
        Constraint::Length(3), // QueryBox
    ])
    .split(content[0]);

    let sidebar = Layout::vertical([
        Constraint::Percentage(30), // InfoPane
        Constraint::Fill(1),        // PreviewPane
    ])
    .split(content[1]);

    HashMap::from([
        (Slot::Main, left_column[0]),
        (Slot::QueryBox, left_column[1]),
        (Slot::InfoPane, sidebar[0]),
        (Slot::PreviewPane, sidebar[1]),
        (Slot::StatusBar, vertical[1]),
    ])
}

fn normalize_sidebar_width_percent(sidebar_width_percent: u16) -> u16 {
    // Keep both panes visible even if the configured value is out of range.
    sidebar_width_percent.clamp(1, 99)
}
