//! A pane: one independent viewport over the shared log store.
//!
//! Every pane owns a source filter, an active search query, a seq-anchored
//! cursor, and the entries its search engine last delivered. Panes never
//! talk to each other; splits and tabs are just arrangements of panes.

use std::{collections::HashMap, sync::Arc};

use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tracing::{debug, error};

use crate::{
    event::{Match, PaneId, Query, SearchEvent, SearchProgress},
    log::{LogEntry, Source, SourceId},
};

/// When the cursor moves within this many rows of a stream window's edge,
/// the pane re-centers its history query to pre-fetch more context.
const PAGE_EDGE: usize = 8;

/// Width the source name is padded/truncated to in a rendered row.
pub const SOURCE_WIDTH: usize = 10;

/// Char offset where the message starts in [`row_text`]:
/// `HH:MM:SS` (8) + space + level (5) + space + source + `│ ` (2).
pub const MSG_CHAR_OFFSET: usize = 8 + 1 + 5 + 1 + SOURCE_WIDTH + 2;

/// The plain text of one rendered log row. This is the single source of
/// truth shared by rendering, column clamping, and yanking, so what you
/// select is exactly what you copy.
pub fn row_text(entry: &LogEntry) -> String {
    let source: String = entry
        .source
        .display_name
        .chars()
        .take(SOURCE_WIDTH)
        .collect();
    format!(
        "{} {:<5} {:<width$}│ {}",
        entry.ts.format("%H:%M:%S"),
        entry
            .level
            .map(|level| level.to_string())
            .unwrap_or_default(),
        source,
        entry.msg,
        width = SOURCE_WIDTH
    )
}

/// Everything a pane needs to (re)dispatch its search, threaded in from
/// `AppState` by the input/reducer layer so panes stay free of global state.
pub struct SearchCtx<'a> {
    /// Live sources, used to resolve filter patterns into source ids.
    pub sources: &'a [Source],
    pub tx: &'a mpsc::Sender<SearchEvent>,
    /// Per-side context size for history windows and tail window size.
    pub buffer: u64,
    /// Retained `(low, high)` store bounds at input time.
    pub bounds: (u64, u64),
}

/// What the pane is currently displaying.
pub enum View {
    /// A contiguous window of the filtered store (tail or history query).
    Stream { entries: Vec<Arc<LogEntry>> },
    /// Fuzzy match list, ascending by seq, with per-entry match spans.
    Results {
        entries: Vec<Arc<LogEntry>>,
        matches: HashMap<u64, Vec<Match>>,
        progress: Option<SearchProgress>,
        term: String,
    },
}

impl View {
    pub fn entries(&self) -> &[Arc<LogEntry>] {
        match self {
            View::Stream { entries } | View::Results { entries, .. } => entries,
        }
    }
}

pub struct Pane {
    pub id: PaneId,
    /// User filter patterns matched (case-insensitive substring) against a
    /// source's producer, group, display name, and id. Empty = all sources.
    pub filter: Vec<String>,
    pub view: View,
    /// Query currently driving this pane's search engine slot.
    pub active_query: Option<Query>,
    /// Cursor anchored to a store sequence id, never a row index, so ring
    /// eviction and live appends cannot silently move it between entries.
    pub cursor_seq: Option<u64>,
    /// Desired cursor column (char offset into [`row_text`]). Like vim,
    /// this is sticky across rows; clamp with [`Pane::effective_col`].
    pub cursor_col: usize,
    /// Follow the newest entry (TAIL). Broken by any cursor motion.
    pub follow: bool,
    /// Last confirmed search term; its hits drive `n`/`N`.
    pub last_search: Option<String>,
    /// Seq ids of the confirmed search's matches, ascending.
    pub hits: Vec<u64>,
    /// Cursor position to restore when a live search is abandoned.
    pub search_return_seq: Option<u64>,
    /// Center of the last dispatched history query, used to avoid
    /// re-dispatch bursts while a re-centered window is still in flight.
    last_history_center: Option<u64>,
    /// Shown when the pane intentionally has nothing to display.
    pub empty_note: Option<String>,
    /// Last rendered content area; drives paging sizes and directional focus.
    pub rect: Rect,
    /// First visible row offset, kept across renders for scroll stability.
    pub scroll: usize,
    /// Whether the detail overlay (full entry JSON) is open.
    pub detail_open: bool,
    pub detail_scroll: u16,
}

