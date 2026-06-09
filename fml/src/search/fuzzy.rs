//! Fuzzy search worker.
//!
//! Runs a configured fuzzy match of the user's needle against every
//! retained [`LogEntry`]. For each entry the worker scores four field
//! classes independently — `msg`, the `level` display name (`"INFO"`,
//! `"WARN"`, …), the source display name, and each entry's `fields` values — and folds the per-class
//! scores into a single weighted aggregate. `msg` dominates ([`WEIGHT_MSG`]),
//! `level` is next ([`WEIGHT_LEVEL`]), and `fields` are lightest
//! ([`WEIGHT_FIELDS`]) so an entry that hits on multiple weak fields can
//! still be outranked by a single strong `msg` hit.
//!
//! Lifecycle: a `tokio::time::interval` ticks at `tick_rate`. The worker
//! holds an in-flight [`ScanState`] across ticks, racing chunk processing
//! against the ticker in a `tokio::select!`: between ticks it scores
//! `SCAN_CHUNK_SIZE` entries at a time, and when a tick fires it emits
//! whatever has been scored so far with `complete = false`. So `tick_rate`
//! doubles as both the emission cadence and the per-tick processing budget
//! — there is no separate scan-budget knob.
//!
//! Scanning is incremental across the query's lifetime: the first snapshot
//! covers the full retained window; once it completes, bounds drift only
//! triggers a scan of the newly ingested seqs (plus eviction of hits that
//! fell out of retention), so steady-state cost under live ingest is
//! O(new entries) per tick rather than a perpetual full rescan.
//! Cancellation of superseded queries is handled by the caller via
//! [`tokio::task::JoinHandle::abort`] — every loop iteration awaits at the
//! ticker or inside the emission helper, so abort is prompt.
//!
//! Each emission carries two things: up to `result_limit` ranked display
//! hits, and the full uncapped list of matching seq ids (for `n`/`N` hit
//! navigation). Scanning is scores-only; the expensive index traceback for
//! highlights runs at emission time against just the display hits, so its
//! cost tracks what is shown rather than what is retained. Partial results
//! are explicitly marked incomplete so UI code can distinguish "best so
//! far" from "full snapshot ranked".
//!
//! Per-hit highlight data is carried in [`Match::indices`] as ascending
//! character offsets into the matched field's value.

use std::{borrow::Cow, sync::Arc};

