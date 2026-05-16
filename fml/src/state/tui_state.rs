use std::collections::{BTreeMap, HashMap, HashSet};

use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use tokio::task::JoinHandle;

pub mod log_pane_state;
pub mod preview_pane_state;

use log_pane_state::LogPaneState;
use preview_pane_state::PreviewPaneState;
use tui_popup::Popup;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePopup {
    Help,
    FieldPicker,
    SourceSelector,
}

pub struct SourceSelectorState {
    pub cursor: usize,
    pub scroll_offset: usize,
    pub visible_row_count: usize,
    pub enabled_source_ids: HashSet<SourceId>,
    pub open_sources: Vec<Source>,
}

impl SourceSelectorState {
    fn new() -> Self {
        Self {
            cursor: 0,
            scroll_offset: 0,
            visible_row_count: usize::MAX,
            enabled_source_ids: HashSet::new(),
            open_sources: Vec::new(),
        }
    }
}

pub struct FieldPickerState {
    pub cursor: usize,
    pub scroll_offset: usize,
    pub visible_row_count: usize,
    pub selected_field_keys: HashSet<String>,
}

impl FieldPickerState {
    fn new() -> Self {
        Self {
            cursor: 0,
            scroll_offset: 0,
            visible_row_count: usize::MAX,
            selected_field_keys: HashSet::new(),
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
    pub active_popup: Option<ActivePopup>,
    pub source_selector: SourceSelectorState,
    pub field_picker: FieldPickerState,
}

impl TuiState {
    pub fn new(tui_config: &TuiConfig, search_config: &SearchConfig) -> Result<Self, FmlError> {
        Self::new_with_themes(tui_config, search_config, &BTreeMap::new())
    }

    /// Build TUI state with top-level user-defined themes available for theme resolution.
    pub fn new_with_themes(
        tui_config: &TuiConfig,
        search_config: &SearchConfig,
        user_themes: &BTreeMap<String, ThemeConfig>,
    ) -> Result<Self, FmlError> {
        let selected_theme = tui_config.resolved_theme_with(user_themes)?;
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
            active_popup: None,
            source_selector: SourceSelectorState::new(),
            field_picker: FieldPickerState::new(),
        })
    }

    pub fn active_popup(&self) -> Option<ActivePopup> {
        self.active_popup
    }

    pub fn open_popup(&mut self, popup: ActivePopup) {
        self.active_popup = Some(popup);
    }

    pub fn close_popup(&mut self) {
        self.active_popup = None;
    }

    pub fn toggle_help(&mut self) {
        if self.active_popup == Some(ActivePopup::Help) {
            self.close_popup();
        } else {
            self.open_popup(ActivePopup::Help);
        }
    }

    pub fn source_selector_is_open(&self) -> bool {
        self.active_popup == Some(ActivePopup::SourceSelector)
    }

    pub fn field_picker_is_open(&self) -> bool {
        self.active_popup == Some(ActivePopup::FieldPicker)
    }

    pub fn open_field_picker(&mut self) {
        self.open_popup(ActivePopup::FieldPicker);
        self.field_picker.cursor = 0;
        self.field_picker.scroll_offset = 0;
        self.prune_field_picker_selection_to_selected_entry();
    }

    pub fn close_field_picker(&mut self) {
        if self.field_picker_is_open() {
            self.close_popup();
        }
    }

    pub fn field_picker_selected_row(&self) -> usize {
        self.field_picker.scroll_offset + self.field_picker.cursor
    }

    pub fn field_picker_cursor_up(&mut self, row_count: usize) {
        if row_count == 0 {
            self.field_picker.cursor = 0;
            self.field_picker.scroll_offset = 0;
            return;
        }

        if self.field_picker.cursor > 0 {
            self.field_picker.cursor -= 1;
        } else {
            self.field_picker.scroll_offset = self.field_picker.scroll_offset.saturating_sub(1);
        }
    }

    pub fn field_picker_cursor_down(&mut self, row_count: usize) {
        if row_count == 0 {
            self.field_picker.cursor = 0;
            self.field_picker.scroll_offset = 0;
            return;
        }

        let visible = self.field_picker.visible_row_count.min(row_count).max(1);
        let selected = self.field_picker_selected_row();
        if selected + 1 >= row_count {
            return;
        }

        if self.field_picker.cursor + 1 < visible {
            self.field_picker.cursor += 1;
        } else {
            self.field_picker.scroll_offset += 1;
        }
    }

    pub fn set_field_picker_visible_row_count(&mut self, row_count: usize, visible: usize) {
        self.field_picker.visible_row_count = visible.max(1);
        if row_count == 0 {
            self.field_picker.cursor = 0;
            self.field_picker.scroll_offset = 0;
            return;
        }

        let visible = self.field_picker.visible_row_count.min(row_count);
        let selected = self
            .field_picker_selected_row()
            .min(row_count.saturating_sub(1));
        self.field_picker.scroll_offset = self
            .field_picker
            .scroll_offset
            .min(row_count.saturating_sub(visible));
        if selected < self.field_picker.scroll_offset {
            self.field_picker.scroll_offset = selected;
        } else if selected >= self.field_picker.scroll_offset + visible {
            self.field_picker.scroll_offset = selected.saturating_sub(visible - 1);
        }
        self.field_picker.cursor = selected - self.field_picker.scroll_offset;
    }

    pub fn toggle_field_picker_key(&mut self, key: &str) {
        if !self.field_picker.selected_field_keys.remove(key) {
            self.field_picker
                .selected_field_keys
                .insert(key.to_string());
        }
    }

    pub fn selected_field_picker_keys(&self) -> Vec<String> {
        let mut keys = self
            .field_picker
            .selected_field_keys
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub fn prune_field_picker_selection_to_selected_entry(&mut self) {
        let Some(selected_entry) = &self.selected_entry else {
            self.field_picker.selected_field_keys.clear();
            self.field_picker.cursor = 0;
            self.field_picker.scroll_offset = 0;
            return;
        };

        self.field_picker
            .selected_field_keys
            .retain(|key| selected_entry.entry.fields.contains_key(key));
        let row_count = selected_entry.entry.fields.len();
        self.set_field_picker_visible_row_count(row_count, self.field_picker.visible_row_count);
    }

    pub fn open_source_selector(&mut self, sources: &[Source]) {
        self.open_popup(ActivePopup::SourceSelector);
        self.source_selector.cursor = 0;
        self.source_selector.scroll_offset = 0;
        self.source_selector.open_sources = sources.to_vec();
    }

    pub fn close_source_selector(&mut self) {
        if self.source_selector_is_open() {
            self.close_popup();
        }
    }

    pub fn toggle_source_selector(&mut self, sources: &[Source]) {
        if self.source_selector_is_open() {
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