impl Pane {
    pub fn new(id: PaneId) -> Self {
        Self {
            id,
            filter: Vec::new(),
            view: View::Stream {
                entries: Vec::new(),
            },
            active_query: None,
            cursor_seq: None,
            cursor_col: 0,
            follow: true,
            last_search: None,
            hits: Vec::new(),
            search_return_seq: None,
            last_history_center: None,
            empty_note: None,
            rect: Rect::default(),
            scroll: 0,
            detail_open: false,
            detail_scroll: 0,
        }
    }

    /// Clone this pane's viewpoint into a new pane id (used by splits).
    pub fn clone_into(&self, id: PaneId) -> Self {
        Self {
            id,
            filter: self.filter.clone(),
            view: match &self.view {
                View::Stream { entries } => View::Stream {
                    entries: entries.clone(),
                },
                View::Results {
                    entries,
                    matches,
                    progress,
                    term,
                } => View::Results {
                    entries: entries.clone(),
                    matches: matches.clone(),
                    progress: *progress,
                    term: term.clone(),
                },
            },
            active_query: None,
            cursor_seq: self.cursor_seq,
            cursor_col: self.cursor_col,
            follow: self.follow,
            last_search: self.last_search.clone(),
            hits: self.hits.clone(),
            search_return_seq: None,
            last_history_center: None,
            empty_note: self.empty_note.clone(),
            rect: Rect::default(),
            scroll: self.scroll,
            detail_open: false,
            detail_scroll: 0,
        }
    }

    /// The entry under the cursor, if any.
    pub fn cursor_entry(&self) -> Option<&Arc<LogEntry>> {
        let seq = self.cursor_seq?;
        let entries = self.view.entries();
        let idx = nearest_index(entries, seq)?;
        (entries[idx].seq == seq).then(|| &entries[idx])
    }

    /// Index of the cursor within the current view, clamped to the nearest
    /// loaded entry.
    pub fn cursor_index(&self) -> Option<usize> {
        nearest_index(self.view.entries(), self.cursor_seq?)
    }

    /// Rows of log content the pane showed last render (paging unit).
    pub fn page_rows(&self) -> usize {
        (self.rect.height as usize).max(1)
    }

    // ---- dispatch ----------------------------------------------------

    /// Resolve the pane's filter patterns against live sources.
    ///
    /// Returns `Some(vec![])` (match-all wildcard) for an empty filter,
    /// `Some(ids)` for a filter with matches, and `None` when the filter
    /// matches no live source — the caller should show an empty state
    /// rather than silently searching everything.
    pub fn resolve_filter(&self, sources: &[Source]) -> Option<Vec<SourceId>> {
        if self.filter.is_empty() {
            return Some(Vec::new());
        }
        let pats: Vec<String> = self.filter.iter().map(|p| p.to_lowercase()).collect();
        let ids: Vec<SourceId> = sources
            .iter()
            .filter(|source| {
                pats.iter().any(|pat| {
                    source.id.to_lowercase().contains(pat)
                        || source.display_name.to_lowercase().contains(pat)
                        || source.producer.to_lowercase().contains(pat)
                        || source
                            .group
                            .as_deref()
                            .is_some_and(|g| g.to_lowercase().contains(pat))
                })
            })
            .map(|source| source.id.clone())
            .collect();
        (!ids.is_empty()).then_some(ids)
    }

