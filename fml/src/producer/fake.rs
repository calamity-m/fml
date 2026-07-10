use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use fake::{
    Fake,
    faker::{lorem::en::Sentence, name::en::Name},
};
use rand::{Rng, seq::IndexedRandom};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
    event::ProducerEvent,
    log::{LogLevel, NewLogEntry, Source, TsSource},
    producer::LogProducer,
};

const DEFAULT_TICK: Duration = Duration::from_millis(500);

/// A synthetic [`LogProducer`] used by the `--demo` flag.
///
/// Emits a single [`ProducerEvent::SourceFound`] for its [`Source`] then
/// ticks out randomised [`NewLogEntry`] values until [`stop`](Self::stop)
/// is called.
pub struct FakeProducer {
    source: Source,
    tick: Duration,
    cancel: Arc<AtomicBool>,
}

impl FakeProducer {
    pub fn new(source: Source) -> Self {
        Self::with_tick(source, DEFAULT_TICK)
    }

    pub fn with_tick(source: Source, tick: Duration) -> Self {
        Self {
            source,
            tick,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl LogProducer for FakeProducer {
    fn start(&self, tx: mpsc::Sender<ProducerEvent>) {
        let source = self.source.clone();
        let cancel = self.cancel.clone();
        let tick = self.tick;

        tokio::spawn(async move {
            if tx
                .send(ProducerEvent::SourceFound(source.clone()))
                .await
                .is_err()
            {
                debug!(
                    "fake producer {} aborting: event channel closed before first tick",
                    source.id
                );
                return;
            }

            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(tick).await;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let entry = synthetic_entry(&source);
                if tx.send(ProducerEvent::StoreEvent(entry)).await.is_err() {
                    debug!("fake producer {} aborting: event channel closed", source.id);
                    break;
                }
            }
        });
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn synthetic_entry(source: &Source) -> NewLogEntry {
    const LEVELS: &[LogLevel] = &[
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Fatal,
    ];

    let mut rng = rand::rng();

    let level = LEVELS.choose(&mut rng).copied();
    let msg: String = Sentence(4..12).fake();
    let host: String = Name().fake();

    let mut fields: HashMap<String, serde_json::Value> = HashMap::new();
    fields.insert("host".to_string(), serde_json::Value::String(host));
    fields.insert(
        "latency_ms".to_string(),
        serde_json::Value::from(rng.random_range(1u32..1000)),
    );
    if rng.random_bool(0.3) {
        const REQUEST_IDS: &[&str] = &[
            "req-demo-1",
            "req-demo-2",
            "req-demo-3",
            "req-demo-4",
            "req-demo-5",
        ];
        fields.insert(
            "request_id".to_string(),
            serde_json::Value::from(*REQUEST_IDS.choose(&mut rng).unwrap_or(&"req-demo-1")),
        );
    }

    NewLogEntry {
        msg,
        ts: Utc::now(),
        ts_source: TsSource::Ingest,
        raw: None,
        level,
        source: source.clone(),
        fields,
    }
}
