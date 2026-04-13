#[derive(Debug, Clone, PartialEq)]
pub enum ScrollMode {
    Tail,
    History,
    Search,
}

pub struct LogPaneState {
    pub mode: ScrollMode,
    /// Absolute index into the current display list (all entries or search results).
    pub absolute_cursor: usize,
    /// Indices into the backing buffer for the current search results, ranked best-last.
    pub search_results: Vec<usize>,
    pub items: Vec<u64>,
    pub height: usize,
}

impl Default for LogPaneState {
    fn default() -> Self {
        LogPaneState {
            mode: ScrollMode::Tail,
            absolute_cursor: 0,
            search_results: Vec::new(),
            items: Vec::new(),
            height: 0,
        }
    }
}