use frizbee::{Config as FrizbeeConfig, match_list_indices};
use nucleo_matcher::{
    Config as NucleoConfig, Matcher as NucleoEngine, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::{
    config::search::FuzzyMatcherKind,
    event::{Match, Query, SearchHit, SearchProgress},
    log::LogEntry,
    search::{EmitOutcome, SearchContext, emit_error, emit_hits},
};

const SCAN_CHUNK_SIZE: usize = 256;

/// Weight applied to a frizbee score on the `msg` field.
///
/// Intentionally large relative to [`WEIGHT_LEVEL`] and [`WEIGHT_FIELDS`]
/// so that a match on the message text dominates ranking — users searching
/// logs overwhelmingly care about the rendered line, not metadata.
const WEIGHT_MSG: f32 = 3.0;
/// Weight applied to a frizbee score on the `level` display name.
const WEIGHT_LEVEL: f32 = 0.5;
/// Weight applied to a frizbee score on the source display name.
const WEIGHT_SOURCE: f32 = 0.8;
/// Weight applied to a frizbee score on each `fields` value.
const WEIGHT_FIELDS: f32 = 0.3;

#[derive(Clone, Copy, Debug)]
pub struct FuzzySearchOptions {
    pub result_limit: usize,
    pub matcher_kind: FuzzyMatcherKind,
    pub max_typos: Option<u16>,
    /// Field leaf values longer than this many bytes are skipped during
    /// matching (`msg` is never capped). `0` disables the cap.
    pub max_field_bytes: usize,
}

/// Starts the background worker for a fuzzy text search.
///
/// The worker matches the needle (carried inside `ctx.query` as
/// `Query::Fuzzy(term)`) against each retained `LogEntry`'s `msg`, `level`
/// display name, source display name, and `fields` values using the
/// configured matcher, weights those matches (msg > level > fields), and
/// emits ranked hits at `ctx.tick_rate` cadence. A snapshot of the retained
/// `(low, high)` window and its [`ScanState`] are held across ticks: between
/// ticks the `tokio::select!` advances the scan one `SCAN_CHUNK_SIZE` batch
/// at a time, and when the ticker wins it emits the best `result_limit`
/// hits scored so far with `complete = false`. The final emission for a
/// snapshot carries `complete = true`. A snapshot is retired when the
/// scan completes or when bounds drift before the next scan begins.
/// Final ranking is by aggregate score, then `seq desc`. The returned
/// [`JoinHandle`] is used to cancel superseded work.
pub fn start_fuzzy_search(ctx: SearchContext, options: FuzzySearchOptions) -> JoinHandle<()> {
    let SearchContext {
        target,
        query,
        sources,
        request_id,
        tick_rate,
        store,
        tx,
    } = ctx;

    tokio::spawn(async move {
        let term = match &query {
            Query::Fuzzy(term) => term.clone(),
            other => panic!("start_fuzzy_search invoked with non-fuzzy query: {other:?}"),
        };

        debug!(
            "spawned fuzzy search - term: {}, sources: {:?}, result_limit: {}, tick_rate: {:?}",
            term, sources, options.result_limit, tick_rate
        );

        // An empty needle would match every entry with score 0 and produce
        // no useful ranking. Emit one empty result so the dispatcher sees
        // this request_id respond at least once, then exit — no point
        // holding a ticker open for something that can never produce hits.
        if term.is_empty() {
            let _ = emit_hits(
                target,
                query,
                Vec::new(),
                Some(Vec::new()),
                request_id,
                true,
                None,
                &tx,
            )
            .await;
            return;
        }

        let mut matcher = FuzzyMatcher::new(
            options.matcher_kind,
            &term,
            options.max_typos,
            options.max_field_bytes,
        );

        let mut ticker = tokio::time::interval(tick_rate);
        // The tick is an emission cadence, not a backlog of mandatory sends.
        // Skipping missed ticks lets scanning make progress after a slow chunk
        // instead of repeatedly servicing stale interval wakeups.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_complete_bounds: Option<(u64, u64)> = None;
        // Active snapshot kept alive across ticks so partial emissions
        // resume instead of throwing away work. (low, high, scan).
        let mut scan: Option<(u64, u64, ScanState)> = None;
        // Hits accumulated across snapshots. The scan is incremental: only
        // seqs above `scanned_high` are fetched per snapshot, while hits
        // below the retained low bound are evicted, so steady-state cost
        // under live ingest is O(new entries) instead of O(retained). The
        // set is score-capped at `result_limit`; once a hit is pruned by
        // the cap it is not rediscovered (capped-ranking-over-a-stream
        // semantics — a fresh query rescans everything anyway).
        let mut scored: Vec<ScoredHit> = Vec::new();
        // Every matching seq (uncapped, ascending) — hit navigation walks
        // this full set even though only `result_limit` hits are displayed.
        let mut hit_seqs: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        let mut scanned_high: u64 = 0;

        loop {
            // No live scan: park on the ticker until something might
            // have changed, then decide whether to start one.
            if scan.is_none() {
                ticker.tick().await;

                let (low, high) = store.bounds();

                if high == 0 {
                    if last_complete_bounds != Some((low, high)) {
                        scored.clear();
                        hit_seqs.clear();
                        match emit_hits(
                            target,
                            query.clone(),
                            Vec::new(),
                            Some(Vec::new()),
                            request_id,
                            true,
                            None,
                            &tx,
                        )
                        .await
                        {
                            EmitOutcome::Sent => {}
                            EmitOutcome::ReceiverGone => return,
                        }
                        last_complete_bounds = Some((low, high));
                    }
                    continue;
                }

                if last_complete_bounds == Some((low, high)) {
                    continue;
                }

                scored.retain(|hit| hit.entry.seq >= low);
                hit_seqs = hit_seqs.split_off(&low);
                let fetch_low = if scanned_high == 0 {
                    low
                } else {
                    (scanned_high + 1).max(low)
                };
                if fetch_low > high {
                    // Pure eviction — ranking is already correct.
                    let hits = build_hits(&scored, options.result_limit, &term, &mut matcher);
                    match emit_hits(
                        target,
                        query.clone(),
                        hits,
                        Some(hit_seqs.iter().copied().collect()),
                        request_id,
                        true,
                        None,
                        &tx,
                    )
                    .await
                    {
                        EmitOutcome::Sent => {}
                        EmitOutcome::ReceiverGone => return,
                    }
                    last_complete_bounds = Some((low, high));
                    continue;
                }

                let mut pool: Vec<Arc<LogEntry>> = Vec::new();
                if let Err(e) = store.fetch_range(fetch_low, high, &mut pool) {
                    let _ = emit_error(e.to_string(), &tx).await;
                    return;
                }
                let mut entries: Vec<Arc<LogEntry>> = pool
                    .into_iter()
                    .filter(|entry| sources.is_empty() || sources.contains(&entry.source.id))
                    .collect();
                entries.reverse();
                scan = Some((low, high, ScanState::new(entries)));
            }

            let (snap_low, snap_high, state) = scan.as_mut().expect("scan just ensured");

            tokio::select! {
                _ = ticker.tick() => {
                    let hits = build_hits(&scored, options.result_limit, &term, &mut matcher);
                    match emit_hits(
                        target,
                        query.clone(),
                        hits,
                        Some(hit_seqs.iter().copied().collect()),
                        request_id,
                        false,
                        Some(state.progress()),
                        &tx,
                    ).await {
                        EmitOutcome::Sent => {}
                        EmitOutcome::ReceiverGone => return,
                    }
                }
                _ = tokio::task::yield_now(), if !state.is_complete() => {
                    state.process_next_chunk(
                        &term,
                        &mut matcher,
                        &mut scored,
                        &mut hit_seqs,
                        options.result_limit,
                    );
                    if state.is_complete() {
                        let hits = build_hits(&scored, options.result_limit, &term, &mut matcher);
                        match emit_hits(
                            target,
                            query.clone(),
                            hits,
                            Some(hit_seqs.iter().copied().collect()),
                            request_id,
                            true,
                            Some(state.progress()),
                            &tx,
                        )
                        .await
                        {
                            EmitOutcome::Sent => {}
                            EmitOutcome::ReceiverGone => return,
                        }
                        scanned_high = *snap_high;
                        last_complete_bounds = Some((*snap_low, *snap_high));
                        scan = None;
                    }
                }
            }
        }
    })
}

struct ScanState {
    entries: Vec<Arc<LogEntry>>,
    next_index: usize,
}

impl ScanState {
    fn new(entries: Vec<Arc<LogEntry>>) -> Self {
        Self {
            entries,
            next_index: 0,
        }
    }

    fn is_complete(&self) -> bool {
        self.next_index >= self.entries.len()
    }

    /// Scores the next chunk into the shared accumulator, pruning to the
    /// score cap once the accumulator doubles it (amortizes the sort).
    /// Every matching seq is recorded in `hit_seqs` before pruning, so the
    /// navigation set stays uncapped.
    fn process_next_chunk(
        &mut self,
        needle: &str,
        matcher: &mut FuzzyMatcher,
        scored: &mut Vec<ScoredHit>,
        hit_seqs: &mut std::collections::BTreeSet<u64>,
        result_limit: usize,
    ) {
        if self.is_complete() {
            return;
        }

        let end = (self.next_index + SCAN_CHUNK_SIZE).min(self.entries.len());
        let batch = matcher.score_batch(needle, &self.entries[self.next_index..end]);
        hit_seqs.extend(batch.iter().map(|hit| hit.entry.seq));
        scored.extend(batch);
        self.next_index = end;

        if scored.len() > result_limit.saturating_mul(2).max(SCAN_CHUNK_SIZE) {
            rank_hits(scored);
            scored.truncate(result_limit);
        }
    }

    fn progress(&self) -> SearchProgress {
        SearchProgress {
            scanned: self.next_index,
            total: self.entries.len(),
        }
    }
}

/// Sorts hits by aggregate score (desc), then recency (`seq` desc).
fn rank_hits(scored: &mut [ScoredHit]) {
    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.entry.seq.cmp(&a.entry.seq))
    });
}

/// One ranked entry in the worker's internal cache.
///
/// `score` is the weighted aggregate across all matched field classes
/// (`i64` to absorb summed u16 scores under weights without overflow).
/// Highlight indices are intentionally absent: the scan is scores-only,
/// and [`build_hits`] re-runs the index traceback for just the hits that
/// are actually emitted.
struct ScoredHit {
    entry: Arc<LogEntry>,
    score: i64,
}

