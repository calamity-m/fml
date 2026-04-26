//! Fuzzy search worker.
//!
//! Runs a frizbee-backed fuzzy match of the user's needle against every
//! retained [`LogEntry`]. For each entry the worker scores three field
//! classes independently — `msg`, the `level` display name (`"INFO"`,
//! `"WARN"`, …), and each entry's `fields` values — and folds the per-class
//! scores into a single weighted aggregate. `msg` dominates ([`WEIGHT_MSG`]),
//! `level` is next ([`WEIGHT_LEVEL`]), and `fields` are lightest
//! ([`WEIGHT_FIELDS`]) so an entry that hits on multiple weak fields can
//! still be outranked by a single strong `msg` hit.
//!
//! Lifecycle: a `tokio::time::interval` ticks at `tick_rate`. The worker
//! holds an in-flight [`ScanState`] across ticks, racing chunk processing
//! against the ticker in a `tokio::select!`: between ticks it scores
//! `SCAN_CHUNK_SIZE` entries at a time, and when a tick fires it emits
//! whatever has been scored so far with `complete = false`. The snapshot
//! is only retired when the scan finishes (final emission with
//! `complete = true`, bounds recorded so we don't re-scan an unchanged
//! window) or when the store's retained bounds drift before a fresh
//! scan starts. So `tick_rate` doubles as both the emission cadence and
//! the per-tick processing budget — there is no separate scan-budget
//! knob.
//! Cancellation of superseded queries is handled by the caller via
//! [`tokio::task::JoinHandle::abort`] — every loop iteration awaits at the
//! ticker or inside the emission helper, so abort is prompt.
//!
//! The emission contract is deliberately "emit everything we have": the
//! worker keeps up to `result_limit` hits (default 20k) and emits all of
//! them each cycle. Partial results are explicitly marked incomplete so
//! UI code can distinguish "best so far" from "full snapshot ranked".
//!
//! Per-hit highlight data is carried in [`Match::indices`] as ascending
//! character offsets into the matched field's value. Frizbee returns
//! these in reverse order; see [`reverse_to_u32`].

use std::{sync::Arc, time::Duration};

use frizbee::{Config as FrizbeeConfig, match_list_indices};
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::debug;

