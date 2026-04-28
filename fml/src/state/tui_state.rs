use std::collections::{HashMap, HashSet};

use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use tokio::task::JoinHandle;

pub mod log_pane_state;
pub mod preview_pane_state;

use log_pane_state::LogPaneState;
use preview_pane_state::PreviewPaneState;

use crate::{
    config::{
        search::SearchConfig,
        tui::{ThemeConfig, TuiConfig},
    },
    error::FmlError,
    event::SelectedEntry,
    log::{Source, SourceId},
    tui::{layout::Slot, widgets::query_box},
};

pub struct SourceSelectorState {
    pub open: bool,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub visible_row_count: usize,
    pub enabled_source_ids: HashSet<SourceId>,
    pub open_sources: Vec<Source>,
}

impl SourceSelectorState {
    fn new() -> Self {
        Self {
            open: false,
            cursor: 0,
            scroll_offset: 0,
            visible_row_count: usize::MAX,
            enabled_source_ids: HashSet::new(),
            open_sources: Vec::new(),
        }
    }
}

pub struct TuiState {
    pub focused: Slot,
    pub areas: HashMap<Slot, Rect>,
    pub selected_theme: ThemeConfig,
    pub query_box_textarea: TextArea<'static>,
    pub query_box_last_dispatched_query: String,
    pub query_box_debounce_handle: Option<JoinHandle<()>>,
    pub source_filter_debounce_handle: Option<JoinHandle<()>>,
    pub fuzzy_debounce_ms: u64,
    /// Selected visible row in the log pane viewport.
    pub log_pane_cursor_row: usize,
    pub selected_entry: Option<SelectedEntry>,
    pub info_pane_scroll_offset: usize,
    pub log_pane: LogPaneState,
    pub preview_pane: PreviewPaneState,
    pub source_selector: SourceSelectorState,
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
            source_filter_debounce_handle: None,
            fuzzy_debounce_ms: search_config.fuzzy_debounce_ms,
            log_pane_cursor_row: 0,
            selected_entry: None,
            info_pane_scroll_offset: 0,
            log_pane: LogPaneState::new(search_config.tail_size),
            preview_pane: PreviewPaneState::new(),
            source_selector: SourceSelectorState::new(),
        })
    }

    pub fn open_source_selector(&mut self, sources: &[Source]) {
        self.source_selector.open = true;
        self.source_selector.cursor = 0;
        self.source_selector.scroll_offset = 0;
        self.source_selector.open_sources = sources.to_vec();
    }

    pub fn close_source_selector(&mut self) {
        self.source_selector.open = false;
    }

    pub fn toggle_source_selector(&mut self, sources: &[Source]) {
        if self.source_selector.open {
            self.close_source_selector();
        } else {
            self.open_source_selector(sources);
        }
    }

    pub fn source_selector_cursor_up(&mut self, row_count: usize) {
        if row_count == 0 {
            self.source_selector.cursor = 0;
            self.source_selector.scroll_offset = 0;
            return;
        }

        if self.source_selector.cursor > 0 {
            self.source_selector.cursor -= 1;
        } else {
            self.source_selector.scroll_offset =
                self.source_selector.scroll_offset.saturating_sub(1);
        }
    }

    pub fn source_selector_cursor_down(&mut self, row_count: usize) {
        if row_count == 0 {
            self.source_selector.cursor = 0;
            self.source_selector.scroll_offset = 0;
            return;
        }

        let visible = self.source_selector.visible_row_count.min(row_count).max(1);
        let selected = self.source_selector_selected_row();
        if selected + 1 >= row_count {
            return;
        }

        if self.source_selector.cursor + 1 < visible {
            self.source_selector.cursor += 1;
        } else {
            self.source_selector.scroll_offset += 1;
        }
    }

    pub fn set_source_selector_visible_row_count(&mut self, row_count: usize, visible: usize) {
        self.source_selector.visible_row_count = visible.max(1);
        if row_count == 0 {
            self.source_selector.cursor = 0;
            self.source_selector.scroll_offset = 0;
            return;
        }

        let visible = self.source_selector.visible_row_count.min(row_count);
        let selected = self
            .source_selector_selected_row()
            .min(row_count.saturating_sub(1));
        self.source_selector.scroll_offset = self
            .source_selector
            .scroll_offset
            .min(row_count.saturating_sub(visible));
        if selected < self.source_selector.scroll_offset {
            self.source_selector.scroll_offset = selected;
        } else if selected >= self.source_selector.scroll_offset + visible {
            self.source_selector.scroll_offset = selected.saturating_sub(visible - 1);
        }
        self.source_selector.cursor = selected - self.source_selector.scroll_offset;
    }

    pub fn source_selector_selected_row(&self) -> usize {
        self.source_selector.scroll_offset + self.source_selector.cursor
    }

    pub fn enable_source_id(&mut self, source_id: SourceId) {
        self.source_selector.enabled_source_ids.insert(source_id);
    }

    pub fn remove_source_id(&mut self, source_id: &SourceId) {
        self.source_selector.enabled_source_ids.remove(source_id);
    }
}
