use std::sync::Arc;

use crate::log::LogEntry;

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollMode {
    Tail,
    History,
    Search,
}

pub struct LogPaneState {
    pub mode: ScrollMode,
    /// Indices into the backing buffer for the current search results, ranked best-last.
    pub search_results: Vec<usize>,
    /// Resolved log entries for the current display window, in render order.
    pub items: Vec<Arc<LogEntry>>,
    pub height: usize,
}

impl Default for LogPaneState {
    fn default() -> Self {
        LogPaneState {
            mode: ScrollMode::Tail,
            search_results: Vec::new(),
            items: Vec::new(),
            height: 0,
        }
    }
}