    /// Dispatch `query` to this pane's search engine, applying the filter.
    pub fn dispatch(&mut self, query: Query, ctx: &SearchCtx) {
        let Some(sources) = self.resolve_filter(ctx.sources) else {
            self.empty_note = Some(format!(
                "no sources match :filter {}",
                self.filter.join(",")
            ));
            self.view = View::Stream {
                entries: Vec::new(),
            };
            self.active_query = None;
            if let Err(err) = ctx.tx.try_send(SearchEvent::Cancel { target: self.id }) {
                error!(
                    "failed to cancel search for filtered-out pane {}: {err}",
                    self.id
                );
            }
            return;
        };

        self.empty_note = None;
        if let Query::History { middle_seq_id, .. } = &query {
            self.last_history_center = Some(*middle_seq_id);
        } else {
            self.last_history_center = None;
        }
        self.active_query = Some(query.clone());
        debug!(pane = %self.id, ?query, "pane dispatching search");
        if let Err(err) = ctx.tx.try_send(SearchEvent::Search {
            target: self.id,
            query,
            sources,
        }) {
            error!("failed to dispatch search for pane {}: {err}", self.id);
        }
    }

    /// Dispatch the stream query this pane wants given its follow state.
    pub fn dispatch_stream(&mut self, ctx: &SearchCtx) {
        if self.follow {
            self.dispatch(Query::Tail, ctx);
        } else {
            let middle = self.cursor_seq.unwrap_or(ctx.bounds.1);
            self.dispatch(
                Query::History {
                    middle_seq_id: middle,
                    buffer: ctx.buffer,
                },
                ctx,
            );
        }
    }

    // ---- result application -------------------------------------------

    /// Apply a routed search result. Results for a query this pane is no
    /// longer running are dropped (the engine's request-id check already
    /// guards staleness; this guards query swaps within one input frame).
    pub fn apply_result(
        &mut self,
        query: &Query,
        mut entries: Vec<Arc<LogEntry>>,
        matches: HashMap<u64, Vec<Match>>,
        progress: Option<SearchProgress>,
        retained_bounds: (u64, u64),
    ) {
        if self.active_query.as_ref() != Some(query) {
            return;
        }
        let _ = retained_bounds;

        match query {
            Query::Fuzzy(term) => {
                // Fuzzy workers emit best-first; logs read better in time order.
                entries.sort_by_key(|entry| entry.seq);
                self.empty_note = entries
                    .is_empty()
                    .then(|| format!("no matches for `{term}`"));
                // Default the cursor to the most recent match.
                let cursor = self
                    .cursor_seq
                    .and_then(|seq| nearest_index(&entries, seq))
                    .or(entries.len().checked_sub(1));
                self.cursor_seq = cursor.map(|idx| entries[idx].seq);
                self.view = View::Results {
                    entries,
                    matches,
                    progress,
                    term: term.clone(),
                };
            }
            _ => {
                if self.follow {
                    self.cursor_seq = entries.last().map(|entry| entry.seq);
                } else {
                    self.cursor_seq = self
                        .cursor_seq
                        .and_then(|seq| nearest_index(&entries, seq))
                        .map(|idx| entries[idx].seq)
                        .or_else(|| entries.last().map(|entry| entry.seq));
                }
                if entries.is_empty() && self.empty_note.is_none() {
                    self.empty_note = Some("waiting for logs…".to_string());
                } else if !entries.is_empty() {
                    self.empty_note = None;
                }
                self.view = View::Stream { entries };
            }
        }
    }

    // ---- motions -------------------------------------------------------

    /// Move the cursor by `delta` rows within the current view. Any motion
    /// breaks follow; stream views re-page near window edges.
    pub fn move_cursor(&mut self, delta: i64, ctx: &SearchCtx) {
        let entries = self.view.entries();
        if entries.is_empty() {
            return;
        }
        let was_following = self.follow;
        self.follow = false;

        let idx = self
            .cursor_seq
            .and_then(|seq| nearest_index(entries, seq))
            .unwrap_or(entries.len() - 1);
        let new_idx = idx
            .saturating_add_signed(delta as isize)
            .min(entries.len() - 1);
        self.cursor_seq = Some(entries[new_idx].seq);

        // Breaking out of tail: anchor a history window on the cursor so the
        // view stops sliding underneath the user.
        if was_following {
            self.dispatch_stream(ctx);
            return;
        }
        self.maybe_repage(new_idx, ctx);
    }

