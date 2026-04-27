use std::sync::Arc;

use crate::{event::Query, log::LogEntry};

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollMode {
    Tail,
    History,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Tail,
    History,
    Fuzzy,
}

pub struct LogPaneState {
    pub mode: ScrollMode,
    /// Indices into the backing buffer for the current search results, ranked best-last.
    pub search_results: Vec<usize>,
    /// Resolved log entries for the current display window, in render order.
    pub items: Vec<Arc<LogEntry>>,
    pub height: usize,
    pub view_start: usize,
    pub selected_seq: Option<u64>,
    pub retained_bounds: (u64, u64),
    pub active_query: SearchKind,
    history_buffer_limit: usize,
}

impl Default for LogPaneState {
    fn default() -> Self {
        LogPaneState {
            mode: ScrollMode::Tail,
            search_results: Vec::new(),
            items: Vec::new(),
            height: 0,
            view_start: 0,
            selected_seq: None,
            retained_bounds: (0, 0),
            active_query: SearchKind::Tail,
            history_buffer_limit: 500,
        }
    }
}

impl LogPaneState {
    pub fn new(history_buffer_limit: usize) -> Self {
        Self {
            history_buffer_limit: history_buffer_limit.max(1),
            ..Self::default()
        }
    }

    pub fn query_kind(query: &Query) -> SearchKind {
        match query {
            Query::Tail => SearchKind::Tail,
            Query::History { .. } => SearchKind::History,
            Query::Fuzzy(_) => SearchKind::Fuzzy,
        }
    }

    pub fn on_search_started(&mut self, query: &Query) {
        self.active_query = Self::query_kind(query);
    }

    pub fn set_height(&mut self, height: usize, cursor: &mut usize) {
        self.height = height;
        self.reconcile_view(cursor);
    }

    pub fn visible_items(&self) -> &[Arc<LogEntry>] {
        let end = self
            .view_start
            .saturating_add(self.height)
            .min(self.items.len());
        &self.items[self.view_start..end]
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        let selected_index = self.selected_index()?;
        let end = self
            .view_start
            .saturating_add(self.height)
            .min(self.items.len());
        if selected_index >= self.view_start && selected_index < end {
            Some(selected_index - self.view_start)
        } else {
            None
        }
    }

    pub fn apply_results(
        &mut self,
        kind: SearchKind,
        entries: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
        cursor: &mut usize,
    ) {
        self.retained_bounds = retained_bounds;

        match kind {
            SearchKind::Tail => {
                if self.mode != ScrollMode::Tail {
                    return;
                }
                self.items = entries;
                self.selected_seq = self.items.last().map(|entry| entry.seq);
                self.view_start = self.tail_view_start();
            }
            SearchKind::History => {
                self.mode = ScrollMode::History;
                self.items = entries;
                self.clamp_selected_seq();
                self.preserve_cursor_row(cursor);
            }
            SearchKind::Fuzzy => {
                self.mode = ScrollMode::Search;
                self.items = entries;
                self.clamp_selected_seq();
                self.preserve_cursor_row(cursor);
            }
        }

        self.reconcile_view(cursor);
    }

    pub fn scroll_backward(&mut self, cursor: &mut usize) -> Option<Query> {
        if self.items.is_empty() {
            return None;
        }

        match self.mode {
            ScrollMode::Tail => {
                let selected = self.seq_at_cursor(*cursor).or(self.selected_seq)?;
                let previous = self
                    .previous_seq(selected)
                    .unwrap_or_else(|| selected.saturating_sub(1).max(self.retained_bounds.0));
                if previous == selected {
                    return None;
                }

                self.mode = ScrollMode::History;
                self.selected_seq = Some(previous);
                self.preserve_cursor_row(cursor);
                Some(self.history_query(previous))
            }
            ScrollMode::History | ScrollMode::Search => {
                let selected = self.selected_seq.or_else(|| self.seq_at_cursor(*cursor))?;
                let previous = self
                    .previous_seq(selected)
                    .unwrap_or_else(|| selected.saturating_sub(1).max(self.retained_bounds.0));
                if previous == selected {
                    return None;
                }

                self.mode = ScrollMode::History;
                self.selected_seq = Some(previous);
                if self
                    .selected_index()
                    .is_some_and(|idx| idx >= self.view_start)
                {
                    self.reconcile_view(cursor);
                    None
                } else {
                    self.preserve_cursor_row(cursor);
                    Some(self.history_query(previous))
                }
            }
        }
    }