/// Ranks the accumulator and produces display hits for the top
/// `result_limit`, computing highlight indices only for those — traceback
/// cost is proportional to what is shown, not what is retained.
fn build_hits(
    scored: &[ScoredHit],
    result_limit: usize,
    needle: &str,
    matcher: &mut FuzzyMatcher,
) -> Vec<SearchHit> {
    let mut ranked: Vec<&ScoredHit> = scored.iter().collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.entry.seq.cmp(&a.entry.seq))
    });
    if ranked.len() > result_limit {
        ranked.truncate(result_limit);
    }

    ranked
        .into_iter()
        .map(|hit| SearchHit {
            seq_id: hit.entry.seq,
            matches: matcher.trace_entry(needle, &hit.entry),
        })
        .collect()
}

enum FuzzyMatcher {
    Frizbee(FrizbeeMatcher),
    Nucleo(NucleoMatcher),
}

impl FuzzyMatcher {
    fn new(
        kind: FuzzyMatcherKind,
        needle: &str,
        max_typos: Option<u16>,
        max_field_bytes: usize,
    ) -> Self {
        match kind {
            FuzzyMatcherKind::Frizbee => {
                Self::Frizbee(FrizbeeMatcher::new(max_typos, max_field_bytes))
            }
            FuzzyMatcherKind::Nucleo => Self::Nucleo(NucleoMatcher::new(needle, max_field_bytes)),
        }
    }

    fn score_batch(&mut self, needle: &str, entries: &[Arc<LogEntry>]) -> Vec<ScoredHit> {
        match self {
            Self::Frizbee(matcher) => matcher.score_batch(needle, entries),
            Self::Nucleo(matcher) => matcher.score_batch(entries),
        }
    }

    /// Computes highlight indices for one already-matched entry. Only
    /// called for hits being emitted, so the per-field index traceback
    /// runs on the displayed few instead of every retained entry.
    fn trace_entry(&mut self, needle: &str, entry: &Arc<LogEntry>) -> Vec<Match> {
        match self {
            Self::Frizbee(matcher) => matcher.trace_entry(needle, entry),
            Self::Nucleo(matcher) => matcher.trace_entry(entry),
        }
    }
}

struct FrizbeeMatcher {
    config: FrizbeeConfig,
    max_field_bytes: usize,
}

impl FrizbeeMatcher {
    fn new(max_typos: Option<u16>, max_field_bytes: usize) -> Self {
        // `sort: false` — we combine scores across several frizbee calls
        // into a single aggregate and need to sort on that combined number.
        Self {
            config: FrizbeeConfig {
                max_typos,
                sort: false,
                scoring: frizbee::Scoring::default(),
            },
            max_field_bytes,
        }
    }

    /// Scores one batch with batched frizbee calls — scores only, no index
    /// traceback. The weighted aggregate and match gating reproduce the
    /// previous indices-based implementation's ranking exactly.
    fn score_batch(&self, needle: &str, entries: &[Arc<LogEntry>]) -> Vec<ScoredHit> {
        if entries.is_empty() {
            return Vec::new();
        }

        let msg_haystack: Vec<&str> = entries.iter().map(|e| e.msg.as_str()).collect();
        let level_haystack: Vec<String> = entries
            .iter()
            .map(|e| e.level.map(|l| l.to_string()).unwrap_or_default())
            .collect();
        let source_haystack: Vec<&str> = entries
            .iter()
            .map(|e| e.source.display_name.as_str())
            .collect();

        let mut field_haystack: Vec<Cow<'_, str>> = Vec::new();
        let mut field_owners: Vec<usize> = Vec::new();
        for (idx, entry) in entries.iter().enumerate() {
            for (key, value) in &entry.fields {
                for_each_leaf(key, value, self.max_field_bytes, &mut |_, text| {
                    field_haystack.push(text);
                    field_owners.push(idx);
                });
            }
        }

        let msg_hits = frizbee::match_list(needle, &msg_haystack, &self.config);
        let level_hits = if level_haystack.iter().any(|s| !s.is_empty()) {
            frizbee::match_list(needle, &level_haystack, &self.config)
        } else {
            Vec::new()
        };
        let source_hits = frizbee::match_list(needle, &source_haystack, &self.config);
        let field_hits = if !field_haystack.is_empty() {
            frizbee::match_list(needle, &field_haystack, &self.config)
        } else {
            Vec::new()
        };

        // (aggregate score, matched-field count) per entry in the batch.
        let mut scores: Vec<(i64, usize)> = vec![(0, 0); entries.len()];
        let mut bump = |entry_idx: usize, raw: u16, weight: f32| {
            if let Some(slot) = scores.get_mut(entry_idx) {
                slot.0 = slot.0.saturating_add((raw as f32 * weight).round() as i64);
                slot.1 += 1;
            }
        };
        for m in msg_hits {
            bump(m.index as usize, m.score, WEIGHT_MSG);
        }
        for m in level_hits {
            bump(m.index as usize, m.score, WEIGHT_LEVEL);
        }
        for m in source_hits {
            bump(m.index as usize, m.score, WEIGHT_SOURCE);
        }
        for m in field_hits {
            if let Some(&entry_idx) = field_owners.get(m.index as usize) {
                bump(entry_idx, m.score, WEIGHT_FIELDS);
            }
        }

        entries
            .iter()
            .zip(scores)
            .filter(|(_, (score, matched))| *score > 0 && *matched > 0)
            .map(|(entry, (score, _))| ScoredHit {
                entry: entry.clone(),
                score,
            })
            .collect()
    }

    /// Index traceback for a single entry's fields (emission-time only).
    fn trace_entry(&self, needle: &str, entry: &Arc<LogEntry>) -> Vec<Match> {
        let fields = searchable_fields(entry, self.max_field_bytes);
        let haystacks: Vec<&str> = fields.iter().map(|field| field.value.as_ref()).collect();
        match_list_indices(needle, &haystacks, &self.config)
            .into_iter()
            .filter_map(|m| {
                let field = fields.get(m.index as usize)?;
                Some(Match {
                    key: field.key.clone(),
                    indices: reverse_to_u32(m.indices),
                })
            })
            .collect()
    }
}