    /// Jump the cursor to an absolute seq and center a window there.
    pub fn jump_to(&mut self, seq: u64, ctx: &SearchCtx) {
        self.follow = false;
        self.cursor_seq = Some(seq.clamp(ctx.bounds.0, ctx.bounds.1));
        self.dispatch(
            Query::History {
                middle_seq_id: self.cursor_seq.unwrap_or_default(),
                buffer: ctx.buffer,
            },
            ctx,
        );
    }

    pub fn goto_top(&mut self, ctx: &SearchCtx) {
        self.jump_to(ctx.bounds.0, ctx);
    }

    pub fn goto_bottom(&mut self, ctx: &SearchCtx) {
        self.jump_to(ctx.bounds.1, ctx);
    }

    /// Re-enter follow (TAIL) mode.
    pub fn enter_follow(&mut self, ctx: &SearchCtx) {
        self.follow = true;
        self.dispatch(Query::Tail, ctx);
    }

    /// Jump to the next/previous confirmed search hit. Returns `false` when
    /// there is no hit in that direction.
    pub fn jump_hit(&mut self, forward: bool, ctx: &SearchCtx) -> bool {
        if self.hits.is_empty() {
            return false;
        }
        let cursor = self.cursor_seq.unwrap_or(0);
        let next = if forward {
            self.hits.iter().copied().find(|&seq| seq > cursor)
        } else {
            self.hits.iter().rev().copied().find(|&seq| seq < cursor)
        };
        let Some(seq) = next else {
            return false;
        };
        match &self.view {
            // In a results view the hit list is the view itself; just move.
            View::Results { .. } => {
                self.cursor_seq = Some(seq);
                self.follow = false;
            }
            View::Stream { entries } => {
                // Avoid a re-dispatch when the hit is already loaded.
                let loaded = entries.first().map(|e| e.seq) <= Some(seq)
                    && entries.last().map(|e| e.seq) >= Some(seq);
                if loaded {
                    self.cursor_seq = Some(seq);
                    self.follow = false;
                } else {
                    self.jump_to(seq, ctx);
                }
            }
        }
        true
    }

    /// Re-center the history window when the cursor nears a window edge and
    /// more retained entries exist beyond it.
    fn maybe_repage(&mut self, idx: usize, ctx: &SearchCtx) {
        let View::Stream { entries } = &self.view else {
            return;
        };
        if entries.is_empty() {
            return;
        }
        let (low, high) = ctx.bounds;
        let first = entries[0].seq;
        let last = entries[entries.len() - 1].seq;
        let near_top = idx <= PAGE_EDGE && first > low;
        let near_bottom = idx + PAGE_EDGE >= entries.len().saturating_sub(1) && last < high;
        if !near_top && !near_bottom {
            return;
        }
        let center = self.cursor_seq.unwrap_or(last);
        // Skip while a re-centered window for (roughly) this spot is already
        // in flight — each arriving window resets the gap.
        if let Some(prev) = self.last_history_center
            && prev.abs_diff(center) < ctx.buffer / 2
        {
            return;
        }
        self.dispatch(
            Query::History {
                middle_seq_id: center,
                buffer: ctx.buffer,
            },
            ctx,
        );
    }

    // ---- search lifecycle ----------------------------------------------

    /// Begin a live fuzzy search: remember where to return on abandon.
    pub fn begin_search(&mut self) {
        self.search_return_seq = self.cursor_seq;
        self.follow = false;
    }

    /// Live-update the in-flight search term.
    pub fn update_search(&mut self, term: &str, ctx: &SearchCtx) {
        if term.is_empty() {
            self.view = View::Results {
                entries: Vec::new(),
                matches: HashMap::new(),
                progress: None,
                term: String::new(),
            };
            self.empty_note = Some("type to search".to_string());
            self.active_query = None;
            let _ = ctx.tx.try_send(SearchEvent::Cancel { target: self.id });
            return;
        }
        self.dispatch(Query::Fuzzy(term.to_string()), ctx);
    }

