use std::{collections::HashSet, sync::Arc};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::{
    error::FmlError,
    event::FieldPredicate,
    log::{LogEntry, SourceId},
    search::{EmitOutcome, SearchContext, emit_error, emit_results},
    store::LogStore,
};

const CHUNK_SIZE: u64 = 4096;

/// Starts a frozen search collecting entries within a timestamp window around an anchor.
pub fn start_time_window_search(
    ctx: SearchContext,
    anchor_ts: DateTime<Utc>,
    window_secs: u64,
    until_seq: u64,
    predicates: Vec<FieldPredicate>,
) -> JoinHandle<()> {
    let SearchContext {
        target,
        query,
        sources,
        request_id,
        tick_rate: _,
        store,
        tx,
    } = ctx;

    tokio::spawn(async move {
        debug!(
            "spawned time-window search - anchor_ts: {}, window_secs: {}, until_seq: {}, predicates: {:?}",
            anchor_ts, window_secs, until_seq, predicates
        );

        let entries = match collect_time_window(
            &store,
            anchor_ts,
            window_secs,
            until_seq,
            &sources,
            &predicates,
        )
        .await
        {
            Ok(entries) => entries,
            Err(e) => {
                let _ = emit_error(e.to_string(), &tx).await;
                return;
            }
        };

        match emit_results(target, query, entries, request_id, true, &tx).await {
            EmitOutcome::Sent | EmitOutcome::ReceiverGone => {}
        }
    })
}

pub(crate) async fn collect_time_window(
    store: &Arc<dyn LogStore>,
    anchor_ts: DateTime<Utc>,
    window_secs: u64,
    until_seq: u64,
    sources: &[SourceId],
    predicates: &[FieldPredicate],
) -> Result<Vec<Arc<LogEntry>>, FmlError> {
    let delta =
        TimeDelta::try_seconds(window_secs.min(i64::MAX as u64) as i64).unwrap_or(TimeDelta::MAX);
    let lo_ts = anchor_ts
        .checked_sub_signed(delta)
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let hi_ts = anchor_ts
        .checked_add_signed(delta)
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    let (store_low, store_high) = store.bounds();
    let scan_high = until_seq.min(store_high);
    if store_low == 0 || scan_high < store_low {
        return Ok(Vec::new());
    }

    let source_set: HashSet<&SourceId> = sources.iter().collect();
    let mut collected = Vec::new();
    let mut lower = store_low;
    while lower <= scan_high {
        let upper = lower.saturating_add(CHUNK_SIZE - 1).min(scan_high);
        let mut buf = Vec::new();
        store.fetch_range(lower, upper, &mut buf)?;
        for entry in buf {
            if entry.ts >= lo_ts
                && entry.ts <= hi_ts
                && (source_set.is_empty() || source_set.contains(&entry.source.id))
                && matches_predicates(&entry, predicates)
            {
                collected.push(entry);
            }
        }
        tokio::task::yield_now().await;
        if upper == scan_high {
            break;
        }
        lower = upper + 1;
    }

    collected.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.seq.cmp(&b.seq)));
    Ok(collected)
}

