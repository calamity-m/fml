use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};

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
    /// Whether log entries are rendered with multi-line wrapping. Shared by
    /// the log pane and the preview pane; toggled via the
    /// `toggle_line_wrap` keybinding.
    pub line_wrap: bool,
    pub active_popup: Option<ActivePopup>,
    pub source_selector: SourceSelectorState,
    pub field_picker: FieldPickerState,
    /// Transient status-bar message and the instant it was set.
    pub status_message: Option<(String, Instant)>,
    /// How long a transient status message remains visible.
    pub status_message_ttl: Duration,
    /// Next message to promote when the current one expires.
    pub status_message_pending: Option<String>,
    /// When `true`, mouse capture is released so the terminal handles
    /// drag-selection and wheel scrollback.
    pub select_mode: bool,
    /// Set to `true` after the first yank in a multiplexer session so the
    /// one-time clipboard-config hint is not repeated.
    pub multiplexer_clipboard_hint_shown: bool,
    /// When `true`, `status_message()` always returns `None`. Used by snapshot
    /// tests to keep existing baselines stable.
    pub suppress_status_messages: bool,
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
            line_wrap: tui_config.line_wrap,
            active_popup: None,
            source_selector: SourceSelectorState::new(),
            field_picker: FieldPickerState::new(),
            status_message: None,
            status_message_ttl: Duration::from_secs(3),
            status_message_pending: None,
            select_mode: false,
            multiplexer_clipboard_hint_shown: false,
            suppress_status_messages: tui_config.suppress_status_messages,
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

    /// Set a transient status-bar message, replacing any current message and
    /// clearing the pending queue.
    pub fn set_status_message(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
        self.status_message_pending = None;
    }

    /// Queue a message to display after the current one expires. If there is no
    /// current active message the queued message becomes current immediately.
    pub fn queue_status_message(&mut self, msg: String) {
        let now = Instant::now();
        let active = self
            .status_message
            .as_ref()
            .is_some_and(|(_, ts)| now.duration_since(*ts) < self.status_message_ttl);
        if active {
            self.status_message_pending = Some(msg);
        } else {
            self.status_message = Some((msg, now));
        }
    }

    /// Return the current transient message if it is within its TTL, promoting
    /// any pending message when the current one has expired.
    ///
    /// Returns `None` when there is no active message or when
    /// `suppress_status_messages` is set.
    pub fn status_message(&mut self, now: Instant) -> Option<&str> {
        if self.suppress_status_messages {
            return None;
        }
        let expired = self
            .status_message
            .as_ref()
            .is_some_and(|(_, ts)| now.duration_since(*ts) >= self.status_message_ttl);
        if expired {
            if let Some(pending) = self.status_message_pending.take() {
                self.status_message = Some((pending, now));
            } else {
                self.status_message = None;
            }
        }
        self.status_message.as_ref().map(|(msg, _)| msg.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::config::{search::SearchConfig, tui::TuiConfig};

    use super::TuiState;

    fn state() -> TuiState {
        TuiState::new(&TuiConfig::default(), &SearchConfig::default()).unwrap()
    }

    #[test]
    fn status_message_visible_within_ttl() {
        let mut s = state();
        s.set_status_message("hello".into());
        let now = Instant::now();
        assert_eq!(s.status_message(now), Some("hello"));
    }

    #[test]
    fn status_message_expires_after_ttl() {
        let mut s = state();
        s.set_status_message("hello".into());
        // Use the stored timestamp to compute a deterministic future instant.
        let ts = s.status_message.as_ref().unwrap().1;
        let after = ts + s.status_message_ttl + Duration::from_millis(1);
        assert_eq!(s.status_message(after), None);
    }

    #[test]
    fn suppress_status_messages_hides_message() {
        let mut config = TuiConfig::default();
        config.suppress_status_messages = true;
        let mut s = TuiState::new(&config, &SearchConfig::default()).unwrap();
        s.set_status_message("hello".into());
        assert_eq!(s.status_message(Instant::now()), None);
    }

    #[test]
    fn pending_message_promoted_when_current_expires() {
        let mut s = state();
        s.set_status_message("first".into());
        s.queue_status_message("second".into());
        // Before expiry: first is visible, pending queued.
        let now = Instant::now();
        assert_eq!(s.status_message(now), Some("first"));
        // After expiry: pending is promoted.
        let ts = s.status_message.as_ref().unwrap().1;
        let after = ts + s.status_message_ttl + Duration::from_millis(1);
        assert_eq!(s.status_message(after), Some("second"));
    }

    #[test]
    fn set_status_message_clears_pending() {
        let mut s = state();
        s.set_status_message("first".into());
        s.queue_status_message("second".into());
        s.set_status_message("replaced".into());
        let ts = s.status_message.as_ref().unwrap().1;
        let after = ts + s.status_message_ttl + Duration::from_millis(1);
        // Pending was cleared, so nothing after expiry.
        assert_eq!(s.status_message(after), None);
    }
}
