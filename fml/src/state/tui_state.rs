use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

pub mod log_pane_state;

use log_pane_state::LogPaneState;

use crate::{
    config::{
        search::SearchConfig,
        tui::{ThemeConfig, TuiConfig},
    },
    error::FmlError,
    tui::{layout::Slot, widgets::query_box},
};

pub struct TuiState {
    pub focused: Slot,
    pub areas: HashMap<Slot, Rect>,
    pub selected_theme: ThemeConfig,
    pub query_box_textarea: TextArea<'static>,
    /// Absolute index into the current display list (all entries or search results).
    pub absolute_cursor: usize,
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
            absolute_cursor: 0,
            log_pane: LogPaneState::new(search_config.tail_size),
        })
    }
}