/// True when every predicate's key maps to an exactly equal JSON value.
pub(crate) fn matches_predicates(entry: &LogEntry, predicates: &[FieldPredicate]) -> bool {
    predicates
        .iter()
        .all(|predicate| entry.fields.get(&predicate.key) == Some(&predicate.value))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::{
        config::store::StoreConfig,
        log::{LogLevel, NewLogEntry, Source, TsSource},
        store::RingBufferStore,
    };

    fn entry(ts_secs: i64, source_id: &str, trace_id: &str) -> NewLogEntry {
        NewLogEntry {
            msg: format!("entry {ts_secs}"),
            ts: Utc
                .timestamp_opt(ts_secs, 0)
                .single()
                .expect("valid timestamp"),
            ts_source: TsSource::Parsed,
            raw: None,
            level: Some(LogLevel::Info),
            source: Source {
                producer: "fake".to_string(),
                id: source_id.to_string(),
                display_name: source_id.to_string(),
                group: None,
            },
            fields: HashMap::from([("trace_id".to_string(), json!(trace_id))]),
        }
    }

    fn seqs(entries: Vec<Arc<LogEntry>>) -> Vec<u64> {
        entries.into_iter().map(|entry| entry.seq).collect()
    }

    #[tokio::test]
    async fn sorts_timestamp_window_independently_of_arrival_order() {
        let store = RingBufferStore::new(StoreConfig { capacity: 8 });
        store.insert(entry(110, "a", "t1"));
        store.insert(entry(90, "b", "t1"));
        store.insert(entry(100, "a", "t1"));
        store.insert(entry(130, "a", "t1"));

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            10,
            4,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert_eq!(seqs(entries), vec![2, 3, 1]);
    }

    #[tokio::test]
    async fn applies_snapshot_source_and_field_bounds() {
        let store = RingBufferStore::new(StoreConfig { capacity: 8 });
        store.insert(entry(99, "a", "t1"));
        store.insert(entry(100, "b", "t1"));
        store.insert(entry(101, "a", "other"));
        store.insert(entry(102, "a", "t1"));
        let predicates = [FieldPredicate {
            key: "trace_id".to_string(),
            value: json!("t1"),
        }];

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            u64::MAX,
            3,
            &["a".to_string()],
            &predicates,
        )
        .await
        .expect("filtered time-window entries");

        assert_eq!(seqs(entries), vec![1]);
    }

    #[test]
    fn predicate_equality_is_exact_without_type_coercion() {
        let mut string_entry = entry(100, "a", "t1");
        string_entry.fields = HashMap::from([("status".to_string(), json!("500"))]);
        let mut number_entry = entry(100, "a", "t1");
        number_entry.fields = HashMap::from([("status".to_string(), json!(500))]);
        let store = RingBufferStore::new(StoreConfig { capacity: 4 });
        store.insert(string_entry);
        let number_seq = store.insert(number_entry);
        let mut out = Vec::new();
        store.fetch_requested(&[1, number_seq], &mut out).unwrap();
        let predicates = [FieldPredicate {
            key: "status".to_string(),
            value: json!(500),
        }];

        assert!(!matches_predicates(&out[0], &predicates));
        assert!(matches_predicates(&out[1], &predicates));
    }

    #[test]
    fn multiple_predicates_use_and_semantics() {
        let mut partial = entry(100, "a", "t1");
        partial.fields = HashMap::from([("request_id".to_string(), json!("abc"))]);
        let mut full = entry(100, "a", "t1");
        full.fields = HashMap::from([
            ("request_id".to_string(), json!("abc")),
            ("status".to_string(), json!(500)),
        ]);
        let store = RingBufferStore::new(StoreConfig { capacity: 4 });
        store.insert(partial);
        store.insert(full);
        let mut out = Vec::new();
        store.fetch_requested(&[1, 2], &mut out).unwrap();
        let predicates = [
            FieldPredicate {
                key: "request_id".to_string(),
                value: json!("abc"),
            },
            FieldPredicate {
                key: "status".to_string(),
                value: json!(500),
            },
        ];

        assert!(!matches_predicates(&out[0], &predicates));
        assert!(matches_predicates(&out[1], &predicates));
    }

    #[tokio::test]
    async fn excludes_entries_past_until_seq() {
        let store = RingBufferStore::new(StoreConfig { capacity: 8 });
        store.insert(entry(100, "a", "t1"));
        store.insert(entry(101, "a", "t1"));
        store.insert(entry(102, "a", "t1"));

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            10,
            2,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert_eq!(seqs(entries), vec![1, 2]);
    }

    #[tokio::test]
    async fn returns_empty_when_store_is_empty() {
        let store = RingBufferStore::new(StoreConfig { capacity: 8 });

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            10,
            u64::MAX,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn returns_empty_when_until_seq_precedes_retained_entries() {
        let store = RingBufferStore::new(StoreConfig { capacity: 2 });
        for ts in 100..105 {
            store.insert(entry(ts, "a", "t1"));
        }
        // Seqs 1..=3 are evicted; a snapshot bound below the oldest retained
        // entry must yield nothing rather than scanning out of bounds.
        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(102, 0).single().unwrap(),
            10,
            3,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn scans_only_retained_entries_after_eviction() {
        let store = RingBufferStore::new(StoreConfig { capacity: 2 });
        for ts in 100..105 {
            store.insert(entry(ts, "a", "t1"));
        }

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            10,
            u64::MAX,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert_eq!(seqs(entries), vec![4, 5]);
    }

    #[tokio::test]
    async fn collects_across_chunk_boundaries() {
        let total = CHUNK_SIZE as usize + 10;
        let store = RingBufferStore::new(StoreConfig { capacity: total });
        for _ in 0..total {
            store.insert(entry(100, "a", "t1"));
        }

        let entries = collect_time_window(
            &store,
            Utc.timestamp_opt(100, 0).single().unwrap(),
            10,
            u64::MAX,
            &[],
            &[],
        )
        .await
        .expect("time-window entries");

        assert_eq!(entries.len(), total);
        // Equal timestamps fall back to seq order, so the chunk seam must
        // not reorder or drop entries.
        assert_eq!(seqs(entries), (1..=total as u64).collect::<Vec<_>>());
    }
}
