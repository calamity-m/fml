use std::{collections::HashMap, sync::Arc};

use crate::{
    event::{Match, Query, SearchProgress, SelectedEntry},
    log::LogEntry,
    store::StoreStats,
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

pub enum LogPaneUpdate {
    Tail {
        entries: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
    },
    History {
        entries: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
    },
    Fuzzy {
        best_first_entries: Vec<Arc<LogEntry>>,
        retained_bounds: (u64, u64),
        matches_by_seq: HashMap<u64, Vec<Match>>,
    },
}

#[derive(Default)]
struct LogPaneContent {
    /// Resolved log entries for the current display window, in render order.
    items: Vec<Arc<LogEntry>>,
    fuzzy_matches: HashMap<u64, Vec<Match>>,
    empty_message: Option<&'static str>,
}

#[derive(Default)]
struct LogPaneViewport {
    height: usize,
    view_start: usize,
    selected_seq: Option<u64>,
}

pub struct LogPaneState {
    pub mode: ScrollMode,
    content: LogPaneContent,
    viewport: LogPaneViewport,
    pub retained_bounds: (u64, u64),
    pub store_stats: StoreStats,
    fuzzy_scan_progress: Option<SearchProgress>,
    pub active_query: Option<Query>,
    history_buffer_limit: usize,
}

impl Default for LogPaneState {
    fn default() -> Self {
        LogPaneState {
            mode: ScrollMode::Tail,
            content: LogPaneContent::default(),
            viewport: LogPaneViewport::default(),
            retained_bounds: (0, 0),
            store_stats: StoreStats::default(),
            fuzzy_scan_progress: None,
            active_query: None,
            history_buffer_limit: 500,
        }
    }
}

fn retained_count(bounds: (u64, u64)) -> usize {
    let (low, high) = bounds;
    if low == 0 && high == 0 || high < low {
        return 0;
    }

    usize::try_from(high.saturating_sub(low).saturating_add(1)).unwrap_or(usize::MAX)
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
            Query::Surrounding { .. } => SearchKind::History,
            Query::Fuzzy(_) => SearchKind::Fuzzy,
        }
    }

    pub fn on_search_started(&mut self, query: &Query) {
        self.content.empty_message = None;
        self.active_query = Some(query.clone());
        self.fuzzy_scan_progress = None;
        match Self::query_kind(query) {
            SearchKind::Tail => {
                self.mode = ScrollMode::Tail;
            }
            SearchKind::Fuzzy => self.mode = ScrollMode::Search,
            SearchKind::History => {}
        }
    }

    pub fn set_store_stats(&mut self, store_stats: StoreStats) {
        self.retained_bounds = store_stats.bounds;
        self.store_stats = store_stats;
    }

    pub fn set_fuzzy_scan_progress(&mut self, progress: Option<SearchProgress>) {
        self.fuzzy_scan_progress = progress;
    }

    pub fn fuzzy_scan_progress(&self) -> Option<SearchProgress> {
        self.fuzzy_scan_progress
    }

    /// Scrollbar metrics for the active scroll domain.
    ///
    /// Tail/history use the retained log stream so the thumb stays stable as
    /// windows shift. Search uses the current fuzzy emission because rank
    /// position, not log sequence, is the user's active coordinate.
    pub fn scrollbar_metrics(&self) -> Option<ScrollbarMetrics> {
        match self.mode {
            ScrollMode::Search => self.search_scrollbar_metrics(),
            ScrollMode::Tail | ScrollMode::History => self.retained_scrollbar_metrics(),
        }
    }

    fn retained_scrollbar_metrics(&self) -> Option<ScrollbarMetrics> {
        let (low, high) = self.retained_bounds;
        let selected_seq = self.viewport.selected_seq?;

        if self.viewport.height == 0 || (low == 0 && high == 0) || high < low {
            return None;
        }

        let retained_count = high.saturating_sub(low).saturating_add(1);
        if retained_count <= self.viewport.height as u64 {
            return None;
        }

        let content_length = usize::try_from(retained_count).unwrap_or(usize::MAX);
        let viewport_content_length = self.viewport.height.min(content_length);
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

    fn search_scrollbar_metrics(&self) -> Option<ScrollbarMetrics> {
        let content_length = self.content.items.len();
        if self.viewport.height == 0 || content_length <= self.viewport.height {
            return None;
        }

        let position = self.selected_index()?;

        Some(ScrollbarMetrics {
            content_length,
            viewport_content_length: self.viewport.height.min(content_length),
            position,
        })
    }

    pub fn set_height(&mut self, height: usize, cursor: &mut usize) {
        self.viewport.height = height;
        self.reconcile_view(cursor);
    }

    pub fn items(&self) -> &[Arc<LogEntry>] {
        &self.content.items
    }

    pub fn view_start(&self) -> usize {
        self.viewport.view_start
    }

    pub fn selected_seq(&self) -> Option<u64> {
        self.viewport.selected_seq
    }

    pub fn set_selected_seq(&mut self, selected_seq: Option<u64>, cursor: &mut usize) {
        self.viewport.selected_seq = selected_seq;
        self.reconcile_view(cursor);
    }

    pub fn fuzzy_matches_for(&self, seq: u64) -> Option<&[Match]> {
        self.content.fuzzy_matches.get(&seq).map(Vec::as_slice)
    }

    pub(crate) fn selected_entry(&self) -> Option<SelectedEntry> {
        let selected_seq = self.viewport.selected_seq?;
        let entry = self
            .content
            .items
            .iter()
            .find(|entry| entry.seq == selected_seq)?
            .clone();
        let matches = self
            .content
            .fuzzy_matches
            .get(&selected_seq)
            .cloned()
            .unwrap_or_default();

        Some(SelectedEntry { entry, matches })
    }

    pub fn fuzzy_matches_is_empty(&self) -> bool {
        self.content.fuzzy_matches.is_empty()
    }

    pub fn visible_items(&self) -> &[Arc<LogEntry>] {
        let end = self
            .viewport
            .view_start
            .saturating_add(self.viewport.height)
            .min(self.content.items.len());
        &self.content.items[self.viewport.view_start..end]
    }

    pub fn empty_message(&self) -> Option<&'static str> {
        self.content.empty_message
    }

    pub fn show_no_sources_selected(&mut self, cursor: &mut usize) {
        self.content.items.clear();
        self.content.fuzzy_matches.clear();
        self.content.empty_message = Some("No sources selected");
        self.viewport.selected_seq = None;
        self.viewport.view_start = 0;
        *cursor = 0;
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        let selected_index = self.selected_index()?;
        let end = self
            .viewport
            .view_start
            .saturating_add(self.viewport.height)
            .min(self.content.items.len());
        if selected_index >= self.viewport.view_start && selected_index < end {
            Some(selected_index - self.viewport.view_start)
        } else {
            None
        }
    }

    pub fn apply_update(&mut self, update: LogPaneUpdate, cursor: &mut usize) {
        self.content.empty_message = None;
        match update {
            LogPaneUpdate::Tail {
                entries,
                retained_bounds,
            } => {
                if self.mode != ScrollMode::Tail {
                    return;
                }
                self.set_retained_bounds(retained_bounds);
                self.fuzzy_scan_progress = None;
                self.content.fuzzy_matches.clear();
                self.content.items = entries;
                self.viewport.selected_seq = self.content.items.last().map(|entry| entry.seq);
                self.viewport.view_start = self.tail_view_start();
            }
            LogPaneUpdate::History {
                entries,
                retained_bounds,
            } => {
                self.set_retained_bounds(retained_bounds);
                self.fuzzy_scan_progress = None;
                self.content.fuzzy_matches.clear();
                self.mode = ScrollMode::History;
                self.content.items = entries;
                self.clamp_selected_seq();
                self.preserve_cursor_row(cursor);
            }
            LogPaneUpdate::Fuzzy {
                best_first_entries,
                retained_bounds,
                matches_by_seq,
            } => {
                self.set_retained_bounds(retained_bounds);
                self.mode = ScrollMode::Search;
                let previous_selected_seq = self.viewport.selected_seq;
                let previous_selected_index = self.selected_index();
                let was_following_highest_rank = previous_selected_index
                    .is_some_and(|idx| idx == self.content.items.len().saturating_sub(1));
                self.content.fuzzy_matches = matches_by_seq;
                // Fuzzy results arrive best-first, but the README defines
                // "tail" as highest rank so End/down naturally move toward
                // the strongest hit just like tail mode moves toward newest.
                self.content.items = best_first_entries.into_iter().rev().collect();
                self.reconcile_fuzzy_selection(
                    previous_selected_seq,
                    previous_selected_index,
                    was_following_highest_rank,
                    cursor,
                );
            }
        }

        self.reconcile_view(cursor);
    }

    pub fn scroll_backward(&mut self, cursor: &mut usize) -> Option<Query> {
        if self.content.items.is_empty() {
            return None;
        }

        match self.mode {
            ScrollMode::Search => {
                // Search mode is rank-local: navigation must not issue history
                // queries because fuzzy result indices are not log sequence
                // neighbors.
                let selected = self
                    .viewport
                    .selected_seq
                    .or_else(|| self.seq_at_cursor(*cursor))?;
                let previous = self.previous_seq(selected)?;
                if previous == selected {
                    return None;
                }

                self.viewport.selected_seq = Some(previous);
                self.reconcile_view(cursor);
                None
            }
            ScrollMode::Tail => {
                let selected = self.seq_at_cursor(*cursor).or(self.viewport.selected_seq)?;
                let previous = self
                    .previous_seq(selected)
                    .unwrap_or_else(|| selected.saturating_sub(1).max(self.retained_bounds.0));
                if previous == selected {
                    return None;
                }

                self.mode = ScrollMode::History;
                self.viewport.selected_seq = Some(previous);
                self.preserve_cursor_row(cursor);
                Some(self.history_query(previous))
            }
            ScrollMode::History => {
                let selected = self
                    .viewport
                    .selected_seq
                    .or_else(|| self.seq_at_cursor(*cursor))?;
                let previous = self
                    .previous_seq(selected)
                    .unwrap_or_else(|| selected.saturating_sub(1).max(self.retained_bounds.0));
                if previous == selected {
                    return None;
                }

                self.mode = ScrollMode::History;
                self.viewport.selected_seq = Some(previous);
                if self
                    .selected_index()
                    .is_some_and(|idx| idx >= self.viewport.view_start)
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
        if self.content.items.is_empty() {
            return None;
        }

        if self.mode == ScrollMode::Search {
            // Search mode is rank-local: navigation must not issue history or
            // tail queries because fuzzy result indices are not log sequence
            // neighbors.
            let selected = self
                .viewport
                .selected_seq
                .or_else(|| self.seq_at_cursor(*cursor))?;
            let next = self.next_seq(selected)?;
            if next == selected {
                return None;
            }

            self.viewport.selected_seq = Some(next);
            self.reconcile_view(cursor);
            return None;
        }

        let selected = self
            .viewport
            .selected_seq
            .or_else(|| self.seq_at_cursor(*cursor))?;
        let retained_high = self.retained_bounds.1;
        let next = self
            .next_seq(selected)
            .unwrap_or_else(|| selected.saturating_add(1).min(retained_high));

        if retained_high != 0 && next >= retained_high {
            self.mode = ScrollMode::Tail;
            self.viewport.selected_seq = Some(retained_high);
            self.viewport.view_start = self.tail_view_start();
            self.reconcile_view(cursor);
            return Some(Query::Tail);
        }

        if next == selected {
            return None;
        }

        self.mode = ScrollMode::History;
        self.viewport.selected_seq = Some(next);
        if self.selected_index().is_some_and(|idx| {
            idx < self
                .viewport
                .view_start
                .saturating_add(self.viewport.height)
                .min(self.content.items.len())
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
            self.viewport.selected_seq = self.content.items.first().map(|entry| entry.seq);
            *cursor = 0;
            self.reconcile_view(cursor);
            return None;
        }

        let low = self.retained_bounds.0;
        if low == 0 {
            return None;
        }
        self.mode = ScrollMode::History;
        self.viewport.selected_seq = Some(low);
        *cursor = 0;
        Some(self.history_query(low))
    }

    pub fn jump_tail(&mut self, cursor: &mut usize) -> Option<Query> {
        if self.mode == ScrollMode::Search {
            self.viewport.selected_seq = self.content.items.last().map(|entry| entry.seq);
            self.viewport.view_start = self.tail_view_start();
            *cursor = self.visible_items().len().saturating_sub(1);
            self.reconcile_view(cursor);
            return None;
        }

        let high = self.retained_bounds.1;
        self.mode = ScrollMode::Tail;
        self.viewport.selected_seq = (high != 0).then_some(high);
        self.viewport.view_start = self.tail_view_start();
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
        let viewport = self.viewport.height.max(1).saturating_mul(2);
        viewport.min(self.history_buffer_limit) as u64
    }

    fn tail_view_start(&self) -> usize {
        self.content
            .items
            .len()
            .saturating_sub(self.viewport.height)
    }

    fn set_retained_bounds(&mut self, retained_bounds: (u64, u64)) {
        self.retained_bounds = retained_bounds;
        self.store_stats.bounds = retained_bounds;
        self.store_stats.retained = retained_count(retained_bounds);
    }

    fn selected_index(&self) -> Option<usize> {
        let selected_seq = self.viewport.selected_seq?;
        self.content
            .items
            .iter()
            .position(|entry| entry.seq == selected_seq)
    }

    fn seq_at_cursor(&self, cursor: usize) -> Option<u64> {
        self.content
            .items
            .get(self.viewport.view_start.saturating_add(cursor))
            .map(|entry| entry.seq)
    }

    fn previous_seq(&self, selected: u64) -> Option<u64> {
        let index = self
            .content
            .items
            .iter()
            .position(|entry| entry.seq == selected)?;
        index.checked_sub(1).map(|idx| self.content.items[idx].seq)
    }

    fn next_seq(&self, selected: u64) -> Option<u64> {
        let index = self
            .content
            .items
            .iter()
            .position(|entry| entry.seq == selected)?;
        self.content.items.get(index + 1).map(|entry| entry.seq)
    }

    fn clamp_selected_seq(&mut self) {
        if self.content.items.is_empty() {
            self.viewport.selected_seq = None;
            return;
        }

        if let Some(selected_seq) = self.viewport.selected_seq {
            if self
                .content
                .items
                .iter()
                .any(|entry| entry.seq == selected_seq)
            {
                return;
            }

            self.viewport.selected_seq = self
                .content
                .items
                .iter()
                .min_by_key(|entry| entry.seq.abs_diff(selected_seq))
                .map(|entry| entry.seq);
            return;
        }

        self.viewport.selected_seq = self.content.items.last().map(|entry| entry.seq);
    }

    fn reconcile_fuzzy_selection(
        &mut self,
        previous_selected_seq: Option<u64>,
        previous_selected_index: Option<usize>,
        was_following_highest_rank: bool,
        cursor: &mut usize,
    ) {
        if self.content.items.is_empty() {
            self.viewport.selected_seq = None;
            self.viewport.view_start = 0;
            *cursor = 0;
            return;
        }

        if was_following_highest_rank {
            self.viewport.selected_seq = self.content.items.last().map(|entry| entry.seq);
            self.viewport.view_start = self.tail_view_start();
            *cursor = self.visible_items().len().saturating_sub(1);
            return;
        }

        if let Some(selected_seq) = previous_selected_seq
            && self
                .content
                .items
                .iter()
                .any(|entry| entry.seq == selected_seq)
        {
            self.viewport.selected_seq = Some(selected_seq);
            self.preserve_cursor_row(cursor);
            return;
        }

        let fallback_index = previous_selected_index
            .unwrap_or_else(|| self.content.items.len().saturating_sub(1))
            .min(self.content.items.len().saturating_sub(1));
        self.viewport.selected_seq = self
            .content
            .items
            .get(fallback_index)
            .map(|entry| entry.seq);
        self.preserve_cursor_row(cursor);
    }

    fn preserve_cursor_row(&mut self, cursor: &mut usize) {
        let Some(selected_index) = self.selected_index() else {
            self.viewport.view_start = self.tail_view_start();
            return;
        };

        let visible_height = self.viewport.height.max(1);
        let desired_row = (*cursor).min(visible_height.saturating_sub(1));
        let max_start = self.content.items.len().saturating_sub(visible_height);
        self.viewport.view_start = selected_index.saturating_sub(desired_row).min(max_start);
    }

    fn reconcile_view(&mut self, cursor: &mut usize) {
        if self.content.items.is_empty() || self.viewport.height == 0 {
            self.viewport.view_start = 0;
            *cursor = 0;
            return;
        }

        if self.mode == ScrollMode::Tail {
            self.viewport.view_start = self.tail_view_start();
            self.viewport.selected_seq = self.content.items.last().map(|entry| entry.seq);
        }

        let max_start = self
            .content
            .items
            .len()
            .saturating_sub(self.viewport.height);
        self.viewport.view_start = self.viewport.view_start.min(max_start);

        if let Some(selected_index) = self.selected_index() {
            if selected_index < self.viewport.view_start {
                self.viewport.view_start = selected_index;
            }

            let end = self
                .viewport
                .view_start
                .saturating_add(self.viewport.height)
                .min(self.content.items.len());
            if selected_index >= end {
                self.viewport.view_start = selected_index
                    .saturating_add(1)
                    .saturating_sub(self.viewport.height);
            }

            *cursor = selected_index.saturating_sub(self.viewport.view_start);
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
                producer: "fake".to_string(),
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

    fn tail_update(start: u64, end: u64, retained_bounds: (u64, u64)) -> LogPaneUpdate {
        LogPaneUpdate::Tail {
            entries: entries(start, end),
            retained_bounds,
        }
    }

    fn history_update(start: u64, end: u64, retained_bounds: (u64, u64)) -> LogPaneUpdate {
        LogPaneUpdate::History {
            entries: entries(start, end),
            retained_bounds,
        }
    }

    fn fuzzy_update(entries: Vec<Arc<LogEntry>>, retained_bounds: (u64, u64)) -> LogPaneUpdate {
        LogPaneUpdate::Fuzzy {
            best_first_entries: entries,
            retained_bounds,
            matches_by_seq: HashMap::new(),
        }
    }

    #[test]
    fn scrollbar_metrics_hide_when_state_is_not_ready() {
        let mut state = LogPaneState::new(500);
        state.viewport.height = 5;
        state.retained_bounds = (1, 10);

        assert_eq!(state.scrollbar_metrics(), None);

        state.viewport.selected_seq = Some(5);
        state.viewport.height = 0;

        assert_eq!(state.scrollbar_metrics(), None);

        state.viewport.height = 5;
        state.retained_bounds = (0, 0);

        assert_eq!(state.scrollbar_metrics(), None);
    }

    #[test]
    fn scrollbar_metrics_hide_when_content_fits() {
        let mut state = LogPaneState::new(500);
        state.viewport.height = 5;
        state.viewport.selected_seq = Some(3);
        state.retained_bounds = (1, 5);

        assert_eq!(state.scrollbar_metrics(), None);
    }

    #[test]
    fn scrollbar_metrics_clamp_selection_within_retained_bounds() {
        let mut state = LogPaneState::new(500);
        state.viewport.height = 4;
        state.retained_bounds = (10, 20);

        state.viewport.selected_seq = Some(10);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 11,
                viewport_content_length: 4,
                position: 0,
            })
        );

        state.viewport.selected_seq = Some(15);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 11,
                viewport_content_length: 4,
                position: 5,
            })
        );

        state.viewport.selected_seq = Some(25);
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
        state.viewport.height = 2;
        state.viewport.selected_seq = Some(u64::MAX);
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
    fn search_scrollbar_metrics_hide_when_results_fit_or_state_is_not_ready() {
        let mut state = LogPaneState::new(500);
        state.mode = ScrollMode::Search;
        state.viewport.height = 3;
        state.content.items = entries(1, 3);
        state.viewport.selected_seq = Some(3);

        assert_eq!(state.scrollbar_metrics(), None);

        state.content.items = entries(1, 4);
        state.viewport.selected_seq = None;

        assert_eq!(state.scrollbar_metrics(), None);

        state.viewport.selected_seq = Some(4);
        state.viewport.height = 0;

        assert_eq!(state.scrollbar_metrics(), None);
    }

    #[test]
    fn search_scrollbar_metrics_use_fuzzy_rank_domain() {
        let mut state = LogPaneState::new(500);
        state.mode = ScrollMode::Search;
        state.viewport.height = 3;
        state.content.items = entries(1, 8);

        state.viewport.selected_seq = Some(1);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 8,
                viewport_content_length: 3,
                position: 0,
            })
        );

        state.viewport.selected_seq = Some(4);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 8,
                viewport_content_length: 3,
                position: 3,
            })
        );

        state.viewport.selected_seq = Some(8);
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 8,
                viewport_content_length: 3,
                position: 7,
            })
        );
    }

    #[test]
    fn search_scrollbar_metrics_follow_sticky_selected_seq_after_rerank() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);
        state.apply_update(
            fuzzy_update(
                vec![entry(5), entry(4), entry(3), entry(2), entry(1)],
                (1, 5),
            ),
            &mut cursor,
        );
        state.viewport.selected_seq = Some(3);
        cursor = 2;

        state.apply_update(
            fuzzy_update(vec![entry(6), entry(3), entry(5), entry(1)], (1, 6)),
            &mut cursor,
        );

        assert_eq!(state.viewport.selected_seq, Some(3));
        assert_eq!(
            state
                .content
                .items
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1, 5, 3, 6]
        );
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 4,
                viewport_content_length: 3,
                position: 2,
            })
        );
    }

    #[test]
    fn search_scrollbar_metrics_stay_at_highest_rank_when_pinned() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(3), entry(2), entry(1)], (1, 3)),
            &mut cursor,
        );

        state.apply_update(
            fuzzy_update(vec![entry(4), entry(2), entry(3), entry(1)], (1, 4)),
            &mut cursor,
        );

        assert_eq!(state.viewport.selected_seq, Some(4));
        assert_eq!(
            state.scrollbar_metrics(),
            Some(ScrollbarMetrics {
                content_length: 4,
                viewport_content_length: 3,
                position: 3,
            })
        );
    }

    #[test]
    fn tail_results_pin_newest_visible_entry() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);

        state.apply_update(tail_update(1, 5, (1, 5)), &mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.viewport.selected_seq, Some(5));
        assert_eq!(state.viewport.view_start, 2);
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
        state.apply_update(tail_update(1, 10, (1, 10)), &mut cursor);

        let query = state.scroll_backward(&mut cursor);

        assert_eq!(state.mode, ScrollMode::History);
        assert_eq!(state.viewport.selected_seq, Some(9));
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
        state.viewport.selected_seq = Some(50);

        state.apply_update(history_update(45, 55, (1, 100)), &mut cursor);

        assert_eq!(state.viewport.selected_seq, Some(50));
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
        state.apply_update(tail_update(80, 100, (1, 100)), &mut cursor);

        let query = state.jump_head(&mut cursor);

        assert_eq!(state.mode, ScrollMode::History);
        assert_eq!(state.viewport.selected_seq, Some(1));
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
        state.content.items = entries(40, 60);
        state.viewport.selected_seq = Some(50);

        let query = state.jump_tail(&mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.viewport.selected_seq, Some(100));
        assert_eq!(query, Some(Query::Tail));
    }

    #[test]
    fn scrolling_down_to_retained_high_returns_to_tail() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.mode = ScrollMode::History;
        state.retained_bounds = (1, 10);
        state.content.items = entries(6, 10);
        state.viewport.selected_seq = Some(9);
        state.reconcile_view(&mut cursor);

        let query = state.scroll_forward(&mut cursor);

        assert_eq!(state.mode, ScrollMode::Tail);
        assert_eq!(state.viewport.selected_seq, Some(10));
        assert_eq!(query, Some(Query::Tail));
    }

    #[test]
    fn fuzzy_refresh_preserves_selected_seq_after_rerank() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(3), entry(2), entry(1)], (1, 3)),
            &mut cursor,
        );
        state.viewport.selected_seq = Some(2);
        cursor = 1;

        state.apply_update(
            fuzzy_update(vec![entry(2), entry(4), entry(1)], (1, 4)),
            &mut cursor,
        );

        assert_eq!(
            state
                .content
                .items
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1, 4, 2]
        );
        assert_eq!(state.viewport.selected_seq, Some(2));
        assert_eq!(cursor, 2);
    }

    #[test]
    fn fuzzy_refresh_falls_back_to_previous_rank_index_when_selected_seq_disappears() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(4), entry(3), entry(2), entry(1)], (1, 4)),
            &mut cursor,
        );
        state.viewport.selected_seq = Some(2);
        cursor = 1;

        state.apply_update(
            fuzzy_update(vec![entry(6), entry(5), entry(1)], (1, 6)),
            &mut cursor,
        );

        assert_eq!(
            state
                .content
                .items
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1, 5, 6]
        );
        assert_eq!(state.viewport.selected_seq, Some(5));
        assert_eq!(cursor, 1);
    }

    #[test]
    fn fuzzy_refresh_fallback_clamps_to_highest_rank_when_new_results_are_shorter() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(4, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(4), entry(3), entry(2), entry(1)], (1, 4)),
            &mut cursor,
        );
        state.viewport.selected_seq = Some(3);
        cursor = 2;

        state.apply_update(fuzzy_update(vec![entry(5), entry(1)], (1, 5)), &mut cursor);

        assert_eq!(
            state
                .content
                .items
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1, 5]
        );
        assert_eq!(state.viewport.selected_seq, Some(5));
        assert_eq!(cursor, 1);
    }

    #[test]
    fn fuzzy_refresh_empty_results_clear_selection() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(3), entry(2), entry(1)], (1, 3)),
            &mut cursor,
        );

        state.apply_update(fuzzy_update(Vec::new(), (1, 3)), &mut cursor);

        assert!(state.content.items.is_empty());
        assert_eq!(state.viewport.selected_seq, None);
        assert_eq!(state.viewport.view_start, 0);
        assert_eq!(cursor, 0);
        assert_eq!(state.selected_visible_index(), None);
    }

    #[test]
    fn fuzzy_refresh_follows_new_highest_rank_when_previous_selection_was_highest() {
        let mut state = LogPaneState::new(500);
        let mut cursor = 0;
        state.set_height(3, &mut cursor);
        state.apply_update(
            fuzzy_update(vec![entry(3), entry(2), entry(1)], (1, 3)),
            &mut cursor,
        );

        state.apply_update(
            fuzzy_update(vec![entry(4), entry(2), entry(3), entry(1)], (1, 4)),
            &mut cursor,
        );

        assert_eq!(
            state
                .content
                .items
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1, 3, 2, 4]
        );
        assert_eq!(state.viewport.selected_seq, Some(4));
        assert_eq!(
            state
                .visible_items()
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![3, 2, 4]
        );
        assert_eq!(cursor, 2);
    }
}