    pub fn scroll_forward(&mut self, cursor: &mut usize) -> Option<Query> {
        if self.items.is_empty() {
            return None;
        }

        let selected = self.selected_seq.or_else(|| self.seq_at_cursor(*cursor))?;
        let retained_high = self.retained_bounds.1;
        let next = self
            .next_seq(selected)
            .unwrap_or_else(|| selected.saturating_add(1).min(retained_high));

        if retained_high != 0 && next >= retained_high {
            self.mode = ScrollMode::Tail;
            self.selected_seq = Some(retained_high);
            self.view_start = self.tail_view_start();
            self.reconcile_view(cursor);
            return Some(Query::Tail);
        }

        if next == selected {
            return None;
        }

        self.mode = ScrollMode::History;
        self.selected_seq = Some(next);
        if self.selected_index().is_some_and(|idx| {
            idx < self
                .view_start
                .saturating_add(self.height)
                .min(self.items.len())
        }) {
            self.reconcile_view(cursor);
            None
        } else {
            self.preserve_cursor_row(cursor);
            Some(self.history_query(next))
        }
    }

    pub fn jump_head(&mut self, cursor: &mut usize) -> Option<Query> {
        let low = self.retained_bounds.0;
        if low == 0 {
            return None;
        }
        self.mode = ScrollMode::History;
        self.selected_seq = Some(low);
        *cursor = 0;
        Some(self.history_query(low))
    }

    pub fn jump_tail(&mut self, cursor: &mut usize) -> Option<Query> {
        let high = self.retained_bounds.1;
        self.mode = ScrollMode::Tail;
        self.selected_seq = (high != 0).then_some(high);
        self.view_start = self.tail_view_start();
        *cursor = self.visible_items().len().saturating_sub(1);
        Some(Query::Tail)
    }

    fn history_query(&self, middle_seq_id: u64) -> Query {
        Query::History {
            middle_seq_id,
            buffer: self.history_buffer(),
        }
    }

    fn history_buffer(&self) -> u64 {
        let viewport = self.height.max(1).saturating_mul(2);
        viewport.min(self.history_buffer_limit) as u64
    }

    fn tail_view_start(&self) -> usize {
        self.items.len().saturating_sub(self.height)
    }

    fn selected_index(&self) -> Option<usize> {
        let selected_seq = self.selected_seq?;
        self.items
            .iter()
            .position(|entry| entry.seq == selected_seq)
    }

    fn seq_at_cursor(&self, cursor: usize) -> Option<u64> {
        self.items
            .get(self.view_start.saturating_add(cursor))
            .map(|entry| entry.seq)
    }

    fn previous_seq(&self, selected: u64) -> Option<u64> {
        let index = self.items.iter().position(|entry| entry.seq == selected)?;
        index.checked_sub(1).map(|idx| self.items[idx].seq)
    }

    fn next_seq(&self, selected: u64) -> Option<u64> {
        let index = self.items.iter().position(|entry| entry.seq == selected)?;
        self.items.get(index + 1).map(|entry| entry.seq)
    }

    fn clamp_selected_seq(&mut self) {
        if self.items.is_empty() {
            self.selected_seq = None;
            return;
        }

        if let Some(selected_seq) = self.selected_seq {
            if self.items.iter().any(|entry| entry.seq == selected_seq) {
                return;
            }

            self.selected_seq = self
                .items
                .iter()
                .min_by_key(|entry| entry.seq.abs_diff(selected_seq))
                .map(|entry| entry.seq);
            return;
        }

        self.selected_seq = self.items.last().map(|entry| entry.seq);
    }

