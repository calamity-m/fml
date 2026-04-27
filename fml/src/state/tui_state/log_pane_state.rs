use std::{collections::HashMap, sync::Arc};

use crate::{
    event::{Match, Query},
    log::LogEntry,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarMetrics {
    pub content_length: usize,
    pub viewport_content_length: usize,
    pub position: usize,
}

pub struct LogPaneState {
    pub mode: ScrollMode,
    /// Indices into the backing buffer for the current search results, ranked best-last.
    pub search_results: Vec<usize>,
    /// Resolved log entries for the current display window, in render order.
    pub items: Vec<Arc<LogEntry>>,
    pub fuzzy_matches: HashMap<u64, Vec<Match>>,
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
            fuzzy_matches: HashMap::new(),
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
        match self.active_query {
            SearchKind::Tail => self.mode = ScrollMode::Tail,
            SearchKind::Fuzzy => self.mode = ScrollMode::Search,
            SearchKind::History => {}
        }
    }

    /// Scrollbar metrics for the retained log stream.
    ///
    /// This is intentionally store-relative rather than window-relative so the
    /// thumb stays stable as the retained window shifts. Search/fuzzy may need
    /// a mode-specific domain later, but that decision is deferred for TODO #6.
    pub fn scrollbar_metrics(&self) -> Option<ScrollbarMetrics> {
        if self.mode == ScrollMode::Search {
            return None;
        }

        let (low, high) = self.retained_bounds;
        let selected_seq = self.selected_seq?;

        if self.height == 0 || (low == 0 && high == 0) || high < low {
            return None;
        }

        let retained_count = high.saturating_sub(low).saturating_add(1);
        if retained_count <= self.height as u64 {
            return None;
        }

        let content_length = usize::try_from(retained_count).unwrap_or(usize::MAX);
        let viewport_content_length = self.height.min(content_length);
        let clamped_selected = selected_seq.clamp(low, high);
        let position = usize::try_from(clamped_selected.saturating_sub(low))
            .unwrap_or(usize::MAX)
            .min(content_length.saturating_sub(1));

        Some(ScrollbarMetrics {
            content_length,
            viewport_content_length,
            position,
        })
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
        self.apply_results_with_matches(kind, entries, retained_bounds, HashMap::new(), cursor);
    }

    pub fn apply_results_with_matches(
        &mut self,
        kind: SearchKind,
        entries: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
        fuzzy_matches: HashMap<u64, Vec<Match>>,
        cursor: &mut usize,
    ) {
        self.retained_bounds = retained_bounds;

        match kind {
            SearchKind::Tail => {
                if self.mode != ScrollMode::Tail {
                    return;
                }
                self.fuzzy_matches.clear();
                self.items = entries;
                self.selected_seq = self.items.last().map(|entry| entry.seq);
                self.view_start = self.tail_view_start();
            }
            SearchKind::History => {
                self.fuzzy_matches.clear();
                self.mode = ScrollMode::History;
                self.items = entries;
                self.clamp_selected_seq();
                self.preserve_cursor_row(cursor);
            }
            SearchKind::Fuzzy => {
                self.mode = ScrollMode::Search;
                // Keep match metadata alongside the resolved entries now so
                // TODO #7 can render highlights without asking the search
                // worker to replay or rescore the current result set.
                self.fuzzy_matches = fuzzy_matches;
                // Fuzzy results arrive best-first, but the README defines
                // "tail" as highest rank so End/down naturally move toward
                // the strongest hit just like tail mode moves toward newest.
                self.items = entries.into_iter().rev().collect();
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
            ScrollMode::Search => {
                // Search mode is rank-local: navigation must not issue history
                // queries because fuzzy result indices are not log sequence
                // neighbors.
                let selected = self.selected_seq.or_else(|| self.seq_at_cursor(*cursor))?;
                let previous = self.previous_seq(selected)?;
                if previous == selected {
                    return None;
                }

                self.selected_seq = Some(previous);
                self.reconcile_view(cursor);
                None
            }
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
            ScrollMode::History => {
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

        if self.mode == ScrollMode::Search {
            // Search mode is rank-local: navigation must not issue history or
            // tail queries because fuzzy result indices are not log sequence
            // neighbors.
            let selected = self.selected_seq.or_else(|| self.seq_at_cursor(*cursor))?;
            let next = self.next_seq(selected)?;
            if next == selected {
                return None;
            }

            self.selected_seq = Some(next);
            self.reconcile_view(cursor);
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
        if self.mode == ScrollMode::Search {
            self.selected_seq = self.items.first().map(|entry| entry.seq);
            *cursor = 0;
            self.reconcile_view(cursor);
            return None;
        }

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
        if self.mode == ScrollMode::Search {
            self.selected_seq = self.items.last().map(|entry| entry.seq);
            self.view_start = self.tail_view_start();
            *cursor = self.visible_items().len().saturating_sub(1);
            self.reconcile_view(cursor);
            return None;
        }

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
    fn scrollbar_metrics_hide_when_state_is_not_ready() {
        let mut state = LogPaneState::new(500);
        state.height = 5;
        state.retained_bounds = (1, 10);

        assert_eq!(state.scrollbar_metrics(), None);

        state.selected_seq = Some(5);
        state.height = 0;

        assert_eq!(state.scrollbar_metrics(), None);

        state.height = 5;
        state.retained_bounds = (0, 0);

        assert_eq!(state.scrollbar_metrics(), None);
    }

    #[test]
    fn scrollbar_metrics_hide_when_content_fits() {
        let mut state = LogPaneState::new(500);
        state.height = 5;
        state.selected_seq = Some(3);
        state.retained_bounds = (1, 5);

        assert_eq!(state.scrollbar_metrics(), None);
    }

    #[test]
    fn scrollbar_metrics_clamp_selection_within_retained_bounds() {
        let mut state = LogPaneState::new(500);
        state.height = 4;
        state.retained_bounds = (10, 20);

        state.selected_seq = Some(10);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 11,
                viewport_content_length: 4,
                position: 0,
            })
        );

        state.selected_seq = Some(15);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 11,
                viewport_content_length: 4,
                position: 5,
            })
        );

        state.selected_seq = Some(25);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 11,
                viewport_content_length: 4,
                position: 10,
            })
        );
    }

    #[test]
    fn scrollbar_metrics_saturate_extreme_ranges_without_panicking() {
        let mut state = LogPaneState::new(500);
        state.height = 2;
        state.selected_seq = Some(u64::MAX);
        state.retained_bounds = (0, u64::MAX);

        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: usize::MAX,
                viewport_content_length: 2,
                position: usize::MAX - 1,
            })
        );
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