struct NucleoMatcher {
    pattern: Pattern,
    engine: NucleoEngine,
    has_positive_atoms: bool,
    max_field_bytes: usize,
}

impl NucleoMatcher {
    fn new(needle: &str, max_field_bytes: usize) -> Self {
        let pattern = Pattern::parse(needle, CaseMatching::Ignore, Normalization::Smart);
        let has_positive_atoms = pattern.atoms.iter().any(|atom| !atom.negative);
        Self {
            pattern,
            engine: NucleoEngine::new(NucleoConfig::DEFAULT),
            has_positive_atoms,
            max_field_bytes,
        }
    }

    fn score_batch(&mut self, entries: &[Arc<LogEntry>]) -> Vec<ScoredHit> {
        entries
            .iter()
            .filter_map(|entry| self.score_entry(entry))
            .collect()
    }

    fn score_entry(&mut self, entry: &Arc<LogEntry>) -> Option<ScoredHit> {
        let fields = searchable_fields(entry, self.max_field_bytes);
        if self.entry_matches_negative_atom(&fields) {
            return None;
        }

        if !self.has_positive_atoms {
            return Some(ScoredHit {
                entry: entry.clone(),
                score: 0,
            });
        }

        let mut score: i64 = 0;
        let mut matched = 0usize;
        for field in &fields {
            let mut buf = Vec::new();
            if let Some(field_score) = self
                .pattern
                .score(Utf32Str::new(&field.value, &mut buf), &mut self.engine)
            {
                score = score.saturating_add((field_score as f32 * field.weight).round() as i64);
                matched += 1;
            }
        }

        (score > 0 && matched > 0).then_some(ScoredHit {
            entry: entry.clone(),
            score,
        })
    }

    /// Index traceback for a single entry's fields (emission-time only).
    fn trace_entry(&mut self, entry: &Arc<LogEntry>) -> Vec<Match> {
        if !self.has_positive_atoms {
            return Vec::new();
        }
        searchable_fields(entry, self.max_field_bytes)
            .iter()
            .filter_map(|field| {
                let (_, indices) = self.match_positive_field(&field.value)?;
                Some(Match {
                    key: field.key.clone(),
                    indices,
                })
            })
            .collect()
    }

    fn entry_matches_negative_atom(&mut self, fields: &[SearchableField]) -> bool {
        self.pattern
            .atoms
            .iter()
            .filter(|atom| atom.negative)
            .any(|atom| {
                fields.iter().any(|field| {
                    let mut buf = Vec::new();
                    atom.score(Utf32Str::new(&field.value, &mut buf), &mut self.engine)
                        .is_none()
                })
            })
    }

    fn match_positive_field(&mut self, value: &str) -> Option<(u32, Vec<u32>)> {
        let mut indices = Vec::new();
        let mut buf = Vec::new();
        let score = self.pattern.indices(
            Utf32Str::new(value, &mut buf),
            &mut self.engine,
            &mut indices,
        )?;
        indices.sort_unstable();
        indices.dedup();
        (!indices.is_empty()).then_some((score, indices))
    }
}

struct SearchableField<'a> {
    key: String,
    value: Cow<'a, str>,
    weight: f32,
}

fn searchable_fields(entry: &LogEntry, max_field_bytes: usize) -> Vec<SearchableField<'_>> {
    let mut fields = Vec::with_capacity(3 + entry.fields.len());
    fields.push(SearchableField {
        key: "msg".to_string(),
        value: Cow::Borrowed(entry.msg.as_str()),
        weight: WEIGHT_MSG,
    });
    fields.push(SearchableField {
        key: "level".to_string(),
        value: Cow::Owned(entry.level.map(|l| l.to_string()).unwrap_or_default()),
        weight: WEIGHT_LEVEL,
    });
    fields.push(SearchableField {
        key: "source".to_string(),
        value: Cow::Borrowed(entry.source.display_name.as_str()),
        weight: WEIGHT_SOURCE,
    });
    for (key, value) in &entry.fields {
        for_each_leaf(key, value, max_field_bytes, &mut |path, text| {
            fields.push(SearchableField {
                key: path.to_string(),
                value: text,
                weight: WEIGHT_FIELDS,
            });
        });
    }
    fields
}

/// Converts frizbee's reverse-order `Vec<usize>` indices into the
/// ascending `Vec<u32>` our [`Match::indices`] consumer expects.
///
/// Frizbee's `match_list_indices` returns matched character positions
/// in reverse order as a side-effect of its Smith-Waterman traceback
/// (cheaper than an extra reversal inside the hot loop). The UI renders
/// highlights left-to-right, so we reverse here at the module boundary
/// and narrow to `u32` to match the event type.
fn reverse_to_u32(mut indices: Vec<usize>) -> Vec<u32> {
    indices.reverse();
    indices.into_iter().map(|i| i as u32).collect()
}