    fn preserve_cursor_row(&mut self, cursor: &mut usize) {
        let Some(selected_index) = self.selected_index() else {
            self.view_start = self.tail_view_start();
            return;
        };

        let visible_height = self.height.max(1);
        let desired_row = (*cursor).min(visible_height.saturating_sub(1));
        let max_start = self.items.len().saturating_sub(visible_height);
        self.view_start = selected_index.saturating_sub(desired_row).min(max_start);
    }

    fn reconcile_view(&mut self, cursor: &mut usize) {
        if self.items.is_empty() || self.height == 0 {
            self.view_start = 0;
            *cursor = 0;
            return;
        }

        if self.mode == ScrollMode::Tail {
            self.view_start = self.tail_view_start();
            self.selected_seq = self.items.last().map(|entry| entry.seq);
        }

        let max_start = self.items.len().saturating_sub(self.height);
        self.view_start = self.view_start.min(max_start);

        if let Some(selected_index) = self.selected_index() {
            if selected_index < self.view_start {
                self.view_start = selected_index;
            }

            let end = self
                .view_start
                .saturating_add(self.height)
                .min(self.items.len());
            if selected_index >= end {
                self.view_start = selected_index.saturating_add(1).saturating_sub(self.height);
            }

            *cursor = selected_index.saturating_sub(self.view_start);
        } else {
            *cursor = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use chrono::Utc;

    use super::*;
    use crate::log::{LogLevel, Source};

    fn entry(seq: u64) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq,
            msg: format!("entry {seq}"),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                id: "src-a".to_string(),
                display_name: "src-a".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    fn entries(start: u64, end: u64) -> Vec<Arc<LogEntry>> {
        (start..=end).map(entry).collect()
    }

    #[test]
    fn tail_results_pin_newest_visible_entry() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);

        state.apply_results(SearchKind::Tail, entries(1, 5), (1, 5), &mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.selected_seq, Some(5));
        assert_eq!(state.view_start, 2);
        assert_eq!(cursor, 2);
        assert_eq!(
            state
                .visible_items()
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn scroll_up_from_tail_enters_history_and_requests_anchor() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.apply_results(SearchKind::Tail, entries(1, 10), (1, 10), &mut cursor);

        let query = state.scroll_backward(&mut cursor);

        assert_eq!(state.mode, ScrollMode::History);
        assert_eq!(state.selected_seq, Some(9));
        assert_eq!(
            query,
            Some(Query::History {
                middle_seq_id: 9,
                buffer: 8
            })
        );
    }

    #[test]
    fn history_refresh_preserves_selected_row() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(5, &mut cursor);
        cursor = 2;
        state.mode = ScrollMode::History;
        state.selected_seq = Some(50);

        state.apply_results(SearchKind::History, entries(45, 55), (1, 100), &mut cursor);

        assert_eq!(state.selected_seq, Some(50));
        assert_eq!(cursor, 2);
        assert_eq!(
            state
                .visible_items()
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![48, 49, 50, 51, 52]
        );
    }

    #[test]
    fn home_anchors_to_retained_low() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 3;
        state.set_height(4, &mut cursor);
        state.apply_results(SearchKind::Tail, entries(80, 100), (1, 100), &mut cursor);

        let query = state.jump_head(&mut cursor);

        assert_eq!(state.mode, ScrollMode::History);
        assert_eq!(state.selected_seq, Some(1));
        assert_eq!(cursor, 0);
        assert_eq!(
            query,
            Some(Query::History {
                middle_seq_id: 1,
                buffer: 8
            })
        );
    }

    #[test]
    fn end_returns_to_tail() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.mode = ScrollMode::History;
        state.retained_bounds = (1, 100);
        state.items = entries(40, 60);
        state.selected_seq = Some(50);

        let query = state.jump_tail(&mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.selected_seq, Some(100));
        assert_eq!(query, Some(Query::Tail));
    }

    #[test]
    fn scrolling_down_to_retained_high_returns_to_tail() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.mode = ScrollMode::History;
        state.retained_bounds = (1, 10);
        state.items = entries(6, 10);
        state.selected_seq = Some(9);
        state.reconcile_view(&mut cursor);

        let query = state.scroll_forward(&mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.selected_seq, Some(10));
        assert_eq!(query, Some(Query::Tail));
    }
}