    /// Confirm the current results view: record hits for `n`/`N`.
    /// Returns the number of hits.
    pub fn confirm_search(&mut self) -> usize {
        if let View::Results { entries, term, .. } = &self.view {
            self.hits = entries.iter().map(|entry| entry.seq).collect();
            self.last_search = (!term.is_empty()).then(|| term.clone());
        }
        self.hits.len()
    }

    /// Abandon a live search and restore the stream where it was.
    pub fn abandon_search(&mut self, ctx: &SearchCtx) {
        self.follow = false;
        self.cursor_seq = self.search_return_seq.take().or(self.cursor_seq);
        let middle = self.cursor_seq.unwrap_or(ctx.bounds.1);
        self.dispatch(
            Query::History {
                middle_seq_id: middle,
                buffer: ctx.buffer,
            },
            ctx,
        );
    }

    /// Leave a results view back into the stream, centered on the cursor.
    pub fn results_to_stream(&mut self, ctx: &SearchCtx) {
        let middle = self.cursor_seq.unwrap_or(ctx.bounds.1);
        self.follow = false;
        self.dispatch(
            Query::History {
                middle_seq_id: middle,
                buffer: ctx.buffer,
            },
            ctx,
        );
    }

    /// Drop confirmed search state (`:clear` / `Esc` on a stream).
    pub fn clear_search(&mut self) {
        self.hits.clear();
        self.last_search = None;
    }

    /// Entries between `anchor` and the cursor inclusive (visual selection).
    pub fn selection(&self, anchor: u64) -> Vec<&Arc<LogEntry>> {
        let Some(cursor) = self.cursor_seq else {
            return Vec::new();
        };
        let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
        self.view
            .entries()
            .iter()
            .filter(|entry| entry.seq >= lo && entry.seq <= hi)
            .collect()
    }

    // ---- column cursor ---------------------------------------------------

    /// The desired column clamped to the cursor row's actual length.
    pub fn effective_col(&self) -> usize {
        let len = self.cursor_row_len();
        self.cursor_col.min(len.saturating_sub(1))
    }

    fn cursor_row_len(&self) -> usize {
        self.cursor_entry()
            .map(|entry| row_text(entry).chars().count())
            .unwrap_or(0)
    }

    /// Anchor the pane if it was following; column motions must not ride a
    /// sliding tail window.
    fn break_follow(&mut self, ctx: &SearchCtx) {
        if self.follow {
            self.follow = false;
            self.dispatch_stream(ctx);
        }
    }

    /// Move the cursor column by `delta` chars within the current row.
    pub fn move_col(&mut self, delta: i64, ctx: &SearchCtx) {
        self.break_follow(ctx);
        let len = self.cursor_row_len();
        self.cursor_col = self
            .effective_col()
            .saturating_add_signed(delta as isize)
            .min(len.saturating_sub(1));
    }

    /// `0` — jump to the first column.
    pub fn col_home(&mut self, ctx: &SearchCtx) {
        self.break_follow(ctx);
        self.cursor_col = 0;
    }

    /// `$` — jump to the last column of the current row.
    pub fn col_end(&mut self, ctx: &SearchCtx) {
        self.break_follow(ctx);
        self.cursor_col = self.cursor_row_len().saturating_sub(1);
    }

