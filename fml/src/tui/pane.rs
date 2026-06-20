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

/// Scroll position of a pane's log view.
///
/// Kept typed so the wrap-off (entry-index) and wrap-on (display-row) meanings
/// can never be silently confused when toggling wrap, cloning on split, or
/// reading the anchor in the renderer. The renderer normalizes the anchor to
/// the pane's current `line_wrap` mode each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAnchor {
    /// Wrap off: index of the first visible entry.
    Entry(usize),
    /// Wrap on: the first visible display row, as the entry index plus the
    /// display-row offset within that entry.
    Display { entry: usize, row: usize },
}

impl ScrollAnchor {
    /// The first visible entry index, regardless of variant.
    pub fn entry(self) -> usize {
        match self {
            Self::Entry(entry) | Self::Display { entry, .. } => entry,
        }
    }
}

impl Default for ScrollAnchor {
    fn default() -> Self {
        Self::Entry(0)
    }
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
    /// Full match list of the in-flight (unconfirmed) search, ascending.
    /// Uncapped, unlike the displayed results.
    live_hit_seqs: Vec<u64>,
    /// A non-following bounded fuzzy search is mid-handoff: the displayed
    /// entries are held (only progress updates render) until that bounded
    /// request's accepted `complete = true` emission replaces them. Cleared
    /// by every new dispatch so a superseding query is never swallowed.
    holding_for_complete: bool,
    /// Cursor position to restore when a live search is abandoned.
    pub search_return_seq: Option<u64>,
    /// Center of the last dispatched history query, used to avoid
    /// re-dispatch bursts while a re-centered window is still in flight.
    last_history_center: Option<u64>,
    /// Shown when the pane intentionally has nothing to display.
    pub empty_note: Option<String>,
    /// Last rendered content area; drives paging sizes and directional focus.
    pub rect: Rect,
    /// Scroll position, kept across renders for stability. Typed so the
    /// wrap-off (entry) and wrap-on (display-row) meanings never collide.
    pub scroll: ScrollAnchor,
    /// Wrap long entries across continuation rows (per-pane runtime toggle).
    /// Seeded from `TuiConfig.line_wrap`; flipped by `:wrap`/`W`.
    pub line_wrap: bool,
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
            live_hit_seqs: Vec::new(),
            holding_for_complete: false,
            search_return_seq: None,
            last_history_center: None,
            empty_note: None,
            rect: Rect::default(),
            scroll: ScrollAnchor::default(),
            line_wrap: false,
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
            // Copy the active query so the freeze bound (carried in
            // `until_seq`) rides into the split; the clone keeps the frozen
            // view until the user interacts.
            active_query: self.active_query.clone(),
            cursor_seq: self.cursor_seq,
            cursor_col: self.cursor_col,
            follow: self.follow,
            last_search: self.last_search.clone(),
            hits: self.hits.clone(),
            live_hit_seqs: self.live_hit_seqs.clone(),
            // Results route by pane id, so a clone mid-hold would never get
            // the release emission; start it clear.
            holding_for_complete: false,
            search_return_seq: None,
            last_history_center: None,
            empty_note: self.empty_note.clone(),
            rect: Rect::default(),
            scroll: self.scroll,
            // Splits inherit the current pane's wrap state.
            line_wrap: self.line_wrap,
            detail_open: false,
            detail_scroll: 0,
        }
    }

    /// Flip line wrapping for this pane. Splits inherit the new state via
    /// [`Pane::clone_into`]; the renderer normalizes the scroll anchor to the
    /// new mode on the next frame.
    pub fn toggle_line_wrap(&mut self) {
        self.line_wrap = !self.line_wrap;
        // Turning wrap on while following: snap so the newest entry shows its
        // last display row rather than wrapping a tall entry from row 0.
        if self.line_wrap && self.follow {
            self.snap_col_to_row_end();
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
        let ids: Vec<SourceId> = sources
            .iter()
            .filter(|source| self.filter.iter().any(|pat| pattern_matches(pat, source)))
            .map(|source| source.id.clone())
            .collect();
        (!ids.is_empty()).then_some(ids)
    }

    /// Dispatch `query` to this pane's search engine, applying the filter.
    pub fn dispatch(&mut self, query: Query, ctx: &SearchCtx) {
        // Any new dispatch supersedes a held freeze handoff; clearing here
        // covers every path (typing, F, :refresh, Esc, filter changes) so a
        // superseding query's partial emissions are never silently swallowed.
        self.holding_for_complete = false;
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
    ///
    /// `hit_seqs` is the full uncapped match list for fuzzy queries —
    /// displayed entries are only the top-scored subset, but `n`/`N`
    /// navigation should walk every match.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_result(
        &mut self,
        query: &Query,
        mut entries: Vec<Arc<LogEntry>>,
        matches: HashMap<u64, Vec<Match>>,
        hit_seqs: Option<Vec<u64>>,
        progress: Option<SearchProgress>,
        complete: bool,
        retained_bounds: (u64, u64),
    ) {
        if self.active_query.as_ref() != Some(query) {
            return;
        }
        let _ = retained_bounds;

        // Frozen handoff: while holding for the bounded freeze request's
        // completion, render scan progress but keep the held entries (and the
        // pre-freeze hit list) until the accepted `complete = true` arrives.
        // `holding_for_complete` is only ever set while `active_query` is the
        // bounded freeze query, so any emission reaching here is for it.
        if self.holding_for_complete {
            if let View::Results { progress: p, .. } = &mut self.view {
                *p = progress;
            }
            if !complete {
                return;
            }
            self.holding_for_complete = false;
        }

        if let Some(hit_seqs) = hit_seqs {
            self.live_hit_seqs = hit_seqs;
        }

        match query {
            Query::Fuzzy { term, .. } => {
                // Fuzzy workers emit best-first; logs read better in time order.
                entries.sort_by_key(|entry| entry.seq);
                self.empty_note = entries
                    .is_empty()
                    .then(|| format!("no matches for `{term}`"));
                if self.follow {
                    // Live search: ride the newest match like a tail stream so
                    // the cursor stays on the freshest hit as results re-rank.
                    self.cursor_seq = entries.last().map(|entry| entry.seq);
                    // Keep the cursor on the newest entry's last display row so
                    // wrapping a tall fresh match shows its tail, not row 0.
                    self.snap_col_to_row_end();
                } else {
                    // Frozen search: keep the cursor on its row when the entry
                    // is still present, else fall back to the nearest match.
                    let cursor = self
                        .cursor_seq
                        .and_then(|seq| nearest_index(&entries, seq))
                        .or(entries.len().checked_sub(1));
                    self.cursor_seq = cursor.map(|idx| entries[idx].seq);
                }
                // A confirmed live search keeps its hit list growing with each
                // new emission, so a later freeze inherits post-confirm matches.
                if self.follow && self.last_search.as_deref() == Some(term.as_str()) {
                    self.hits = self.live_hit_seqs.clone();
                }
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
                    // Tail/startup: land on the newest entry's last display row
                    // when wrapping so a tall fresh entry isn't clipped at row 0.
                    self.snap_col_to_row_end();
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

    /// Move the cursor by `delta` rows within the current view. Real movement
    /// (the cursor index actually changes) breaks follow; a clamped no-op
    /// keeps the view live so an underfilled tail/results view keeps updating.
    /// Stream views re-page near window edges.
    pub fn move_cursor(&mut self, delta: i64, ctx: &SearchCtx) {
        let entries = self.view.entries();
        if entries.is_empty() {
            return;
        }
        let idx = self
            .cursor_seq
            .and_then(|seq| nearest_index(entries, seq))
            .unwrap_or(entries.len() - 1);
        let new_idx = idx
            .saturating_add_signed(delta as isize)
            .min(entries.len() - 1);
        // The cursor can't move (0–1 rows, or already at the boundary): no
        // freeze, stay live.
        if new_idx == idx {
            return;
        }
        let new_seq = entries[new_idx].seq;
        let was_following = self.follow;
        self.cursor_seq = Some(new_seq);

        if was_following {
            // Breaking out of tail: freeze a live search in place, or anchor a
            // history window on the cursor so a stream stops sliding.
            match &self.view {
                View::Results { .. } => self.freeze_search(ctx),
                View::Stream { .. } => {
                    self.follow = false;
                    self.dispatch_stream(ctx);
                }
            }
            return;
        }
        self.maybe_repage(new_idx, ctx);
    }

    /// Freeze a live (following) results view: capture the store high bound,
    /// trim the displayed list to it for instant stability, dispatch the
    /// bounded query, and hold the trimmed view until that request's
    /// `complete` emission replaces it. No-op outside a results view.
    fn freeze_search(&mut self, ctx: &SearchCtx) {
        let bound = ctx.bounds.1;
        let term = match &mut self.view {
            View::Results { term, entries, .. } => {
                entries.retain(|entry| entry.seq <= bound);
                term.clone()
            }
            View::Stream { .. } => return,
        };
        self.follow = false;
        self.dispatch(
            Query::Fuzzy {
                term,
                until_seq: Some(bound),
            },
            ctx,
        );
        // Set after dispatch, which clears it.
        self.holding_for_complete = true;
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
        self.snap_col_to_row_end();
        self.jump_to(ctx.bounds.1, ctx);
    }

    /// When wrapping, park the sticky column past the end of the line so the
    /// cursor lands on the newest entry's *last* display row — otherwise tail
    /// would show the first display row of a tall newest entry and clip the
    /// rest. The effective column clamps to the row's last char, so a sentinel
    /// is enough and stays correct as the newest entry changes. No-op with
    /// wrap off, keeping that render path byte-for-byte unchanged.
    fn snap_col_to_row_end(&mut self) {
        if self.line_wrap {
            self.cursor_col = usize::MAX;
        }
    }

    /// Re-enter follow (TAIL) mode. View-aware: on an active fuzzy results
    /// view it goes live on that search (unbounded, pinned to the newest
    /// matches, staying in results); on a plain stream it tails the stream.
    pub fn enter_follow(&mut self, ctx: &SearchCtx) {
        self.follow = true;
        self.snap_col_to_row_end();
        let fuzzy_term = match &self.view {
            View::Results { term, .. } if !term.is_empty() => Some(term.clone()),
            _ => None,
        };
        match fuzzy_term {
            Some(term) => self.dispatch(
                Query::Fuzzy {
                    term,
                    until_seq: None,
                },
                ctx,
            ),
            None => self.dispatch(Query::Tail, ctx),
        }
    }

    /// Re-snapshot a frozen fuzzy search at the current store high seq: one
    /// bounded re-rank, then stable again, staying in normal mode. Returns
    /// `false` without dispatching when there is nothing to refresh — the
    /// pane is following (already live) or has no active fuzzy search.
    pub fn refresh_search(&mut self, ctx: &SearchCtx) -> bool {
        if self.follow {
            return false;
        }
        let term = match &self.view {
            View::Results { term, .. } if !term.is_empty() => term.clone(),
            _ => return false,
        };
        self.dispatch(
            Query::Fuzzy {
                term,
                until_seq: Some(ctx.bounds.1),
            },
            ctx,
        );
        // Set after dispatch, which clears it.
        self.holding_for_complete = true;
        true
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
                let was_following = self.follow;
                self.cursor_seq = Some(seq);
                if was_following {
                    // First n/N out of a live results view is cursor motion:
                    // freeze at jump time so the live worker stops re-ranking.
                    self.freeze_search(ctx);
                }
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

    /// Begin a fuzzy search: remember where to return on abandon. Follow is
    /// inherited — a search started from TAIL stays live, one started from a
    /// scrolled (frozen) pane stays bounded — so the live/frozen split is the
    /// pane's existing follow boundary.
    pub fn begin_search(&mut self) {
        self.search_return_seq = self.cursor_seq;
    }

    /// Live-update the in-flight search term. A following pane stays live
    /// (unbounded, re-ranking every tick); a non-following pane dispatches a
    /// fresh bounded query per keystroke and holds the displayed entries until
    /// that query completes, so frozen results never flicker between terms.
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
            self.holding_for_complete = false;
            let _ = ctx.tx.try_send(SearchEvent::Cancel { target: self.id });
            return;
        }
        let until_seq = (!self.follow).then_some(ctx.bounds.1);
        let hold = until_seq.is_some();
        self.dispatch(
            Query::Fuzzy {
                term: term.to_string(),
                until_seq,
            },
            ctx,
        );
        // Set after dispatch, which clears it.
        self.holding_for_complete = hold;
    }

    /// Confirm the current results view: record hits for `n`/`N`.
    /// Returns the number of hits.
    pub fn confirm_search(&mut self) -> usize {
        if let View::Results { entries, term, .. } = &self.view {
            // The worker's full match list covers hits beyond the displayed
            // top-scored subset; fall back to displayed entries when the
            // result predates split emission (e.g. unit-injected results).
            self.hits = if self.live_hit_seqs.is_empty() {
                entries.iter().map(|entry| entry.seq).collect()
            } else {
                self.live_hit_seqs.clone()
            };
            self.last_search = (!term.is_empty()).then(|| term.clone());
        }
        self.hits.len()
    }

    /// Abandon a search and restore the stream where it was. Follow-aware:
    /// abandoning a live search returns to the tailing stream (follow kept),
    /// while abandoning a frozen search restores the anchored position. Both
    /// dispatch a stream query, which also tears down the fuzzy worker via
    /// the engine's latest-wins abort.
    pub fn abandon_search(&mut self, ctx: &SearchCtx) {
        self.cursor_seq = self.search_return_seq.take().or(self.cursor_seq);
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

    /// Anchor a following stream before a column motion so it stops sliding.
    /// Column motion stays within the cursor row, so it never freezes a live
    /// results view — only row motion / `n`/`N` do that.
    fn break_follow(&mut self, ctx: &SearchCtx) {
        if self.follow && matches!(self.view, View::Stream { .. }) {
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

/// Whether one filter pattern matches a source.
///
/// A leading `=` requires a case-insensitive exact match on the source's
/// display name or id (the form the source picker writes, so `=Demo 1`
/// cannot also catch a future `Demo 10`). Anything else is a
/// case-insensitive substring match against the source's id, display name,
/// producer, and group.
pub fn pattern_matches(pattern: &str, source: &Source) -> bool {
    if let Some(exact) = pattern.strip_prefix('=') {
        return source.display_name.eq_ignore_ascii_case(exact)
            || source.id.eq_ignore_ascii_case(exact);
    }
    let pat = pattern.to_lowercase();
    source.id.to_lowercase().contains(&pat)
        || source.display_name.to_lowercase().contains(&pat)
        || source.producer.to_lowercase().contains(&pat)
        || source
            .group
            .as_deref()
            .is_some_and(|group| group.to_lowercase().contains(&pat))
}

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
            None,
            true,
            (1, 3),
        );
        assert_eq!(pane.cursor_seq, Some(3));

        pane.apply_result(
            &Query::Tail,
            entries(&[2, 3, 4]),
            HashMap::new(),
            None,
            None,
            true,
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
        pane.apply_result(
            &query,
            entries(&[7, 8, 9]),
            HashMap::new(),
            None,
            None,
            true,
            (7, 9),
        );
        assert_eq!(pane.cursor_seq, Some(7));
    }

    #[test]
    fn stale_query_results_are_dropped() {
        let mut pane = Pane::new(PaneId(1));
        pane.active_query = Some(Query::Tail);
        pane.apply_result(
            &Query::Fuzzy {
                term: "err".to_string(),
                until_seq: None,
            },
            entries(&[1]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 1),
        );
        assert!(pane.view.entries().is_empty());
        assert!(matches!(pane.view, View::Stream { .. }));
    }

    #[test]
    fn fuzzy_result_sorts_by_seq_and_cursors_latest() {
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy {
            term: "entry".to_string(),
            until_seq: None,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[9, 2, 5]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 9),
        );

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
            None,
            true,
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
        pane.apply_result(
            &query,
            entries(&[1, 2, 3]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 3),
        );
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
        pane.apply_result(
            &query,
            entries(&seqs),
            HashMap::new(),
            None,
            None,
            true,
            (1, 200),
        );
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
    fn confirm_search_prefers_full_hit_list_over_displayed_entries() {
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy {
            term: "entry".to_string(),
            until_seq: None,
        };
        pane.active_query = Some(query.clone());
        // Display shows only seq 9, but the worker reported three matches.
        pane.apply_result(
            &query,
            entries(&[9]),
            HashMap::new(),
            Some(vec![2, 5, 9]),
            None,
            true,
            (1, 9),
        );

        assert_eq!(pane.confirm_search(), 3);
        assert_eq!(pane.hits, vec![2, 5, 9]);
    }

    #[test]
    fn confirm_search_records_hits_and_jump_hit_walks_them() {
        let srcs = [source("src-a", None)];
        let (ctx, _rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy {
            term: "entry".to_string(),
            until_seq: None,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[2, 5, 9]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 9),
        );

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
    fn following_pane_search_dispatches_unbounded_fuzzy() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        assert!(pane.follow);

        pane.begin_search();
        pane.update_search("err", &ctx);

        assert!(pane.follow, "search from TAIL stays live");
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy { until_seq, .. },
                ..
            } => assert_eq!(until_seq, None, "live search is unbounded"),
            event => panic!("expected fuzzy dispatch, got {event:?}"),
        }
    }

    #[test]
    fn non_following_pane_search_dispatches_bounded_fuzzy() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;

        pane.begin_search();
        pane.update_search("err", &ctx);

        assert!(!pane.follow);
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy { until_seq, .. },
                ..
            } => assert_eq!(until_seq, Some(9), "frozen search is bounded at high seq"),
            event => panic!("expected bounded fuzzy dispatch, got {event:?}"),
        }
    }

    #[test]
    fn confirmed_live_search_extends_hits_on_new_emission() {
        let mut pane = Pane::new(PaneId(1));
        assert!(pane.follow);
        let query = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: None,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[2, 4]),
            HashMap::new(),
            Some(vec![2, 4]),
            None,
            true,
            (1, 5),
        );
        assert_eq!(pane.confirm_search(), 2);
        assert_eq!(pane.hits, vec![2, 4]);

        // A later live emission adds a match; the confirmed hit list grows.
        pane.apply_result(
            &query,
            entries(&[2, 4, 6]),
            HashMap::new(),
            Some(vec![2, 4, 6]),
            None,
            true,
            (1, 6),
        );
        assert_eq!(pane.hits, vec![2, 4, 6]);
    }

    #[test]
    fn confirm_frozen_search_keeps_bounded_query_and_follow_off() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let query = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: Some(5),
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[2, 4]),
            HashMap::new(),
            Some(vec![2, 4]),
            None,
            true,
            (1, 5),
        );

        assert_eq!(pane.confirm_search(), 2);
        assert!(!pane.follow, "confirming a frozen search does not go live");
        assert_eq!(pane.active_query, Some(query), "bounded query is preserved");
    }

    #[test]
    fn abandon_live_search_restores_tail_keeping_follow() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        assert!(pane.follow);
        pane.begin_search();
        pane.update_search("err", &ctx);
        let _ = rx.try_recv().expect("fuzzy dispatch");

        pane.abandon_search(&ctx);

        assert!(pane.follow, "abandoning a live search stays in TAIL");
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::Tail, ..
            } => {}
            event => panic!("expected tail dispatch, got {event:?}"),
        }
    }

    /// Sets up a following pane sitting on a live fuzzy results view.
    fn live_results_pane(seqs: &[u64], high: u64) -> Pane {
        let mut pane = Pane::new(PaneId(1));
        let query = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: None,
        };
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(seqs),
            HashMap::new(),
            Some(seqs.to_vec()),
            None,
            true,
            (1, high),
        );
        pane
    }

    #[test]
    fn scroll_up_in_live_results_freezes_at_high_bound() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = live_results_pane(&[3, 6, 9], 9);
        assert!(pane.follow);
        assert_eq!(pane.cursor_seq, Some(9));

        pane.move_cursor(-1, &ctx);

        assert!(!pane.follow, "real movement freezes the live search");
        assert!(pane.holding_for_complete, "held until the bounded complete");
        assert_eq!(
            pane.active_query,
            Some(Query::Fuzzy {
                term: "e".to_string(),
                until_seq: Some(9)
            })
        );
        match rx.try_recv().expect("freeze dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy { term, until_seq },
                ..
            } => {
                assert_eq!(term, "e");
                assert_eq!(until_seq, Some(9), "bound captured at jump time");
            }
            event => panic!("expected bounded fuzzy, got {event:?}"),
        }
    }

    #[test]
    fn unmovable_scroll_in_live_results_stays_live() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = live_results_pane(&[9], 9);
        assert!(pane.follow);

        // A single match means the cursor can't move: no freeze, no dispatch.
        pane.move_cursor(-1, &ctx);

        assert!(pane.follow, "underfilled live view keeps following");
        assert!(rx.try_recv().is_err(), "no dispatch on an unmovable cursor");
    }

    #[test]
    fn first_jump_hit_from_live_results_freezes() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = live_results_pane(&[3, 6, 9], 9);
        pane.confirm_search();
        pane.cursor_seq = Some(3);

        assert!(pane.jump_hit(true, &ctx));

        assert_eq!(pane.cursor_seq, Some(6));
        assert!(!pane.follow, "first n/N freezes the live search");
        assert!(pane.holding_for_complete);
        match rx.try_recv().expect("freeze dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy {
                    until_seq: Some(9), ..
                },
                ..
            } => {}
            event => panic!("expected bounded fuzzy, got {event:?}"),
        }
    }

    #[test]
    fn freeze_excludes_entries_appended_after_the_bound() {
        let srcs = [source("src-a", None)];
        // Bound is captured from ctx.bounds at jump time (high = 9).
        let (ctx, mut rx) = ctx_with(&srcs, (1, 9));
        let mut pane = live_results_pane(&[3, 6, 9], 9);

        pane.move_cursor(-1, &ctx);

        match rx.try_recv().expect("freeze dispatch") {
            SearchEvent::Search {
                query:
                    Query::Fuzzy {
                        until_seq: Some(bound),
                        ..
                    },
                ..
            } => assert_eq!(bound, 9, "newer entries (seq > 9) are excluded"),
            event => panic!("expected bounded fuzzy, got {event:?}"),
        }
    }

    #[test]
    fn stale_unbounded_emission_after_freeze_is_dropped() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        // Frozen: active query is bounded.
        pane.active_query = Some(Query::Fuzzy {
            term: "err".to_string(),
            until_seq: Some(100),
        });
        // A late emission from the torn-down live worker (unbounded query).
        pane.apply_result(
            &Query::Fuzzy {
                term: "err".to_string(),
                until_seq: None,
            },
            entries(&[1, 2, 3]),
            HashMap::new(),
            Some(vec![1, 2, 3]),
            None,
            true,
            (1, 200),
        );
        assert!(
            pane.view.entries().is_empty(),
            "an unbounded emission must not land on a frozen pane"
        );
    }

    #[test]
    fn same_term_different_bound_emission_is_rejected() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        pane.active_query = Some(Query::Fuzzy {
            term: "err".to_string(),
            until_seq: Some(150),
        });
        // An older-bounded result for the same term must not be accepted.
        pane.apply_result(
            &Query::Fuzzy {
                term: "err".to_string(),
                until_seq: Some(100),
            },
            entries(&[1, 2, 3]),
            HashMap::new(),
            Some(vec![1, 2, 3]),
            None,
            true,
            (1, 200),
        );
        assert!(pane.view.entries().is_empty());
    }

    #[test]
    fn partial_freeze_emissions_held_complete_replaces_and_preserves_cursor() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let bounded = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: Some(9),
        };
        pane.active_query = Some(bounded.clone());
        // Seed the held view and enter the hold.
        pane.apply_result(
            &bounded,
            entries(&[3, 6]),
            HashMap::new(),
            Some(vec![3, 6]),
            None,
            true,
            (1, 9),
        );
        pane.cursor_seq = Some(6);
        pane.holding_for_complete = true;

        // A partial emission updates progress but must not replace entries.
        pane.apply_result(
            &bounded,
            entries(&[3]),
            HashMap::new(),
            Some(vec![3]),
            Some(SearchProgress {
                scanned: 1,
                total: 3,
            }),
            false,
            (1, 9),
        );
        assert!(pane.holding_for_complete);
        assert_eq!(
            pane.view
                .entries()
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            vec![3, 6],
            "held entries unchanged by a partial"
        );
        match &pane.view {
            View::Results { progress, .. } => assert_eq!(
                *progress,
                Some(SearchProgress {
                    scanned: 1,
                    total: 3
                })
            ),
            View::Stream { .. } => panic!("expected results view"),
        }

        // The complete emission replaces the view and preserves the cursor.
        pane.apply_result(
            &bounded,
            entries(&[3, 6, 9]),
            HashMap::new(),
            Some(vec![3, 6, 9]),
            None,
            true,
            (1, 9),
        );
        assert!(!pane.holding_for_complete);
        assert_eq!(
            pane.view
                .entries()
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            vec![3, 6, 9]
        );
        assert_eq!(pane.cursor_seq, Some(6), "selected seq preserved");
    }

    #[test]
    fn typing_during_handoff_clears_old_hold() {
        let srcs = [source("src-a", None)];
        let (ctx, _rx) = ctx_with(&srcs, (1, 9));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let bounded = Query::Fuzzy {
            term: "e".to_string(),
            until_seq: Some(9),
        };
        pane.active_query = Some(bounded.clone());
        pane.apply_result(
            &bounded,
            entries(&[3, 6]),
            HashMap::new(),
            Some(vec![3, 6]),
            None,
            true,
            (1, 9),
        );
        pane.holding_for_complete = true;

        // Typing dispatches a new bounded term; the old hold is cleared and a
        // new one begins for the new term.
        pane.update_search("er", &ctx);

        assert!(
            pane.holding_for_complete,
            "new bounded term re-arms the hold"
        );
        match &pane.active_query {
            Some(Query::Fuzzy { term, until_seq }) => {
                assert_eq!(term, "er");
                assert_eq!(*until_seq, Some(9));
            }
            other => panic!("expected bounded fuzzy, got {other:?}"),
        }
    }

    #[test]
    fn clone_of_frozen_search_carries_bound_and_resets_hold() {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        pane.holding_for_complete = true;
        pane.active_query = Some(Query::Fuzzy {
            term: "err".to_string(),
            until_seq: Some(100),
        });

        let clone = pane.clone_into(PaneId(2));

        assert_eq!(
            clone.active_query,
            Some(Query::Fuzzy {
                term: "err".to_string(),
                until_seq: Some(100)
            }),
            "the freeze bound rides into the split"
        );
        assert!(
            !clone.holding_for_complete,
            "a clone never receives the release emission, so it starts clear"
        );
    }

    #[test]
    fn enter_follow_on_frozen_results_goes_live_unbounded() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 20));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let frozen = Query::Fuzzy {
            term: "err".to_string(),
            until_seq: Some(9),
        };
        pane.active_query = Some(frozen.clone());
        pane.apply_result(
            &frozen,
            entries(&[3, 6, 9]),
            HashMap::new(),
            Some(vec![3, 6, 9]),
            None,
            true,
            (1, 9),
        );

        pane.enter_follow(&ctx);

        assert!(pane.follow);
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy { term, until_seq },
                ..
            } => {
                assert_eq!(term, "err");
                assert_eq!(until_seq, None, "going live is unbounded");
            }
            event => panic!("expected live fuzzy, got {event:?}"),
        }
    }

    #[test]
    fn refresh_resnapshots_frozen_search_at_fresh_bound_without_following() {
        let srcs = [source("src-a", None)];
        // The store high (20) has advanced past the original freeze bound (9).
        let (ctx, mut rx) = ctx_with(&srcs, (1, 20));
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        let frozen = Query::Fuzzy {
            term: "err".to_string(),
            until_seq: Some(9),
        };
        pane.active_query = Some(frozen.clone());
        pane.apply_result(
            &frozen,
            entries(&[3, 6, 9]),
            HashMap::new(),
            Some(vec![3, 6, 9]),
            None,
            true,
            (1, 9),
        );

        assert!(pane.refresh_search(&ctx));
        assert!(!pane.follow, "refresh stays frozen");
        assert!(pane.holding_for_complete);
        match rx.try_recv().expect("dispatch") {
            SearchEvent::Search {
                query: Query::Fuzzy { term, until_seq },
                ..
            } => {
                assert_eq!(term, "err");
                assert_eq!(until_seq, Some(20), "refresh captures a fresh high bound");
            }
            event => panic!("expected bounded fuzzy, got {event:?}"),
        }
    }

    #[test]
    fn refresh_is_noop_when_following_or_no_active_search() {
        let srcs = [source("src-a", None)];
        let (ctx, mut rx) = ctx_with(&srcs, (1, 20));
        let mut pane = Pane::new(PaneId(1));

        // Following pane: already live, nothing to refresh.
        assert!(pane.follow);
        assert!(!pane.refresh_search(&ctx));

        // Non-following plain stream: no fuzzy search to refresh.
        pane.follow = false;
        assert!(!pane.refresh_search(&ctx));
        assert!(
            rx.try_recv().is_err(),
            "no dispatch when nothing to refresh"
        );
    }

    #[test]
    fn charwise_selection_text_is_wrap_independent() {
        // Wrapping is render-only: a charwise selection spanning what would be
        // a wrap boundary still yanks the exact contiguous `row_text` slice.
        let mut pane = Pane::new(PaneId(1));
        pane.line_wrap = true;
        let e = Arc::new(LogEntry {
            seq: 1,
            msg: "abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "p".to_string(),
                id: "s".to_string(),
                display_name: "src".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        });
        pane.view = View::Stream {
            entries: vec![e.clone()],
        };
        pane.cursor_seq = Some(1);
        pane.cursor_col = 45;

        // Select cols 35..=45 — a span that straddles a 40-col wrap boundary.
        let got = pane.charwise_selection_text(1, 35);
        let expected: String = row_text(&e).chars().skip(35).take(45 - 35 + 1).collect();
        assert_eq!(got, expected);
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
            None,
            true,
            (1, 5),
        );
        pane.cursor_seq = Some(2);

        let selected: Vec<u64> = pane.selection(4).iter().map(|e| e.seq).collect();
        assert_eq!(selected, vec![2, 3, 4]);
    }

    #[test]
    fn follow_results_snap_col_when_wrapping() {
        // A following pane that advances to a fresh tall entry must park the
        // sticky column past the line end so wrapping shows the entry's last
        // display row, not row 0. Covers tail/startup result application.
        let mut pane = Pane::new(PaneId(1));
        pane.line_wrap = true;
        pane.cursor_col = 0;
        let query = Query::Tail;
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[1, 2, 3]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 3),
        );
        assert!(pane.follow);
        assert_eq!(pane.cursor_col, usize::MAX, "follow snaps to row end");

        // Wrap off leaves the column untouched (render path stays unchanged).
        let mut pane = Pane::new(PaneId(1));
        pane.cursor_col = 0;
        pane.active_query = Some(query.clone());
        pane.apply_result(
            &query,
            entries(&[1, 2, 3]),
            HashMap::new(),
            None,
            None,
            true,
            (1, 3),
        );
        assert_eq!(pane.cursor_col, 0, "wrap off is a no-op snap");
    }

    #[test]
    fn toggle_wrap_on_while_following_snaps_col() {
        let mut pane = Pane::new(PaneId(1));
        pane.cursor_col = 0;
        assert!(pane.follow && !pane.line_wrap);

        pane.toggle_line_wrap();
        assert_eq!(pane.cursor_col, usize::MAX, "wrap-on while following snaps");

        // Toggling back off should not re-snap, and a non-following pane keeps
        // its column when wrap is toggled.
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        pane.cursor_col = 7;
        pane.toggle_line_wrap();
        assert_eq!(pane.cursor_col, 7, "no snap when not following");
    }
}