use crate::{
    event::{Match, SearchEvent, SearchHit},
    log::{LogEntry, SourceId},
    search::{EmitOutcome, emit_error, emit_hits},
    store::LogStore,
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
/// Weight applied to a frizbee score on each `fields` value.
const WEIGHT_FIELDS: f32 = 0.3;

/// Starts the background worker for a fuzzy text search.
///
/// The worker matches `term` against each retained `LogEntry`'s `msg`,
/// `level` display name, and `fields` values using frizbee, weights
/// those matches (msg > level > fields), and emits ranked hits at
/// `tick_rate` cadence. A snapshot of the retained `(low, high)` window
/// and its [`ScanState`] are held across ticks: between ticks the
/// `tokio::select!` advances the scan one `SCAN_CHUNK_SIZE` batch at a
/// time, and when the ticker wins it emits the best `result_limit`
/// hits scored so far with `complete = false`. The final emission for a
/// snapshot carries `complete = true`. A snapshot is retired when the
/// scan completes or when bounds drift before the next scan begins.
/// Final ranking is by aggregate score, then `seq desc`. The returned
/// [`JoinHandle`] is used to cancel superseded work.
pub fn start_fuzzy_search(
    term: String,
    sources: Vec<SourceId>,
    result_limit: usize,
    tick_rate: Duration,
    max_typos: Option<u16>,
    store: Arc<dyn LogStore>,
    request_id: u64,
    tx: mpsc::Sender<SearchEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "spawned fuzzy search - term: {}, sources: {:?}, result_limit: {}, tick_rate: {:?}",
            term, sources, result_limit, tick_rate
        );

        // An empty needle would match every entry with score 0 and produce
        // no useful ranking. Emit one empty result so the dispatcher sees
        // this request_id respond at least once, then exit — no point
        // holding a ticker open for something that can never produce hits.
        if term.is_empty() {
            let _ = emit_hits(Vec::new(), request_id, true, &tx).await;
            return;
        }

        // `sort: false` — we combine scores across three frizbee calls
        // (msg/level/fields) into a single aggregate and need to sort on
        // that combined number ourselves. Letting frizbee sort would waste
        // work and give us per-call orderings we'd immediately discard.
        let frizbee_config = FrizbeeConfig {
            max_typos,
            sort: false,
            scoring: frizbee::Scoring::default(),
        };

        let mut ticker = tokio::time::interval(tick_rate);
        let mut last_complete_bounds: Option<(u64, u64)> = None;
        // Active snapshot kept alive across ticks so partial emissions
        // resume instead of throwing away work. (low, high, scan).
        let mut scan: Option<(u64, u64, ScanState)> = None;

        loop {
            // No live scan: park on the ticker until something might
            // have changed, then decide whether to start one.
            if scan.is_none() {
                ticker.tick().await;

                let (low, high) = store.bounds();

                if high == 0 {
                    if last_complete_bounds != Some((low, high)) {
                        match emit_hits(Vec::new(), request_id, true, &tx).await {
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

                let mut pool: Vec<Arc<LogEntry>> = Vec::new();
                if let Err(e) = store.fetch_range(low, high, &mut pool) {
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
                    let hits = build_hits(&state.scored, result_limit);
                    match emit_hits(hits, request_id, false, &tx).await {
                        EmitOutcome::Sent => {}
                        EmitOutcome::ReceiverGone => return,
                    }
                }
                _ = tokio::task::yield_now(), if !state.is_complete() => {
                    state.process_next_chunk(&term, &frizbee_config);
                    if state.is_complete() {
                        let hits = build_hits(&state.scored, result_limit);
                        match emit_hits(hits, request_id, true, &tx).await {
                            EmitOutcome::Sent => {}
                            EmitOutcome::ReceiverGone => return,
                        }
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
    scored: Vec<ScoredHit>,
}

impl ScanState {
    fn new(entries: Vec<Arc<LogEntry>>) -> Self {
        Self {
            entries,
            next_index: 0,
            scored: Vec::new(),
        }
    }

    fn is_complete(&self) -> bool {
        self.next_index >= self.entries.len()
    }

    fn process_next_chunk(&mut self, needle: &str, config: &FrizbeeConfig) {
        if self.is_complete() {
            return;
        }

        let end = (self.next_index + SCAN_CHUNK_SIZE).min(self.entries.len());
        self.scored.extend(score_batch(
            needle,
            &self.entries[self.next_index..end],
            config,
        ));
        self.next_index = end;
    }
}

/// One ranked entry in the worker's internal cache.
///
/// `score` is the weighted aggregate across all matched field classes
/// (`i64` to absorb summed u16 scores under weights without overflow).
/// `matches` carries one [`Match`] per field that contributed — the
/// downstream `SearchHit` clones these out so the UI can highlight
/// matched characters per-field.
struct ScoredHit {
    entry: Arc<LogEntry>,
    score: i64,
    matches: Vec<Match>,
}

fn build_hits(scored: &[ScoredHit], result_limit: usize) -> Vec<SearchHit> {
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
            matches: clone_matches(&hit.matches),
        })
        .collect()
}

/// Deep-clones a per-hit `Match` list.
///
/// `Match` doesn't derive `Clone` (see `fml/src/event.rs`), so we
/// reconstruct manually when copying from the cache into each emitted
/// `SearchHit`.
fn clone_matches(matches: &[Match]) -> Vec<Match> {
    matches
        .iter()
        .map(|m| Match {
            key: m.key.clone(),
            indices: m.indices.clone(),
        })
        .collect()
}

/// Scores one batch of new entries against `needle`.
///
/// Batches three frizbee calls — one per field class — rather than
/// looping per-entry. Frizbee is designed to amortise its SIMD-wide
/// inner loop across many haystack strings at once; a per-entry call
/// with a 1–3 element haystack would leave that parallelism on the
/// floor.
///
/// Returns only entries that scored on at least one field. Zero-score
/// entries would waste cache capacity with no useful ranking signal.
fn score_batch(
    needle: &str,
    entries: &[Arc<LogEntry>],
    config: &FrizbeeConfig,
) -> Vec<ScoredHit> {
    if entries.is_empty() {
        return Vec::new();
    }

    // Per-entry haystacks: the frizbee result's `index` maps 1:1 back
    // into `entries` for these two classes.
    let msg_haystack: Vec<&str> = entries.iter().map(|e| e.msg.as_str()).collect();
    let level_haystack: Vec<String> = entries
        .iter()
        .map(|e| e.level.map(|l| l.to_string()).unwrap_or_default())
        .collect();

    // Fields are a variable number per entry, so the haystack is
    // flattened across all entries and a parallel `field_owners`
    // side-table records which entry and field name each haystack
    // position came from.
    let mut field_haystack: Vec<String> = Vec::new();
    let mut field_owners: Vec<(usize, String)> = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        for (key, value) in &entry.fields {
            field_haystack.push(stringify_value(value));
            field_owners.push((idx, key.clone()));
        }
    }

    let msg_hits = match_list_indices(needle, &msg_haystack, config);
    // Skip the frizbee call entirely when every entry's level is None;
    // otherwise we'd pay to match the needle against a slice of empty
    // strings for no benefit.
    let level_hits = if level_haystack.iter().any(|s| !s.is_empty()) {
        match_list_indices(needle, &level_haystack, config)
    } else {
        Vec::new()
    };
    let field_hits = if !field_haystack.is_empty() {
        match_list_indices(needle, &field_haystack, config)
    } else {
        Vec::new()
    };

    // Pre-allocate one slot per entry so `apply_match` can O(1) index
    // into the right accumulator regardless of which field class hit.
    // `Option` wrapping is just so `apply_match` can borrow `&mut self`
    // without owning/replacing.
    let mut scored: Vec<Option<ScoredHit>> = entries
        .iter()
        .map(|e| {
            Some(ScoredHit {
                entry: e.clone(),
                score: 0,
                matches: Vec::new(),
            })
        })
        .collect();

    for m in msg_hits {
        apply_match(&mut scored, m.index as usize, "msg", m.score, m.indices, WEIGHT_MSG);
    }
    for m in level_hits {
        apply_match(&mut scored, m.index as usize, "level", m.score, m.indices, WEIGHT_LEVEL);
    }
    for m in field_hits {
        let owner_idx = m.index as usize;
        if owner_idx >= field_owners.len() {
            continue;
        }
        let (entry_idx, key) = &field_owners[owner_idx];
        apply_match(&mut scored, *entry_idx, key, m.score, m.indices, WEIGHT_FIELDS);
    }

    // Frizbee can return a match for a haystack whose score computes to
    // zero under our weights (e.g. an empty level string could in theory
    // slip through). Filter to genuinely useful hits only.
    scored
        .into_iter()
        .flatten()
        .filter(|hit| hit.score > 0 && !hit.matches.is_empty())
        .collect()
}

/// Folds a single frizbee match into the accumulator slot for its entry.
///
/// The score is weighted by field class and added to the running
/// aggregate; the raw character indices are reversed (see
/// [`reverse_to_u32`]) and appended as a per-field [`Match`]. Silently
/// drops out-of-range indices rather than panicking — upstream code
/// already bounds-checks `field_owners`, so this is a belt-and-braces
/// guard against future changes.
fn apply_match(
    scored: &mut [Option<ScoredHit>],
    entry_idx: usize,
    key: &str,
    raw_score: u16,
    indices: Vec<usize>,
    weight: f32,
) {
    let Some(slot) = scored.get_mut(entry_idx).and_then(Option::as_mut) else {
        return;
    };
    slot.score = slot
        .score
        .saturating_add((raw_score as f32 * weight).round() as i64);
    slot.matches.push(Match {
        key: key.to_string(),
        indices: reverse_to_u32(indices),
    });
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

/// Flattens a `serde_json::Value` into a plain string for frizbee.
///
/// Frizbee matches on byte sequences, so any JSON structure needs a
/// concrete representation. For scalars we use the natural textual
/// form; for composite types we fall back to the JSON encoding so
/// users can still fuzzy-find keys or substrings inside arrays/objects
/// (e.g. typing `user_id` finds it inside a JSON blob field).
fn stringify_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        log::{LogLevel, NewLogEntry, Source},
        store::RingBufferStore,
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
                results,
                request_id,
                complete,
            } => (results, request_id, complete),
            SearchEvent::Error(e) => panic!("unexpected SearchEvent::Error({e})"),
            SearchEvent::Search { .. } => panic!("unexpected SearchEvent::Search"),
        }
    }

    #[tokio::test]
    async fn empty_term_emits_empty_result() {
        let store = RingBufferStore::new(store_config(64));

        let (tx, mut rx) = mpsc::channel(8);
        let handle = start_fuzzy_search(
            String::new(),
            vec![],
            100,
            Duration::from_millis(10),
            None,
            store.clone(),
            99,
            tx,
        );

        let (results, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 99);
        assert!(complete);
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            100,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec!["s1".to_string()],
            100,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            100,
            Duration::from_millis(10),
            None,
            store.clone(),
            3,
            tx,
        );

        let (seed, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 3);
        assert!(complete);
        assert_eq!(
            seed.iter().map(|r| r.seq_id).collect::<Vec<_>>(),
            vec![1]
        );

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
        let handle = start_fuzzy_search(
            "er".to_string(),
            vec![],
            100,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "WARN".to_string(),
            vec![],
            100,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            5,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            10,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            2,
            Duration::from_millis(10),
            None,
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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            20,
            Duration::from_millis(1),
            None,
            store.clone(),
            13,
            tx,
        );

        let (_, rid, complete) = recv_result(&mut rx).await;
        assert_eq!(rid, 13);
        assert!(!complete, "expected an incomplete partial result");

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
        let handle = start_fuzzy_search(
            "error".to_string(),
            vec![],
            20_000,
            Duration::from_millis(1),
            None,
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
}