    /// `w` — start of the next whitespace-delimited word in the row.
    pub fn word_forward(&mut self, ctx: &SearchCtx) {
        self.break_follow(ctx);
        let Some(entry) = self.cursor_entry() else {
            return;
        };
        let chars: Vec<char> = row_text(entry).chars().collect();
        let mut col = self.effective_col();
        // Skip the rest of the current word, then the gap.
        while col < chars.len() && !chars[col].is_whitespace() {
            col += 1;
        }
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }
        self.cursor_col = col.min(chars.len().saturating_sub(1));
    }

    /// `b` — start of the previous whitespace-delimited word in the row.
    pub fn word_back(&mut self, ctx: &SearchCtx) {
        self.break_follow(ctx);
        let Some(entry) = self.cursor_entry() else {
            return;
        };
        let chars: Vec<char> = row_text(entry).chars().collect();
        let mut col = self.effective_col();
        // Step off a word start / out of the gap, then walk to the start.
        while col > 0 && chars[col.saturating_sub(1)].is_whitespace() {
            col -= 1;
        }
        while col > 0 && !chars[col - 1].is_whitespace() {
            col -= 1;
        }
        self.cursor_col = col;
    }

    // ---- charwise selection ----------------------------------------------

    /// The ordered `(start, end)` positions of a charwise selection, where a
    /// position is `(seq, col)`. `None` when the pane has no cursor.
    fn charwise_bounds(&self, anchor_seq: u64, anchor_col: usize) -> Option<ColRange> {
        let cursor = (self.cursor_seq?, self.effective_col());
        let anchor = (anchor_seq, anchor_col);
        Some(if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        })
    }

    /// Char range `(from, to_inclusive)` of `entry`'s row covered by the
    /// charwise selection, or `None` when the row is outside it.
    pub fn charwise_row_range(
        &self,
        entry: &LogEntry,
        anchor_seq: u64,
        anchor_col: usize,
    ) -> Option<(usize, usize)> {
        let (start, end) = self.charwise_bounds(anchor_seq, anchor_col)?;
        if entry.seq < start.0 || entry.seq > end.0 {
            return None;
        }
        let last = row_text(entry).chars().count().saturating_sub(1);
        let from = if entry.seq == start.0 {
            start.1.min(last)
        } else {
            0
        };
        let to = if entry.seq == end.0 {
            end.1.min(last)
        } else {
            last
        };
        Some((from, to))
    }

    /// Exactly the selected characters, rows joined with newlines.
    pub fn charwise_selection_text(&self, anchor_seq: u64, anchor_col: usize) -> String {
        let Some((start, end)) = self.charwise_bounds(anchor_seq, anchor_col) else {
            return String::new();
        };
        self.view
            .entries()
            .iter()
            .filter(|entry| entry.seq >= start.0 && entry.seq <= end.0)
            .filter_map(|entry| {
                let (from, to) = self.charwise_row_range(entry, anchor_seq, anchor_col)?;
                let text: String = row_text(entry)
                    .chars()
                    .skip(from)
                    .take(to + 1 - from)
                    .collect();
                Some(text)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Full rows of the linewise selection, exactly as rendered.
    pub fn linewise_selection_text(&self, anchor_seq: u64) -> String {
        self.selection(anchor_seq)
            .iter()
            .map(|entry| row_text(entry))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `((seq, col), (seq, col))` start/end positions of a charwise selection.
type ColRange = ((u64, usize), (u64, usize));

/// Index of the entry with seq closest to `seq` in an ascending slice.
pub fn nearest_index(entries: &[Arc<LogEntry>], seq: u64) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    match entries.binary_search_by_key(&seq, |entry| entry.seq) {
        Ok(idx) => Some(idx),
        Err(0) => Some(0),
        Err(idx) if idx >= entries.len() => Some(entries.len() - 1),
        Err(idx) => {
            // Pick whichever neighbor is numerically closer.
            let before = entries[idx - 1].seq;
            let after = entries[idx].seq;
            if seq - before <= after - seq {
                Some(idx - 1)
            } else {
                Some(idx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use tokio::sync::mpsc;

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
                display_name: "Source A".to_string(),
                group: Some("backend".to_string()),
            },
            fields: HashMap::new(),
        })
    }

    fn entries(seqs: &[u64]) -> Vec<Arc<LogEntry>> {
        seqs.iter().copied().map(entry).collect()
    }

    fn ctx_with(
        sources: &[Source],
        bounds: (u64, u64),
    ) -> (SearchCtx<'_>, mpsc::Receiver<SearchEvent>) {
        // Leak the channel sender so the ctx can borrow it; tests are short-lived.
        let (tx, rx) = mpsc::channel(64);
        let tx = Box::leak(Box::new(tx));
        (
            SearchCtx {
                sources,
                tx,
                buffer: 100,
                bounds,
            },
            rx,
        )
    }

    fn source(id: &str, group: Option<&str>) -> Source {
        Source {
            producer: "fake".to_string(),
            id: id.to_string(),
            display_name: format!("Source {id}"),
            group: group.map(str::to_string),
        }
    }

    #[test]
    fn nearest_index_picks_exact_and_neighbors() {
        let list = entries(&[10, 20, 30]);
        assert_eq!(nearest_index(&list, 20), Some(1));
        assert_eq!(nearest_index(&list, 1), Some(0));
        assert_eq!(nearest_index(&list, 99), Some(2));
        assert_eq!(nearest_index(&list, 24), Some(1));
        assert_eq!(nearest_index(&list, 26), Some(2));
        assert_eq!(nearest_index(&[], 5), None);
    }

    #[test]
    fn tail_result_pins_cursor_to_newest_while_following() {
        let mut pane = Pane::new(PaneId(1));
        pane.active_query = Some(Query::Tail);
        pane.apply_result(
            &Query::Tail,
            entries(&[1, 2, 3]),
            HashMap::new(),
            None,
            (1, 3),
        );
        assert_eq!(pane.cursor_seq, Some(3));

        pane.apply_result(
            &Query::Tail,
            entries(&[2, 3, 4]),
            HashMap::new(),
            None,
            (2, 4),
        );
        assert_eq!(pane.cursor_seq, Some(4));
    }

    #[test]
    fn history_result_clamps_cursor_after_eviction() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        pane.cursor_seq = Some(5);
        let query = Query::History {
            middle_seq_id: 5,
            buffer: 10,
        };
        pane.active_query = Some(query.clone());
        // Seq 5 was evicted; cursor should clamp to the nearest survivor.
        pane.apply_result(&query, entries(&[7, 8, 9]), HashMap::new(), None, (7, 9));
        assert_eq!(pane.cursor_seq, Some(7));
    }

    #[test]
    fn stale_query_results_are_dropped() {
        let mut pane = Pane::new(PaneId(1));
        pane.active_query = Some(Query::Tail);
        pane.apply_result(
            &Query::Fuzzy("err".to_string()),
            entries(&[1]),
            HashMap::new(),
            None,
            (1, 1),
        );
        assert!(pane.view.entries().is_empty());
        assert!(matches!(pane.view, View::Stream { .. }));
    }

    #[test]
    fn fuzzy_result_sorts_by_seq_and_cursors_latest() {
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy("entry".to_string());
        pane.active_query = Some(query.clone());
        pane.apply_result(&query, entries(&[9, 2, 5]), HashMap::new(), None, (1, 9));

        let seqs: Vec<u64> = pane.view.entries().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 5, 9]);
        assert_eq!(pane.cursor_seq, Some(9));
    }

    #[test]
    fn move_cursor_breaks_follow_and_anchors_history() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 3));
        let mut pane = Pane::new(PaneId(1));
        pane.active_query = Some(Query::Tail);
        pane.apply_result(
            &Query::Tail,
            entries(&[1, 2, 3]),
            HashMap::new(),
            None,
            (1, 3),
        );

        pane.move_cursor(-1, &ctx);

        assert!(!pane.follow);
        assert_eq!(pane.cursor_seq, Some(2));
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::History { middle_seq_id, .. },
                ..
            } => assert_eq!(middle_seq_id, 2),
            event => panic!("expected history dispatch, got {event:?}"),
        }
    }

    #[test]
    fn move_cursor_clamps_at_ends() {
        let srcs = [source("src-a", None)];
        let (ctx, _rx) = ctx_with(&srcs, (1, 3));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let query = Query::History {
            middle_seq_id: 2,
            buffer: 100,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(&query, entries(&[1, 2, 3]), HashMap::new(), None, (1, 3));
        pane.cursor_seq = Some(3);

        pane.move_cursor(10, &ctx);
        assert_eq!(pane.cursor_seq, Some(3));
        pane.move_cursor(-10, &ctx);
        assert_eq!(pane.cursor_seq, Some(1));
    }

    #[test]
    fn repage_triggers_near_edge_only_when_more_is_retained() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 200));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let seqs: Vec<u64> = (50..=150).collect();
        let query = Query::History {
            middle_seq_id: 100,
            buffer: 100,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(&query, entries(&seqs), HashMap::new(), None, (1, 200));
        // apply_result kept cursor at latest; drag it near the top edge.
        pane.cursor_seq = Some(55);

        pane.move_cursor(-1, &ctx);

        match rx.try_recv().expect("repage dispatch") {
            SearchEvent::Search {
                query: Query::History { middle_seq_id, .. },
                ..
            } => assert_eq!(middle_seq_id, 54),
            event => panic!("expected history repage, got {event:?}"),
        }
        // A second nudge while the window is in flight must not re-dispatch.
        pane.move_cursor(-1, &ctx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn filter_resolution_matches_name_group_and_producer() {
        let sources = [
            source("api-1", Some("backend")),
            source("web-1", Some("frontend")),
        ];
        let mut pane = Pane::new(PaneId(1));

        pane.filter = vec!["backend".to_string()];
        assert_eq!(
            pane.resolve_filter(&sources),
            Some(vec!["api-1".to_string()])
        );

        pane.filter = vec!["web".to_string()];
        assert_eq!(
            pane.resolve_filter(&sources),
            Some(vec!["web-1".to_string()])
        );

        pane.filter = vec!["nope".to_string()];
        assert_eq!(pane.resolve_filter(&sources), None);

        pane.filter.clear();
        assert_eq!(pane.resolve_filter(&sources), Some(Vec::new()));
    }

    #[test]
    fn unresolvable_filter_empties_pane_and_cancels_engine() {
        let sources = [source("api-1", Some("backend"))];
        let (ctx, mut rx) = ctx_with(&sources, (1, 10));
        let mut pane = Pane::new(PaneId(1));
        pane.filter = vec!["nope".to_string()];

        pane.dispatch(Query::Tail, &ctx);

        assert!(
            pane.empty_note
                .as_deref()
                .unwrap_or("")
                .contains("no sources match")
        );
        assert!(pane.active_query.is_none());
        assert!(matches!(
            rx.try_recv().expect("cancel"),
            SearchEvent::Cancel { .. }
        ));
    }

    #[test]
    fn confirm_search_records_hits_and_jump_hit_walks_them() {
        let srcs = [source("src-a", None)];
        let (ctx, _rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy("entry".to_string());
        pane.active_query = Some(query.clone());
        pane.apply_result(&query, entries(&[2, 5, 9]), HashMap::new(), None, (1, 9));

        assert_eq!(pane.confirm_search(), 3);
        assert_eq!(pane.hits, vec![2, 5, 9]);

        pane.cursor_seq = Some(5);
        assert!(pane.jump_hit(true, &ctx));
        assert_eq!(pane.cursor_seq, Some(9));
        assert!(!pane.jump_hit(true, &ctx));
        assert!(pane.jump_hit(false, &ctx));
        assert_eq!(pane.cursor_seq, Some(5));
    }

    #[test]
    fn selection_spans_anchor_to_cursor_inclusive() {
        let mut pane = Pane::new(PaneId(1));
        let query = Query::History {
            middle_seq_id: 3,
            buffer: 10,
        };
        pane.follow = false;
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[1, 2, 3, 4, 5]),
            HashMap::new(),
            None,
            (1, 5),
        );
        pane.cursor_seq = Some(2);

        let selected: Vec<u64> = pane.selection(4).iter().map(|e| e.seq).collect();
        assert_eq!(selected, vec![2, 3, 4]);
    }
}
