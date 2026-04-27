use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use tokio::task::JoinHandle;

pub mod log_pane_state;

use log_pane_state::LogPaneState;

use crate::{
    config::{
        search::SearchConfig,
        tui::{ThemeConfig, TuiConfig},
    },
    error::FmlError,
    event::SelectedEntry,
    tui::{layout::Slot, widgets::query_box},
};

pub struct TuiState {
    pub focused: Slot,
    pub areas: HashMap<Slot, Rect>,
    pub selected_theme: ThemeConfig,
    pub query_box_textarea: TextArea<'static>,
    pub query_box_last_dispatched_query: String,
    pub query_box_debounce_handle: Option<JoinHandle<()>>,
    pub fuzzy_debounce_ms: u64,
    /// Selected visible row in the log pane viewport.
    pub log_pane_cursor_row: usize,
    pub selected_entry: Option<SelectedEntry>,
    pub info_pane_scroll_offset: usize,
    pub log_pane: LogPaneState,
}

impl TuiState {
    pub fn new(tui_config: &TuiConfig, search_config: &SearchConfig) -> Result<Self, FmlError> {
        let selected_theme = tui_config.resolved_theme()?;
        Ok(TuiState {
            focused: Slot::Main,
            areas: HashMap::new(),
            selected_theme,
            query_box_textarea: query_box::query_box_textarea(),
            query_box_last_dispatched_query: String::new(),
            query_box_debounce_handle: None,
            fuzzy_debounce_ms: search_config.fuzzy_debounce_ms,
            log_pane_cursor_row: 0,
            selected_entry: None,
            info_pane_scroll_offset: 0,
            log_pane: LogPaneState::new(search_config.tail_size),
        })
    }
}