/// Walks a `serde_json::Value` and yields each scalar leaf as a haystack,
/// with object members reported under dotted paths (`http.method`).
///
/// String leaves are borrowed straight out of the entry — the scan happens
/// every tick over every retained entry, so cloning multi-KB payload
/// strings per pass dominated scan cost. Numbers/bools are tiny owned
/// formats. Composite values are never serialized: the needle matches
/// values, not JSON syntax or key names.
/// String leaves longer than `max_bytes` are skipped entirely (`0` = no
/// cap): giant payload blobs are rarely search targets but dominate match
/// cost. The cap never applies to `msg` — only callers walking `fields`
/// come through here.
fn for_each_leaf<'a>(
    path: &str,
    value: &'a serde_json::Value,
    max_bytes: usize,
    f: &mut dyn FnMut(&str, Cow<'a, str>),
) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::String(s) => {
            if max_bytes == 0 || s.len() <= max_bytes {
                f(path, Cow::Borrowed(s.as_str()))
            }
        }
        serde_json::Value::Bool(b) => f(path, Cow::Owned(b.to_string())),
        serde_json::Value::Number(n) => f(path, Cow::Owned(n.to_string())),
        serde_json::Value::Array(items) => {
            for item in items {
                for_each_leaf(path, item, max_bytes, f);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                for_each_leaf(&format!("{path}.{key}"), value, max_bytes, f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        event::SearchEvent,
        log::{LogLevel, NewLogEntry, Source, SourceId},
        store::{LogStore, RingBufferStore},
    };

    fn store_config(capacity: usize) -> StoreConfig {
        StoreConfig { capacity }
    }

    fn make_entry(msg: &str, source_id: &str, level: Option<LogLevel>) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level,
            source: Source {
                producer: "fake".to_string(),
                id: source_id.to_string(),
                display_name: source_id.to_string(),
                group: None,
            },
            fields: HashMap::new(),
        }
    }

    fn make_entry_with_fields(
        msg: &str,
        source_id: &str,
        level: Option<LogLevel>,
        fields: HashMap<String, serde_json::Value>,
    ) -> NewLogEntry {
        NewLogEntry {
            msg: msg.to_string(),
            ts: Utc::now(),
            level,
            source: Source {
                producer: "fake".to_string(),
                id: source_id.to_string(),
                display_name: source_id.to_string(),
                group: None,
            },
            fields,
        }
    }

    async fn recv_result(rx: &mut mpsc::Receiver<SearchEvent>) -> (Vec<SearchHit>, u64, bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out awaiting SearchEvent")
            .expect("channel closed before delivering SearchEvent");
        match evt {
            SearchEvent::Result {
                target: _,
                query: _,
                results,
                hit_seqs: _,
                request_id,
                complete,
                progress: _,
            } => (results, request_id, complete),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
            SearchEvent::Cancel { .. } => panic!("unexpected SearchEvent::Cancel"),
        }
    }

    async fn recv_result_with_progress(
        rx: &mut mpsc::Receiver<SearchEvent>,
    ) -> (Vec<SearchHit>, u64, bool, Option<SearchProgress>) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out awaiting SearchEvent")
            .expect("channel closed before delivering SearchEvent");
        match evt {
            SearchEvent::Result {
                target: _,
                query: _,
                results,
                hit_seqs: _,
                request_id,
                complete,
                progress,
            } => (results, request_id, complete, progress),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
            SearchEvent::Cancel { .. } => panic!("unexpected SearchEvent::Cancel"),
        }
    }

    fn fuzzy_options(
        result_limit: usize,
        matcher_kind: FuzzyMatcherKind,
        max_typos: Option<u16>,
    ) -> FuzzySearchOptions {
        FuzzySearchOptions {
            result_limit,
            matcher_kind,
            max_typos,
            // Tests opt out of the leaf cap unless they exercise it.
            max_field_bytes: 0,
        }
    }

    fn start_test_fuzzy_search(
        term: String,
        sources: Vec<SourceId>,
        tick_rate: Duration,
        options: FuzzySearchOptions,
        store: Arc<dyn LogStore>,
        request_id: u64,
        tx: mpsc::Sender<SearchEvent>,
    ) -> JoinHandle<()> {
        start_fuzzy_search(
            SearchContext {
                target: crate::event::PaneId(1),
                query: Query::Fuzzy(term),
                sources,
                request_id,
                tick_rate,
                store,
                tx,
            },
            options,
        )
    }

    #[tokio::test]
    async fn empty_term_emits_empty_result() {
        let store = RingBufferStore::new(store_config(64));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            String::new(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            99,
            tx,
        );

        let (results, rid, complete, progress) = recv_result_with_progress(&mut rx).await;
        assert_eq!(rid, 99);
        assert!(complete);
        assert_eq!(progress, None);
        assert!(results.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn msg_match_ranks_above_field_match() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("unrelated line", "s1", Some(LogLevel::Info)));
        let mut fields = HashMap::new();
        fields.insert(
            "code".to_string(),
            serde_json::Value::String("error".to_string()),
        );
        store.insert(make_entry_with_fields(
            "boring",
            "s1",
            Some(LogLevel::Info),
            fields,
        ));
        store.insert(make_entry(
            "server error occurred",
            "s1",
            Some(LogLevel::Error),
        ));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            1,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 1);
        assert!(complete);
        assert!(!results.is_empty(), "expected matches");
        assert_eq!(
            results[0].seq_id, 3,
            "entry with msg match should rank first"
        );
        assert!(
            results[0].matches.iter().any(|m| m.key == "msg"),
            "top hit should include a msg Match"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn filters_by_source_id() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("server error", "s1", Some(LogLevel::Error)));
        store.insert(make_entry("server error", "s2", Some(LogLevel::Error)));
        store.insert(make_entry("server error", "s1", Some(LogLevel::Error)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec!["s1".to_string()],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            7,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = results.iter().map(|r| r.seq_id).collect();
        seqs.iter().for_each(|s| assert!(*s == 1 || *s == 3));
        assert_eq!(seqs.len(), 2);

        handle.abort();
    }

    #[tokio::test]
    async fn new_entries_appear_on_subsequent_emit() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("alpha error", "s1", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            3,
            tx,
        );

        let (seed, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 3);
        assert!(complete);
        assert_eq!(seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(), vec![1]);

        store.insert(make_entry("beta error", "s1", Some(LogLevel::Info)));

        let (update, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let seqs: Vec<u64> = update.iter().map(|r| r.seq_id).collect();
        assert!(seqs.contains(&1) && seqs.contains(&2), "seqs={:?}", seqs);

        handle.abort();
    }

    #[tokio::test]
    async fn msg_match_indices_are_ascending() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("server", "s1", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "er".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            1,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        let msg_match = results
            .iter()
            .flat_map(|hit| hit.matches.iter())
            .find(|m| m.key == "msg")
            .expect("expected a msg Match");
        assert!(!msg_match.indices.is_empty(), "indices should be populated");
        assert!(
            msg_match.indices.windows(2).all(|w| w[0] < w[1]),
            "indices should be strictly ascending, got {:?}",
            msg_match.indices
        );

        handle.abort();
    }

    #[tokio::test]
    async fn level_match_keyed_as_level() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("nothing", "s1", Some(LogLevel::Warn)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "WARN".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            1,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert!(!results.is_empty(), "expected a level match");
        assert!(
            results[0].matches.iter().any(|m| m.key == "level"),
            "expected a Match with key=level, got {:?}",
            results[0].matches
        );

        handle.abort();
    }

    #[tokio::test]
    async fn source_display_name_match_keyed_as_source() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(NewLogEntry {
            msg: "nothing".to_string(),
            ts: Utc::now(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: "s1".to_string(),
                display_name: "payments-api".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        });

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "payments".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, Some(0)),
            store.clone(),
            1,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert!(!results.is_empty(), "expected a source display name match");
        assert!(
            results[0].matches.iter().any(|m| m.key == "source"),
            "expected a Match with key=source, got {:?}",
            results[0].matches
        );

        handle.abort();
    }

    #[tokio::test]
    async fn result_limit_truncates() {
        let store = RingBufferStore::new(store_config(128));
        for i in 1..=30 {
            store.insert(make_entry(
                &format!("server error {i}"),
                "s1",
                Some(LogLevel::Info),
            ));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(5, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            1,
            tx,
        );

        let (results, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert_eq!(results.len(), 5);

        handle.abort();
    }

    #[tokio::test]
    async fn equal_scores_prefer_newer_seq() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("error", "s1", Some(LogLevel::Info)));
        store.insert(make_entry("error", "s1", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(10, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            11,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 11);
        assert!(complete);
        assert_eq!(
            results.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![2, 1]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn full_rescan_recovers_truncated_retained_hit_after_eviction() {
        let store = RingBufferStore::new(store_config(3));
        store.insert(make_entry("error", "s1", Some(LogLevel::Info)));
        store.insert(make_entry(
            "server error occurred",
            "s1",
            Some(LogLevel::Error),
        ));
        let mut fields = HashMap::new();
        fields.insert(
            "code".to_string(),
            serde_json::Value::String("error".to_string()),
        );
        store.insert(make_entry_with_fields(
            "boring",
            "s1",
            Some(LogLevel::Info),
            fields,
        ));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(2, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            12,
            tx,
        );

        let (seed, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 12);
        assert!(complete);
        assert_eq!(
            seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1, 2]
        );

        store.insert(make_entry("noise", "s1", Some(LogLevel::Info)));

        let (update, _, complete) = recv_result(&mut rx).await;
        assert!(complete);
        assert_eq!(
            update.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![2, 3]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn partial_result_is_marked_incomplete_when_tick_wins() {
        let store = RingBufferStore::new(store_config(20_000));
        for i in 1..=10_000 {
            store.insert(make_entry(
                &format!("server error {i}"),
                "s1",
                Some(LogLevel::Info),
            ));
        }

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(1),
            fuzzy_options(20, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            13,
            tx,
        );

        let (_, rid, complete, progress) = recv_result_with_progress(&mut rx).await;
        assert_eq!(rid, 13);
        assert!(!complete, "expected an incomplete partial result");
        let progress = progress.expect("partial fuzzy result should include scan progress");
        assert!(progress.scanned < progress.total);

        handle.abort();
    }

    #[tokio::test]
    async fn partial_emits_progress_across_ticks() {
        let store = RingBufferStore::new(store_config(20_000));
        for i in 1..=5_000 {
            store.insert(make_entry(
                &format!("server error {i}"),
                "s1",
                Some(LogLevel::Info),
            ));
        }

        let (tx, mut rx) = mpsc::channel(64);
        // 1ms tick is short enough that several emissions race the scan
        // before it finishes, but long enough that a few chunks score
        // between ticks — so partials grow rather than always being empty.
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec![],
            Duration::from_millis(1),
            fuzzy_options(20_000, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            21,
            tx,
        );

        // Walk emissions, asserting hit count is non-decreasing (proves
        // work is resumed across ticks, not restarted), until we see a
        // final complete=true.
        let mut saw_partial = false;
        let mut prev_len = 0usize;
        let mut saw_complete = false;
        for _ in 0..2_000 {
            let (next, rid, complete) = recv_result(&mut rx).await;
            assert_eq!(rid, 21);
            assert!(
                next.len() >= prev_len,
                "scan progress regressed: {} -> {}",
                prev_len,
                next.len()
            );
            prev_len = next.len();
            if !complete {
                saw_partial = true;
            } else {
                saw_complete = true;
                break;
            }
        }
        assert!(saw_partial, "expected at least one partial emission");
        assert!(saw_complete, "expected a final complete=true emission");

        handle.abort();
    }

    #[tokio::test]
    async fn live_arrivals_wait_for_snapshot_completion_then_rescan_with_source_filter() {
        let store = RingBufferStore::new(store_config(20_000));
        for i in 1..=5_000 {
            store.insert(make_entry(
                &format!("server error {i}"),
                "s1",
                Some(LogLevel::Info),
            ));
            store.insert(make_entry(
                &format!("server error {i}"),
                "s2",
                Some(LogLevel::Info),
            ));
        }

        let (tx, mut rx) = mpsc::channel(128);
        let handle = start_test_fuzzy_search(
            "error".to_string(),
            vec!["s1".to_string()],
            Duration::from_millis(1),
            fuzzy_options(20_000, FuzzyMatcherKind::Frizbee, None),
            store.clone(),
            22,
            tx,
        );

        let (_, rid, complete, progress) = recv_result_with_progress(&mut rx).await;
        assert_eq!(rid, 22);
        assert!(!complete, "expected the first emission to be partial");
        assert_eq!(
            progress.expect("partial should include progress").total,
            5_000,
            "progress total should count only the filtered snapshot"
        );

        store.insert(make_entry("late allowed error", "s1", Some(LogLevel::Info)));
        let allowed_late_seq = store.bounds().1;
        store.insert(make_entry("late blocked error", "s2", Some(LogLevel::Info)));
        let blocked_late_seq = store.bounds().1;

        let mut snapshot_complete = None;
        for _ in 0..2_000 {
            let (results, event_rid, event_complete) = recv_result(&mut rx).await;
            assert_eq!(event_rid, 22);
            if event_complete {
                snapshot_complete = Some(results);
                break;
            }
        }
        let snapshot_complete = snapshot_complete.expect("expected old snapshot to complete");
        assert!(
            snapshot_complete
                .iter()
                .all(|hit| hit.seq_id != allowed_late_seq && hit.seq_id != blocked_late_seq),
            "in-flight snapshot should not include logs appended after it started"
        );

        let mut rescan_complete = None;
        for _ in 0..2_000 {
            let (results, event_rid, event_complete) = recv_result(&mut rx).await;
            assert_eq!(event_rid, 22);
            if event_complete {
                rescan_complete = Some(results);
                break;
            }
        }
        let rescan_complete = rescan_complete.expect("expected changed bounds to trigger rescan");
        assert!(
            rescan_complete
                .iter()
                .any(|hit| hit.seq_id == allowed_late_seq),
            "rescan should include the new log from an enabled source"
        );
        assert!(
            rescan_complete
                .iter()
                .all(|hit| hit.seq_id != blocked_late_seq),
            "rescan should still exclude disabled sources"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn nucleo_parsed_apostrophe_query_requires_contiguous_substring() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("n e e d l e", "s1", Some(LogLevel::Info)));
        store.insert(make_entry(
            "contains needle here",
            "s1",
            Some(LogLevel::Info),
        ));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "'needle".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Nucleo, None),
            store.clone(),
            31,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 31);
        assert!(complete);
        assert_eq!(
            results.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![2]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn nucleo_negative_terms_exclude_matching_entries() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("alpha error", "s1", Some(LogLevel::Info)));
        store.insert(make_entry("beta error", "s1", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "error !beta".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Nucleo, None),
            store.clone(),
            32,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 32);
        assert!(complete);
        assert_eq!(
            results.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1]
        );

        handle.abort();
    }

    #[tokio::test]
    async fn nucleo_negative_only_query_returns_non_excluded_entries_newest_first() {
        let store = RingBufferStore::new(store_config(64));
        store.insert(make_entry("alpha", "s1", Some(LogLevel::Info)));
        store.insert(make_entry("beta", "s1", Some(LogLevel::Info)));
        store.insert(make_entry("gamma", "s1", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_test_fuzzy_search(
            "!beta".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(100, FuzzyMatcherKind::Nucleo, None),
            store.clone(),
            33,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 33);
        assert!(complete);
        assert_eq!(
            results.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![3, 1]
        );
        assert!(results.iter().all(|hit| hit.matches.is_empty()));

        handle.abort();
    }

    /// The leaf-size cap skips long field values but never the message.
    #[tokio::test]
    async fn leaf_cap_skips_long_field_values_but_not_msg() {
        let store = RingBufferStore::new(store_config(8));
        let long_blob = format!("{}alpha{}", "x".repeat(300), "x".repeat(300));
        // seq 1: needle only inside an over-cap field value.
        store.insert(make_entry_with_fields(
            "quiet message",
            "src-a",
            Some(LogLevel::Info),
            HashMap::from([("payload".to_string(), serde_json::json!(long_blob))]),
        ));
        // seq 2: needle in a short field value — must still match.
        store.insert(make_entry_with_fields(
            "quiet message",
            "src-a",
            Some(LogLevel::Info),
            HashMap::from([("tag".to_string(), serde_json::json!("alpha"))]),
        ));
        // seq 3: needle in a msg longer than the cap — msg is never capped.
        store.insert(make_entry(
            &format!("alpha {}", "m".repeat(600)),
            "src-a",
            Some(LogLevel::Info),
        ));

        let (tx, mut rx) = mpsc::channel(16);
        let mut options = fuzzy_options(100, FuzzyMatcherKind::Frizbee, Some(0));
        options.max_field_bytes = 512;
        let handle = start_test_fuzzy_search(
            "alpha".to_string(),
            vec![],
            Duration::from_millis(10),
            options,
            store.clone(),
            11,
            tx,
        );

        let hits = loop {
            let (results, rid, complete) = recv_result(&mut rx).await;
            assert_eq!(rid, 11);
            if complete {
                break results;
            }
        };
        let mut seqs: Vec<u64> = hits.iter().map(|h| h.seq_id).collect();
        seqs.sort_unstable();
        assert_eq!(
            seqs,
            vec![2, 3],
            "capped blob skipped; short field and long msg match"
        );

        handle.abort();
    }

    /// With a display cap of 1, the emitted results are capped but the
    /// hit-seq list still names every match, and the displayed hit carries
    /// highlight indices from the emission-time traceback.
    #[tokio::test]
    async fn display_cap_keeps_full_hit_seqs_for_navigation() {
        let store = RingBufferStore::new(store_config(8));
        store.insert(make_entry("alpha one", "src-a", Some(LogLevel::Info)));
        store.insert(make_entry("noise", "src-a", Some(LogLevel::Info)));
        store.insert(make_entry("alpha two", "src-a", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(16);
        let handle = start_test_fuzzy_search(
            "alpha".to_string(),
            vec![],
            Duration::from_millis(10),
            fuzzy_options(1, FuzzyMatcherKind::Frizbee, Some(0)),
            store.clone(),
            5,
            tx,
        );

        let (results, hit_seqs) = loop {
            let evt = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out awaiting result")
                .expect("channel closed");
            if let SearchEvent::Result {
                results,
                hit_seqs,
                complete: true,
                ..
            } = evt
            {
                break (results, hit_seqs);
            }
        };

        assert_eq!(results.len(), 1, "display hits are capped");
        assert!(
            !results[0].matches.is_empty(),
            "displayed hit carries traceback indices"
        );
        assert_eq!(hit_seqs, Some(vec![1, 3]), "navigation set is uncapped");

        handle.abort();
    }

    /// New entries are picked up by an incremental scan and ring-evicted
    /// hits are dropped, without rescanning the already-scored window.
    #[tokio::test]
    async fn incremental_rescan_adds_new_hits_and_drops_evicted() {
        let store = RingBufferStore::new(store_config(4));
        store.insert(make_entry("alpha match one", "src-a", Some(LogLevel::Info)));
        store.insert(make_entry("noise", "src-a", Some(LogLevel::Info)));
        store.insert(make_entry("alpha match two", "src-a", Some(LogLevel::Info)));

        let (tx, mut rx) = mpsc::channel(16);
        let handle = start_test_fuzzy_search(
            "alpha".to_string(),
            vec![],
            Duration::from_millis(10),
            // Zero typos: only true subsequence matches count, so noise
            // entries can't sneak in via weak source/level matches.
            fuzzy_options(100, FuzzyMatcherKind::Frizbee, Some(0)),
            store.clone(),
            7,
            tx,
        );

        // First full scan completes with both alpha entries.
        let hits = loop {
            let (results, rid, complete) = recv_result(&mut rx).await;
            assert_eq!(rid, 7);
            if complete {
                break results;
            }
        };
        let mut seqs: Vec<u64> = hits.iter().map(|h| h.seq_id).collect();
        seqs.sort_unstable();
        assert_eq!(seqs, vec![1, 3]);

        // Capacity 4: these inserts evict seqs 1-2, retaining 3..=6.
        store.insert(make_entry("noise two", "src-a", Some(LogLevel::Info)));
        store.insert(make_entry(
            "alpha match three",
            "src-a",
            Some(LogLevel::Info),
        ));
        store.insert(make_entry("noise three", "src-a", Some(LogLevel::Info)));

        // Next complete emission: evicted hit gone, new hit found.
        let hits = loop {
            let (results, _, complete) = recv_result(&mut rx).await;
            let mut seqs: Vec<u64> = results.iter().map(|h| h.seq_id).collect();
            seqs.sort_unstable();
            if complete && seqs != vec![1, 3] {
                break results;
            }
        };
        let mut seqs: Vec<u64> = hits.iter().map(|h| h.seq_id).collect();
        seqs.sort_unstable();
        assert_eq!(seqs, vec![3, 5]);

        handle.abort();
    }

    /// Release-mode throughput probe, not a regression test. Run with:
    /// `cargo test --release -p fml bench_fuzzy_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_fuzzy_scan_throughput() {
        use std::time::Instant;

        // Firehose-shaped entries: short msg, ~10 fields including a fat
        // ~1KB payload string, mirroring the kubernetes load test.
        let msgs = [
            "request completed",
            "validation failed",
            "retrying downstream dependency",
            "cache lookup completed",
            "database query completed",
            "downstream call completed",
        ];
        // Alphabet-soup blob like the firehose's random payload: it passes
        // char-frequency prefilters, forcing real match work when uncapped.
        let fat: String = "abcdefghijklmnopqrstuvwxyz0123456789"
            .chars()
            .cycle()
            .take(1024)
            .collect();
        let entries: Vec<Arc<LogEntry>> = (0..10_000u64)
            .map(|seq| {
                let mut fields = HashMap::new();
                fields.insert("trace_id".into(), serde_json::json!("49c7562f535215f2a43921220002d6d9"));
                fields.insert("span_id".into(), serde_json::json!("505bd20dbaa9f06d"));
                fields.insert("request_id".into(), serde_json::json!(format!("req-json-firehose-hot-{seq}")));
                fields.insert("tenant_id".into(), serde_json::json!(seq % 5000));
                fields.insert("user_id".into(), serde_json::json!(seq * 7));
                fields.insert("http".into(), serde_json::json!({"method": "POST", "path": "/internal/events", "status": 203, "duration_ms": 2018, "bytes_in": 10673, "bytes_out": 213177}));
                fields.insert("kubernetes".into(), serde_json::json!({"namespace": "default", "pod": "json-firehose-hot-787f564958-xzf2p", "container": "json-firehose"}));
                fields.insert("labels".into(), serde_json::json!({"app": "firehose", "version": "v3.13.13", "shard": "17"}));
                fields.insert("payload".into(), serde_json::json!({"cart_id": "cart-60572823", "random": fat}));
                Arc::new(LogEntry {
                    seq,
                    msg: msgs[seq as usize % msgs.len()].to_string(),
                    ts: Utc::now(),
                    level: Some(LogLevel::Info),
                    source: Source {
                        producer: "kubernetes".into(),
                        id: "default/json-firehose-hot/json-firehose".into(),
                        display_name: "json-firehose-hot-787f564958-xzf2p/json-firehose".into(),
                        group: Some("default".into()),
                    },
                    fields,
                })
            })
            .collect();
        let needle = "retrying downstream";
        let n = entries.len() as f64;

        let time = |label: &str, f: &mut dyn FnMut()| {
            let start = Instant::now();
            f();
            let secs = start.elapsed().as_secs_f64();
            println!("{label:<46} {:>10.0} entries/s", n / secs);
        };

        // 1. Production path: all fields, indices, per-scan stringify.
        let matcher = FrizbeeMatcher::new(None, 512);
        time(
            "production scan pass (scores-only, 512B leaf cap)",
            &mut || {
                let mut total = 0usize;
                for chunk in entries.chunks(SCAN_CHUNK_SIZE) {
                    total += matcher.score_batch(needle, chunk).len();
                }
                assert!(total > 0);
            },
        );

        // 1b. Nucleo (the default matcher — no SIMD prefilter), capped
        // and uncapped, to show what the leaf cap buys it.
        for (label, cap) in [
            ("nucleo scan pass (512B leaf cap)", 512usize),
            ("nucleo scan pass (uncapped)", 0usize),
        ] {
            let mut matcher = NucleoMatcher::new(needle, cap);
            time(label, &mut || {
                let mut total = 0usize;
                for chunk in entries.chunks(SCAN_CHUNK_SIZE) {
                    total += matcher.score_batch(chunk).len();
                }
                assert!(total > 0);
            });
        }

        // 2. Same haystacks, scores only (no index traceback).
        let config = FrizbeeConfig {
            max_typos: None,
            sort: false,
            scoring: frizbee::Scoring::default(),
        };
        let mut field_strings: Vec<Cow<'_, str>> = Vec::new();
        for entry in &entries {
            for (key, value) in &entry.fields {
                for_each_leaf(key, value, 0, &mut |_, text| field_strings.push(text));
            }
        }
        time("all-field haystacks, match_list (scores only)", &mut || {
            let msg_haystack: Vec<&str> = entries.iter().map(|e| e.msg.as_str()).collect();
            let hits = frizbee::match_list(needle, &msg_haystack, &config);
            let field_hits = frizbee::match_list(needle, &field_strings, &config);
            assert!(hits.len() + field_hits.len() > 0);
        });

        // 3. Leaf-walk haystack collection alone (per-scan prep cost).
        time("leaf-walk collection over all fields", &mut || {
            let mut leaves: Vec<Cow<'_, str>> = Vec::new();
            for entry in &entries {
                for (key, value) in &entry.fields {
                    for_each_leaf(key, value, 0, &mut |_, text| leaves.push(text));
                }
            }
            assert!(!leaves.is_empty());
        });

        // 4. msg-only, indices (what a haystack-capped scan would pay).
        time("msg-only match_list_indices", &mut || {
            let msg_haystack: Vec<&str> = entries.iter().map(|e| e.msg.as_str()).collect();
            let hits = match_list_indices(needle, &msg_haystack, &config);
            assert!(!hits.is_empty());
        });

        // 5. msg-only, scores only — the matcher's ceiling on short lines.
        time("msg-only match_list (scores only)", &mut || {
            let msg_haystack: Vec<&str> = entries.iter().map(|e| e.msg.as_str()).collect();
            let hits = frizbee::match_list(needle, &msg_haystack, &config);
            assert!(!hits.is_empty());
        });
    }
}
